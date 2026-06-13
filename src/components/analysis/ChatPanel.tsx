import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, MessageSquare, Trash2, GitCompare, Clock, FileJson, FileCode } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ChatInputBar } from "@/components/analysis/ChatInputBar";
import { ChatMessageList, type StreamDraft } from "@/components/analysis/ChatMessageList";
import { api, CHAT_CANCELLED_MESSAGE } from "@/lib/api";
import { useSettingsStore, useAppStore } from "@/store/app-store";
import { cn, formatBytes } from "@/lib/utils";
import type {
  AnalysisSession,
  ChatContext,
  ChatMessage,
  ChatSendResult,
  ChatStreamEvent,
  ChatToolEvent,
  HarEntryDetail,
} from "@/lib/types";

const SCROLL_STICK_THRESHOLD = 80;

export interface ToolActivityItem {
  id: string;
  step: number;
  tool: string;
  status: string;
  detail: string;
  reasoning?: string;
  order: number;
}

function sortToolActivity(items: ToolActivityItem[]): ToolActivityItem[] {
  const planning = items.filter((item) => item.id === "agent-planning");
  const rest = items
    .filter((item) => item.id !== "agent-planning")
    .sort((a, b) => a.order - b.order || a.step - b.step || a.tool.localeCompare(b.tool));
  return [...rest, ...planning];
}

interface AgentContinuePrompt {
  kind: "step" | "tool";
  stepsUsed: number;
  stepLimit: number;
  toolsExecuted?: number;
  toolRunLimit?: number;
  nextToolRunLimit?: number;
}

interface ChatPanelProps {
  sessionId: string | null;
  entryContext: HarEntryDetail | null;
  onClearEntryContext: () => void;
}

export function ChatPanel({
  sessionId,
  entryContext,
  onClearEntryContext,
}: ChatPanelProps) {
  const thinkingModel = useSettingsStore((s) => s.settings.thinking_model);
  const chatAgentMaxSteps = useSettingsStore((s) => s.settings.chat_agent_max_steps ?? 10);
  const sessions = useAppStore((s) => s.sessions);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [hasMoreMessages, setHasMoreMessages] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [sending, setSending] = useState(false);
  const [thinkingMode, setThinkingMode] = useState(false);
  const [streamDraft, setStreamDraft] = useState<StreamDraft | null>(null);
  const [toolActivity, setToolActivity] = useState<ToolActivityItem[]>([]);
  const [agentContinuePrompt, setAgentContinuePrompt] = useState<AgentContinuePrompt | null>(
    null
  );
  const [agentPaused, setAgentPaused] = useState(false);
  const [stopDialogOpen, setStopDialogOpen] = useState(false);
  const [compareDialogOpen, setCompareDialogOpen] = useState(false);
  const [showJumpToBottom, setShowJumpToBottom] = useState(false);
  const [contextBudget, setContextBudget] = useState<{
    contextTokens: number;
    percentUsed: number;
    hardMaxChars: number;
    usedChars: number;
  } | null>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const stickToBottomRef = useRef(true);
  const streamDraftRef = useRef<StreamDraft>({ content: "", reasoning: "" });
  const streamRafRef = useRef<number | null>(null);
  const continueResolverRef = useRef<((value: boolean) => void) | null>(null);
  const toolActivityOrderRef = useRef(0);
  const toolActivityFallbackIdRef = useRef(0);

  const toolActivityBufferRef = useRef<ToolActivityItem[]>([]);
  const toolActivityRafRef = useRef<number | null>(null);

  const upsertToolActivity = useCallback((item: Omit<ToolActivityItem, "order">) => {
    const previousPlanning = toolActivityBufferRef.current.find(
      (entry) => entry.id === "agent-planning"
    );
    const order =
      item.id === "agent-planning"
        ? (previousPlanning?.order ?? toolActivityOrderRef.current++)
        : toolActivityOrderRef.current++;

    const newItem = { ...item, order };
    toolActivityBufferRef.current = toolActivityBufferRef.current
      .filter((entry) => entry.id !== item.id)
      .concat(newItem);

    if (toolActivityRafRef.current === null) {
      toolActivityRafRef.current = requestAnimationFrame(() => {
        const batch = toolActivityBufferRef.current;
        toolActivityBufferRef.current = [];
        setToolActivity((prev) => {
          let next = [...prev];
          for (const bi of batch) {
            next = next.filter((entry) => entry.id !== bi.id);
            if (bi.tool !== "agent" && bi.status === "running") {
              next = next.filter((entry) => entry.id !== "agent-planning");
            }
            if (bi.tool === "agent" && bi.status === "done") {
              next = next.filter((entry) => entry.id !== "agent-planning");
            }
            next.push(bi);
          }
          return sortToolActivity(next);
        });
        toolActivityRafRef.current = null;
      });
    }
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    viewport.scrollTo({ top: viewport.scrollHeight, behavior });
  }, []);

  const updateStickState = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    const distanceFromBottom =
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    const atBottom = distanceFromBottom <= SCROLL_STICK_THRESHOLD;
    stickToBottomRef.current = atBottom;
    setShowJumpToBottom(!atBottom);
  }, []);

  const waitForContinueDecision = useCallback(
    (result: ChatSendResult) =>
      new Promise<boolean>((resolve) => {
        setAgentContinuePrompt({
          kind: result.limit_kind === "tool" ? "tool" : "step",
          stepsUsed: result.steps_used,
          stepLimit: result.step_limit,
          toolsExecuted: result.tools_executed,
          toolRunLimit: result.tool_run_limit,
          nextToolRunLimit: result.next_tool_run_limit,
        });
        continueResolverRef.current = resolve;
      }),
    []
  );

  const clearStreamUi = useCallback(() => {
    setStreamDraft(null);
    if (streamRafRef.current !== null) {
      cancelAnimationFrame(streamRafRef.current);
      streamRafRef.current = null;
    }
  }, []);

  const clearAgentUi = useCallback(() => {
    clearStreamUi();
    setToolActivity([]);
    setAgentContinuePrompt(null);
  }, [clearStreamUi]);

  const runAgentWithContinuations = useCallback(
    async (initialSend: () => Promise<ChatSendResult>) => {
      let result = await initialSend();

      while (result.needs_continue) {
        const shouldContinue = await waitForContinueDecision(result);
        setAgentContinuePrompt(null);

        if (!shouldContinue) {
          setAgentContinuePrompt(null);
          setStreamDraft({ content: "", reasoning: "" });
          if (sessionId) {
            result = await api.finalizeChatAgent(sessionId, thinkingMode);
          }
          break;
        }

        setStreamDraft(null);
        if (sessionId) {
          result = await api.continueChatAgent(sessionId, thinkingMode);
        } else {
          break;
        }
      }

      return result;
    },
    [sessionId, thinkingMode, waitForContinueDecision]
  );

  useEffect(() => {
    if (sessionId) {
      console.log(`[ChatPanel] Loading initial messages for session ${sessionId}`);
      api.getChatMessages(sessionId, 50, 0).then((msgs) => {
        console.log(`[ChatPanel] Loaded ${msgs.length} initial messages`);
        setMessages(msgs);
        setHasMoreMessages(msgs.length === 50);
      }).catch((err) => {
        console.error(`[ChatPanel] Failed to load messages:`, err);
      });
      api.getToolSteps(sessionId, 100).then((steps) => {
        console.log(`[ChatPanel] Loaded ${steps.length} tool steps`);
        const items: ToolActivityItem[] = steps.map((s, i) => ({
          id: s.event_id || `tool-${s.id}`,
          step: s.step,
          tool: s.tool,
          status: s.status,
          detail: s.detail,
          reasoning: s.reasoning || undefined,
          order: i,
        }));
        setToolActivity(items);
        toolActivityOrderRef.current = items.length;
      }).catch((err) => {
        console.error(`[ChatPanel] Failed to load tool steps:`, err);
      });
    } else {
      setMessages([]);
      setToolActivity([]);
      setHasMoreMessages(false);
    }
    setStreamDraft(null);
    setAgentContinuePrompt(null);
    setAgentPaused(false);
    setContextBudget(null);
    toolActivityOrderRef.current = 0;
    toolActivityFallbackIdRef.current = 0;
    toolActivityBufferRef.current = [];
    if (toolActivityRafRef.current !== null) {
      cancelAnimationFrame(toolActivityRafRef.current);
      toolActivityRafRef.current = null;
    }
    stickToBottomRef.current = true;
    setShowJumpToBottom(false);
  }, [sessionId]);

  useEffect(() => {
    if (!sessionId) return;

    let unlistenStream: (() => void) | undefined;
    let unlistenTool: (() => void) | undefined;
    let unlistenCancel: (() => void) | undefined;

    listen<ChatStreamEvent>("chat-stream", (event) => {
      if (event.payload.session_id !== sessionId) return;
      if (event.payload.done) {
        if (streamRafRef.current !== null) {
          cancelAnimationFrame(streamRafRef.current);
          streamRafRef.current = null;
        }
        clearStreamUi();
        if (event.payload.message_id != null) {
          stickToBottomRef.current = true;
          requestAnimationFrame(() => scrollToBottom("auto"));
        }
        return;
      }

      streamDraftRef.current = {
        content: event.payload.content,
        reasoning: event.payload.reasoning,
      };

      if (streamRafRef.current === null) {
        streamRafRef.current = requestAnimationFrame(() => {
          setStreamDraft({ ...streamDraftRef.current });
          streamRafRef.current = null;
        });
      }
    }).then((fn) => {
      unlistenStream = fn;
    });

    listen<ChatToolEvent>("chat-tool", (event) => {
      if (event.payload.session_id !== sessionId) return;
      const { id, step, tool, status, detail, reasoning = "" } = event.payload;

      if (id === "context-budget") {
        const tk = parseInt(detail.split("/")[0] || "0", 10);
        const chars = parseInt(detail.split("/")[1] || "0", 10);
        const pct = parseInt(reasoning || "0", 10);
        setContextBudget({
          contextTokens: tk,
          percentUsed: pct,
          hardMaxChars: 0,
          usedChars: chars,
        });
        return;
      }

      upsertToolActivity({
        id: id || `tool-${toolActivityFallbackIdRef.current++}`,
        step,
        tool,
        status,
        detail: tool === "reasoning" ? "" : detail,
        reasoning:
          tool === "reasoning"
            ? reasoning || detail
            : (tool === "run_script" || tool === "edit_script") && reasoning
              ? reasoning
              : undefined,
      });
    }).then((fn) => {
      unlistenTool = fn;
    });

    let unlistenPaused: (() => void) | undefined;

    listen<string>("chat-agent-paused", (event) => {
      if (event.payload !== sessionId) return;
      clearStreamUi();
      setAgentPaused(true);
      setSending(false);
    }).then((fn) => {
      unlistenPaused = fn;
    });

    listen<string>("chat-cancelled", (event) => {
      if (event.payload !== sessionId) return;
      clearAgentUi();
      setAgentPaused(false);
    }).then((fn) => {
      unlistenCancel = fn;
    });

    return () => {
      unlistenStream?.();
      unlistenTool?.();
      unlistenCancel?.();
      unlistenPaused?.();
      if (streamRafRef.current !== null) {
        cancelAnimationFrame(streamRafRef.current);
      }
    };
  }, [sessionId, upsertToolActivity, clearAgentUi, clearStreamUi, scrollToBottom]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    updateStickState();
    viewport.addEventListener("scroll", updateStickState, { passive: true });
    return () => viewport.removeEventListener("scroll", updateStickState);
  }, [sessionId, updateStickState]);

  useEffect(() => {
    if (!stickToBottomRef.current) return;
    scrollToBottom(streamDraft || toolActivity.length > 0 ? "auto" : "smooth");
  }, [messages, streamDraft, toolActivity, scrollToBottom]);

  const handleJumpToBottom = useCallback(() => {
    stickToBottomRef.current = true;
    setShowJumpToBottom(false);
    scrollToBottom("smooth");
  }, [scrollToBottom]);

  const thinkingModelConfigured = thinkingModel.trim().length > 0;

  const loadMoreMessages = useCallback(async () => {
    if (!sessionId || loadingMore || !hasMoreMessages) return;
    setLoadingMore(true);
    console.log(`[ChatPanel] Loading more messages (offset: ${messages.length})`);
    try {
      const older = await api.getChatMessages(sessionId, 50, messages.length);
      console.log(`[ChatPanel] Loaded ${older.length} older messages`);
      setMessages((prev) => [...older, ...prev]);
      setHasMoreMessages(older.length === 50);
    } catch (err) {
      console.error(`[ChatPanel] Failed to load more messages:`, err);
    } finally {
      setLoadingMore(false);
    }
  }, [sessionId, loadingMore, hasMoreMessages, messages.length]);

  const handleSend = useCallback(
    async (text: string) => {
      if (!sessionId || sending) return;

      stickToBottomRef.current = true;
      setShowJumpToBottom(false);

      setSending(true);
      setStreamDraft(null);
      setToolActivity([]);
      toolActivityOrderRef.current = 0;
      toolActivityFallbackIdRef.current = 0;

      const optimistic: ChatMessage = {
        id: Date.now(),
        session_id: sessionId,
        role: "user",
        content: text,
        created_at: new Date().toISOString(),
      };
      setMessages((m) => [...m, optimistic]);
      requestAnimationFrame(() => scrollToBottom("auto"));

      let pausedRun = false;
      try {
        const context: ChatContext | undefined = entryContext
          ? { context_type: "entry", entry_index: entryContext.summary.index }
          : undefined;

        const result = await runAgentWithContinuations(() =>
          api.sendChatMessage(sessionId, text, context, thinkingMode)
        );

        if (result.limit_kind === "paused") {
          pausedRun = true;
          setAgentPaused(true);
          clearStreamUi();
        } else {
          setAgentPaused(false);
          const recent = await api.getChatMessages(sessionId, 50, 0);
          setMessages(recent);
          setHasMoreMessages(recent.length === 50);
        }
      } catch (err) {
        const message = String(err);
        if (message.includes(CHAT_CANCELLED_MESSAGE)) {
          const recent = await api.getChatMessages(sessionId, 50, 0);
          setMessages(recent);
          setHasMoreMessages(recent.length === 50);
        } else {
          console.error(err);
          alert(message);
          setMessages((m) => m.filter((x) => x.id !== optimistic.id));
        }
      } finally {
        continueResolverRef.current = null;
        setSending(false);
        if (!pausedRun) {
          clearAgentUi();
        } else {
          clearStreamUi();
          setAgentContinuePrompt(null);
        }
      }
    },
    [sessionId, sending, entryContext, thinkingMode, scrollToBottom, runAgentWithContinuations, clearAgentUi, clearStreamUi]
  );

  const handleStop = useCallback(() => {
    if (!sessionId || !sending) return;
    if (agentContinuePrompt) {
      continueResolverRef.current?.(false);
      continueResolverRef.current = null;
      return;
    }
    setStopDialogOpen(true);
  }, [sessionId, sending, agentContinuePrompt]);

  const handleStopKeep = useCallback(async () => {
    if (!sessionId) return;
    setStopDialogOpen(false);
    try {
      await api.cancelChatAgent(sessionId, "keep");
    } catch (err) {
      console.error(err);
    }
  }, [sessionId]);

  const handleStopFinalize = useCallback(async () => {
    if (!sessionId) return;
    setStopDialogOpen(false);
    try {
      await api.cancelChatAgent(sessionId, "finalize");
    } catch (err) {
      console.error(err);
    }
  }, [sessionId]);

  const handleContinueAgent = useCallback(() => {
    continueResolverRef.current?.(true);
    continueResolverRef.current = null;
  }, []);

  const handleCancelAgent = useCallback(() => {
    continueResolverRef.current?.(false);
    continueResolverRef.current = null;
  }, []);

  const handleCompare = useCallback(
    async (targetId: string) => {
      if (!sessionId || !targetId) return;
      setCompareDialogOpen(false);
      try {
        await api.sendChatMessage(
          sessionId,
          `Compare this HAR session with session ID "${targetId}". Use the compare_sessions tool.`,
          undefined,
          false
        );
        const all = await api.getChatMessages(sessionId);
        setMessages(all);
      } catch (err) {
        console.error(err);
        alert(String(err));
      }
    },
    [sessionId]
  );

  const handleLoadScript = useCallback(async () => {
    if (!sessionId) return;
    try {
      const result = await api.loadAgentScript(sessionId);
      alert(
        `Script loaded: ${result.file_name}\n${result.lines} lines · rev ${result.revision}`
      );
    } catch (err) {
      console.error(err);
      if (String(err) !== "No file selected") {
        alert(String(err));
      }
    }
  }, [sessionId]);

  const handleClearChat = useCallback(async () => {
    if (!sessionId || sending) return;
    if (
      messages.length > 0 &&
      !window.confirm("Clear all messages in this chat for the current session?")
    ) {
      return;
    }
    try {
      await api.cancelChatAgent(sessionId).catch(() => undefined);
      await api.clearChatMessages(sessionId);
      setMessages([]);
      setStreamDraft(null);
      setToolActivity([]);
      setAgentContinuePrompt(null);
    } catch (err) {
      console.error(err);
      alert(String(err));
    }
  }, [sessionId, sending, messages.length]);

  if (!sessionId) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        Select a session to chat about the analysis
      </p>
    );
  }

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="flex items-center justify-between gap-2 border-b border-primary/10 px-4 py-1.5">
        <div className="flex items-center gap-2">
          {contextBudget && (
            <ContextCircle
              percentUsed={contextBudget.percentUsed}
              contextK={contextBudget.contextTokens}
            />
          )}
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 text-xs text-muted-foreground"
            onClick={() => setCompareDialogOpen(true)}
            disabled={sending}
          >
            <GitCompare className="h-3.5 w-3.5" />
            Compare
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 text-xs text-muted-foreground"
            onClick={handleLoadScript}
            disabled={sending}
          >
            <FileCode className="h-3.5 w-3.5" />
            Load Script
          </Button>
        </div>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 gap-1.5 text-xs text-muted-foreground"
          onClick={handleClearChat}
          disabled={sending || messages.length === 0}
        >
          <Trash2 className="h-3.5 w-3.5" />
          Clear chat
        </Button>
      </div>

      {entryContext && (
        <div className="mx-4 mt-2 flex min-w-0 items-start gap-2 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs">
          <MessageSquare className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
          <span className="min-w-0 flex-1 break-all font-mono leading-snug">
            Context: {entryContext.summary.method} {entryContext.summary.url}
          </span>
          <Button
            size="sm"
            variant="ghost"
            className="ml-auto h-6 shrink-0 px-2 text-[10px]"
            onClick={onClearEntryContext}
          >
            Clear
          </Button>
        </div>
      )}

      <div className="relative min-h-0 min-w-0 flex-1">
        <ScrollArea className="h-full min-w-0 px-4" viewportRef={viewportRef}>
          <div className="min-w-0 max-w-full space-y-4 py-4">
            {hasMoreMessages && (
              <div className="flex justify-center pb-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={loadMoreMessages}
                  disabled={loadingMore}
                  className="text-xs"
                >
                  {loadingMore ? "Loading..." : "Load older messages"}
                </Button>
              </div>
            )}
            <ChatMessageList
              messages={messages}
              streamDraft={streamDraft}
              toolActivity={toolActivity}
              sending={sending}
              thinkingMode={thinkingMode}
              thinkingModelConfigured={thinkingModelConfigured}
              agentPaused={agentPaused}
            />
          </div>
        </ScrollArea>

        {agentPaused && (
          <div className="mx-4 mb-2 rounded-lg border border-primary/25 bg-primary/10 px-3 py-2 text-xs text-muted-foreground">
            Agent stopped — tool steps above are kept. Send a follow-up to continue from this
            point.
          </div>
        )}

        {showJumpToBottom && (
          <Button
            size="sm"
            variant="secondary"
            className="absolute bottom-3 left-1/2 z-10 h-8 -translate-x-1/2 gap-1.5 rounded-full border border-primary/25 bg-background/90 px-3 text-xs shadow-lg backdrop-blur-sm"
            onClick={handleJumpToBottom}
          >
            <ChevronDown className="h-3.5 w-3.5" />
            Jump to latest
          </Button>
        )}
      </div>

      {agentContinuePrompt && (
        <div className="mx-4 mb-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-xs">
          {agentContinuePrompt.kind === "tool" ? (
            <>
              <p className="font-medium text-amber-100/90">
                Tool call limit reached ({agentContinuePrompt.toolsExecuted ?? 0} /{" "}
                {agentContinuePrompt.toolRunLimit ?? 0})
              </p>
              <p className="mt-1 text-muted-foreground">
                This task is complex — continue with up to{" "}
                {agentContinuePrompt.nextToolRunLimit ?? 0} total tool calls in this reply, or
                finish now with a summary of what was found so far.
              </p>
            </>
          ) : (
            <>
              <p className="font-medium text-amber-100/90">
                Tool step limit reached ({agentContinuePrompt.stepsUsed} /{" "}
                {agentContinuePrompt.stepLimit})
              </p>
              <p className="mt-1 text-muted-foreground">
                Continue with {chatAgentMaxSteps} more LLM tool rounds, or finish now with a summary
                of what was found.
              </p>
            </>
          )}
          <div className="mt-2 flex gap-2">
            <Button size="sm" className="h-7 text-xs" onClick={handleContinueAgent}>
              Continue
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-xs"
              onClick={handleCancelAgent}
            >
              Finish answer
            </Button>
          </div>
        </div>
      )}

      <ChatInputBar
        sending={sending}
        thinkingMode={thinkingMode}
        thinkingModelConfigured={thinkingModelConfigured}
        thinkingModelLabel={thinkingModel}
        onThinkingModeChange={setThinkingMode}
        onSend={handleSend}
        onStop={handleStop}
      />

      {stopDialogOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
          role="presentation"
          onClick={() => setStopDialogOpen(false)}
        >
          <div
            className="w-full max-w-md rounded-xl border border-border bg-card p-5 shadow-2xl"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-semibold">Stop agent?</h3>
            <p className="mt-2 text-xs text-muted-foreground">
              Keep tool steps and reasoning for a follow-up, or ask for a partial answer now
              (planning was interrupted — the reply may be incomplete).
            </p>
            <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:justify-end">
              <Button variant="outline" size="sm" onClick={() => setStopDialogOpen(false)}>
                Cancel
              </Button>
              <Button variant="secondary" size="sm" onClick={handleStopKeep}>
                Keep progress
              </Button>
              <Button size="sm" onClick={handleStopFinalize}>
                Partial answer
              </Button>
            </div>
          </div>
        </div>
      )}
      {compareDialogOpen && (
        <SessionCompareDialog
          sessions={sessions}
          activeSessionId={activeSessionId}
          onSelect={handleCompare}
          onClose={() => setCompareDialogOpen(false)}
        />
      )}
    </div>
  );
}

function ContextCircle({
  percentUsed,
  contextK,
}: {
  percentUsed: number;
  contextK: number;
}) {
  const circumference = 2 * Math.PI * 8;
  const offset = circumference - (Math.min(percentUsed, 100) / 100) * circumference;
  const danger = percentUsed > 80;
  const warn = percentUsed > 60;

  return (
    <div className="group relative flex items-center gap-1.5" title={`${percentUsed}% of ${contextK}K context used`}>
      <svg width="20" height="20" className="shrink-0">
        <circle cx="10" cy="10" r="8" fill="none" stroke="currentColor" strokeWidth="1.5" className="text-border" />
        <circle
          cx="10"
          cy="10"
          r="8"
          fill="none"
          strokeWidth="2"
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          className={danger ? "text-red-400" : warn ? "text-amber-400" : "text-emerald-400"}
          style={{ transform: "rotate(-90deg)", transformOrigin: "10px 10px" }}
        />
      </svg>
      <span className="text-[10px] tabular-nums text-muted-foreground">{percentUsed}%</span>
      <div className="pointer-events-none absolute bottom-full left-0 mb-1 hidden rounded bg-popover px-2 py-1 text-[10px] whitespace-nowrap group-hover:block">
        ~{percentUsed}% of {contextK}K tokens used
      </div>
    </div>
  );
}

function SessionCompareDialog({
  sessions,
  activeSessionId,
  onSelect,
  onClose,
}: {
  sessions: AnalysisSession[];
  activeSessionId: string | null;
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const otherSessions = sessions.filter((s) => s.id !== activeSessionId);

  function statusColor(status: string) {
    switch (status) {
      case "complete":
        return "bg-emerald-500";
      case "analyzing":
        return "bg-blue-500 animate-pulse";
      case "parsed":
        return "bg-amber-500";
      default:
        return "bg-muted-foreground";
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        className="w-full max-w-xl rounded-xl border border-border bg-card p-4 shadow-2xl"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-sm font-semibold">Compare with another HAR</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          The AI will use compare_sessions to diff the two captures.
        </p>
        <div className="mt-3 max-h-96 overflow-auto">
          {otherSessions.length === 0 ? (
            <p className="text-xs text-muted-foreground">No other sessions to compare with.</p>
          ) : (
            otherSessions.map((s) => (
              <button
                key={s.id}
                className="flex w-full flex-col gap-1 rounded-md px-3 py-2 text-left text-xs hover:bg-accent"
                onClick={() => onSelect(s.id)}
              >
                <div className="flex items-start gap-2">
                  <FileJson className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
                  <span className="min-w-0 flex-1 break-all text-xs font-medium leading-snug">
                    {s.file_name}
                  </span>
                  <div
                    className={cn("mt-1 h-2 w-2 shrink-0 rounded-full", statusColor(s.status))}
                    title={s.status}
                  />
                </div>
                <div className="flex items-center gap-2 pl-5 text-[10px] text-muted-foreground">
                  <span>{s.total_entries.toLocaleString()} entries</span>
                  <span>·</span>
                  <span>{formatBytes(s.total_bytes)}</span>
                </div>
                <div className="flex items-center gap-1 pl-5 text-[10px] text-muted-foreground">
                  <Clock className="h-2.5 w-2.5" />
                  <span>{new Date(s.created_at).toLocaleString()}</span>
                </div>
              </button>
            ))
          )}
        </div>
        <div className="mt-3 flex justify-end">
          <Button variant="outline" size="sm" onClick={onClose}>
            Cancel
          </Button>
        </div>
      </div>
    </div>
  );
}
