import { useCallback, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { ChevronDown, MessageSquare, Trash2 } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ChatInputBar } from "@/components/analysis/ChatInputBar";
import { ChatMessageList, type StreamDraft } from "@/components/analysis/ChatMessageList";
import { api, CHAT_CANCELLED_MESSAGE } from "@/lib/api";
import { useSettingsStore } from "@/store/app-store";
import type {
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
  stepsUsed: number;
  stepLimit: number;
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
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sending, setSending] = useState(false);
  const [thinkingMode, setThinkingMode] = useState(false);
  const [streamDraft, setStreamDraft] = useState<StreamDraft | null>(null);
  const [toolActivity, setToolActivity] = useState<ToolActivityItem[]>([]);
  const [agentContinuePrompt, setAgentContinuePrompt] = useState<AgentContinuePrompt | null>(
    null
  );
  const [showJumpToBottom, setShowJumpToBottom] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const stickToBottomRef = useRef(true);
  const streamDraftRef = useRef<StreamDraft>({ content: "", reasoning: "" });
  const streamRafRef = useRef<number | null>(null);
  const continueResolverRef = useRef<((value: boolean) => void) | null>(null);
  const toolActivityOrderRef = useRef(0);
  const toolActivityFallbackIdRef = useRef(0);

  const upsertToolActivity = useCallback((item: Omit<ToolActivityItem, "order">) => {
    flushSync(() => {
      setToolActivity((prev) => {
        let next = prev.filter((entry) => entry.id !== item.id);
        if (item.tool !== "agent" && item.status === "running") {
          next = next.filter((entry) => entry.id !== "agent-planning");
        }
        if (item.tool === "agent" && item.status === "done") {
          next = next.filter((entry) => entry.id !== "agent-planning");
        }
        const previousPlanning = prev.find((entry) => entry.id === "agent-planning");
        const order =
          item.id === "agent-planning"
            ? (previousPlanning?.order ?? toolActivityOrderRef.current++)
            : toolActivityOrderRef.current++;
        next.push({ ...item, order });
        return sortToolActivity(next);
      });
    });
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
    (stepsUsed: number, stepLimit: number) =>
      new Promise<boolean>((resolve) => {
        setAgentContinuePrompt({ stepsUsed, stepLimit });
        continueResolverRef.current = resolve;
      }),
    []
  );

  const clearAgentUi = useCallback(() => {
    setStreamDraft(null);
    setToolActivity([]);
    setAgentContinuePrompt(null);
    if (streamRafRef.current !== null) {
      cancelAnimationFrame(streamRafRef.current);
      streamRafRef.current = null;
    }
  }, []);

  const runAgentWithContinuations = useCallback(
    async (initialSend: () => Promise<ChatSendResult>) => {
      let result = await initialSend();

      while (result.needs_continue) {
        const shouldContinue = await waitForContinueDecision(
          result.steps_used,
          result.step_limit
        );
        setAgentContinuePrompt(null);

        if (!shouldContinue) {
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
      api.getChatMessages(sessionId).then(setMessages).catch(console.error);
    } else {
      setMessages([]);
    }
    setStreamDraft(null);
    setToolActivity([]);
    setAgentContinuePrompt(null);
    toolActivityOrderRef.current = 0;
    toolActivityFallbackIdRef.current = 0;
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
        setStreamDraft(null);
        setToolActivity([]);
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
      const { id, step, tool, status, detail } = event.payload;
      upsertToolActivity({
        id: id || `tool-${toolActivityFallbackIdRef.current++}`,
        step,
        tool,
        status,
        detail,
      });
    }).then((fn) => {
      unlistenTool = fn;
    });

    listen<string>("chat-cancelled", (event) => {
      if (event.payload !== sessionId) return;
      clearAgentUi();
    }).then((fn) => {
      unlistenCancel = fn;
    });

    return () => {
      unlistenStream?.();
      unlistenTool?.();
      unlistenCancel?.();
      if (streamRafRef.current !== null) {
        cancelAnimationFrame(streamRafRef.current);
      }
    };
  }, [sessionId, upsertToolActivity, clearAgentUi]);

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

      try {
        const context: ChatContext | undefined = entryContext
          ? { context_type: "entry", entry_index: entryContext.summary.index }
          : undefined;

        await runAgentWithContinuations(() =>
          api.sendChatMessage(sessionId, text, context, thinkingMode)
        );

        const all = await api.getChatMessages(sessionId);
        setMessages(all);
      } catch (err) {
        const message = String(err);
        if (message.includes(CHAT_CANCELLED_MESSAGE)) {
          const all = await api.getChatMessages(sessionId);
          setMessages(all);
        } else {
          console.error(err);
          alert(message);
          setMessages((m) => m.filter((x) => x.id !== optimistic.id));
        }
      } finally {
        clearAgentUi();
        continueResolverRef.current = null;
        setSending(false);
      }
    },
    [sessionId, sending, entryContext, thinkingMode, scrollToBottom, runAgentWithContinuations, clearAgentUi]
  );

  const handleStop = useCallback(async () => {
    if (!sessionId || !sending) return;
    if (agentContinuePrompt) {
      continueResolverRef.current?.(false);
      continueResolverRef.current = null;
      return;
    }
    try {
      await api.cancelChatAgent(sessionId);
    } catch (err) {
      console.error(err);
    }
  }, [sessionId, sending, agentContinuePrompt]);

  const handleContinueAgent = useCallback(() => {
    continueResolverRef.current?.(true);
    continueResolverRef.current = null;
  }, []);

  const handleCancelAgent = useCallback(() => {
    continueResolverRef.current?.(false);
    continueResolverRef.current = null;
  }, []);

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
      <div className="flex items-center justify-end gap-2 border-b border-primary/10 px-4 py-1.5">
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
            <ChatMessageList
              messages={messages}
              streamDraft={streamDraft}
              toolActivity={toolActivity}
              sending={sending}
              thinkingMode={thinkingMode}
              thinkingModelConfigured={thinkingModelConfigured}
            />
          </div>
        </ScrollArea>

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
          <p className="font-medium text-amber-100/90">
            Tool step limit reached ({agentContinuePrompt.stepsUsed} /{" "}
            {agentContinuePrompt.stepLimit})
          </p>
          <p className="mt-1 text-muted-foreground">
            Continue with {chatAgentMaxSteps} more tool steps, or finish now with a summary of what
            was found.
          </p>
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
    </div>
  );
}
