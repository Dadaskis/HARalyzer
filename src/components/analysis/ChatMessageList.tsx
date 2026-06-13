import { memo, useMemo, Component, type ReactNode } from "react";
import { CheckCircle2, ChevronDown, Loader2, Terminal, XCircle } from "lucide-react";
import { MarkdownContent } from "@/components/markdown/MarkdownContent";
import { CopyButton } from "@/components/ui/copy-button";
import { tryFormatJson } from "@/lib/format-json";
import type { ChatMessage } from "@/lib/types";
import type { ToolActivityItem } from "@/components/analysis/ChatPanel";
import { cn } from "@/lib/utils";

interface ErrorBoundaryProps {
  children: ReactNode;
  fallback?: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error?: Error;
}

class ChatErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: any) {
    console.error("[ChatErrorBoundary] Caught error:", error);
    console.error("[ChatErrorBoundary] Error info:", errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback || (
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
          <p className="font-semibold">Failed to render chat message</p>
          <p className="mt-1 text-xs opacity-80">{this.state.error?.message}</p>
        </div>
      );
    }

    return this.props.children;
  }
}

export interface StreamDraft {
  content: string;
  reasoning: string;
}

const THINKING_PREFIX = "### Thinking\n\n";
const THINKING_SEPARATOR = "\n\n---\n\n";

const ANSWER_MARKERS = ["### Observed", "### Inferred", "### Self-check", "### Limit reached"];

function looksLikeFinalAnswer(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  return ANSWER_MARKERS.some((m) => t.includes(m)) || (t.length > 400 && t.includes("### "));
}

/** Reasoning models often stream the final answer in `reasoning` instead of `content`. */
export function streamVisibleContent(draft: StreamDraft): { answer: string; planning: string | null } {
  const content = draft.content.trim();
  const reasoning = draft.reasoning.trim();

  if (content && !looksLikeFinalAnswer(reasoning)) {
    return { answer: draft.content, planning: reasoning || null };
  }
  if (looksLikeFinalAnswer(reasoning)) {
    for (const marker of ANSWER_MARKERS) {
      const idx = reasoning.indexOf(marker);
      if (idx > 0) {
        return {
          answer: reasoning.slice(idx),
          planning: reasoning.slice(0, idx).trim() || null,
        };
      }
      if (idx === 0) {
        return { answer: reasoning, planning: content || null };
      }
    }
    return { answer: reasoning, planning: content || null };
  }
  if (content) {
    return { answer: draft.content, planning: reasoning || null };
  }
  return { answer: "", planning: reasoning || null };
}

export function streamHasVisibleAnswer(draft: StreamDraft | null): boolean {
  if (!draft) return false;
  return streamVisibleContent(draft).answer.trim().length > 0;
}

export function parseAssistantContent(content: string): {
  reasoning: string | null;
  answer: string;
} {
  if (!content.startsWith(THINKING_PREFIX)) {
    return { reasoning: null, answer: content };
  }

  const rest = content.slice(THINKING_PREFIX.length);
  const sepIdx = rest.indexOf(THINKING_SEPARATOR);
  if (sepIdx === -1) {
    return { reasoning: rest.trim() || null, answer: "" };
  }

  return {
    reasoning: rest.slice(0, sepIdx).trim() || null,
    answer: rest.slice(sepIdx + THINKING_SEPARATOR.length).trim(),
  };
}

const ReasoningDropdown = memo(function ReasoningDropdown({
  reasoning,
  streaming = false,
}: {
  reasoning: string;
  streaming?: boolean;
}) {
  return (
    <details className="group mb-2 rounded-md border border-primary/20 bg-primary/5 open:bg-primary/[0.07]">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2 py-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground marker:content-none [&::-webkit-details-marker]:hidden">
        <ChevronDown className="h-3 w-3 shrink-0 transition-transform group-open:rotate-180" />
        Reasoning
      </summary>
      <div className="border-t border-primary/10 px-2 py-1.5">
        <p className="whitespace-pre-wrap break-words text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
          {reasoning}
          {streaming && (
            <span className="ml-0.5 inline-block animate-pulse text-primary">▋</span>
          )}
        </p>
      </div>
    </details>
  );
});

function toolStatusIcon(status: string) {
  if (status === "running" || status === "thinking") {
    return <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-primary" />;
  }
  if (status === "error") {
    return <XCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-destructive" />;
  }
  return <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />;
}

const ReasoningBlock = memo(function ReasoningBlock({ item }: { item: ToolActivityItem }) {
  if (!item.reasoning && item.status !== "streaming") return null;
  const isStreaming = item.status === "streaming";
  if (isStreaming) {
    return (
      <div className="mb-2">
        <p className="mb-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/70">
          Thinking · step {item.step}
        </p>
        <p className="whitespace-pre-wrap break-words text-xs italic leading-relaxed text-muted-foreground/85 [overflow-wrap:anywhere]">
          {item.reasoning}
          <span className="ml-0.5 inline-block animate-pulse text-primary/70">▋</span>
        </p>
      </div>
    );
  }
  return (
    <details open className="group mb-2 rounded-md border border-primary/20 bg-primary/[0.04]">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground marker:content-none [&::-webkit-details-marker]:hidden">
        <ChevronDown className="h-3 w-3 shrink-0 transition-transform group-open:rotate-180" />
        Thinking · step {item.step}
      </summary>
      <div className="border-t border-primary/10 px-2 py-1.5">
        <p className="whitespace-pre-wrap break-words text-xs italic leading-relaxed text-muted-foreground/85 [overflow-wrap:anywhere]">
          {item.reasoning}
        </p>
      </div>
    </details>
  );
});

function toolDisplayName(tool: string): string {
  switch (tool) {
    case "agent":
      return "Choosing tools";
    case "context-summarize":
      return "Summarizing context";
    case "reasoning":
      return "Thinking";
    case "run_script":
      return "run_script";
    case "edit_script":
      return "edit_script";
    default:
      return tool;
  }
}

const ScriptDiffView = memo(function ScriptDiffView({ diff }: { diff: string }) {
  const lines = useMemo(() => {
    const raw = diff.split("\n").filter(Boolean);
    let end = raw.length;
    while (end > 0) {
      const t = raw[end - 1].trim();
      if (
        t.startsWith("```") ||
        t.startsWith("](#)") ||
        (t.includes("**") && t.includes("120KB"))
      ) {
        end -= 1;
      } else {
        break;
      }
    }
    return raw.slice(0, end);
  }, [diff]);
  if (lines.length === 0) return null;

  return (
    <div className="mt-1.5 overflow-hidden rounded border border-primary/20 bg-black/25">
      <p className="border-b border-primary/15 px-2 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        Script changes
      </p>
      <pre className="p-2 font-mono text-[10px] leading-snug">
        {lines.map((line, i) => (
          <div
            key={`${i}-${line.slice(0, 8)}`}
            className={cn(
              "whitespace-pre [overflow-wrap:anywhere]",
            line.startsWith("+") && "text-emerald-400/95",
              line.startsWith("-") && "text-red-400/95",
              line.startsWith("✓") && "text-emerald-400/95",
              line.startsWith("✗") && "text-red-400/95",
              !line.startsWith("+") &&
                !line.startsWith("-") &&
                !line.startsWith("✓") &&
                !line.startsWith("✗") &&
                "text-muted-foreground"
            )}
          >
            {line}
          </div>
        ))}
      </pre>
    </div>
  );
});

const ToolRow = memo(function ToolRow({ item }: { item: ToolActivityItem }) {
  const detailDisplay = useMemo(() => {
    if (!item.detail) return null;
    if (item.status !== "done" && item.status !== "stalled" && item.tool !== "agent") {
      return item.detail.length > 500 ? `${item.detail.slice(0, 500)}…` : item.detail;
    }
    if (item.tool === "context-summarize") {
      return item.detail;
    }
    const formatted = tryFormatJson(item.detail);
    if (formatted.ok) return formatted.formatted;
    return item.detail;
  }, [item.detail, item.status, item.tool]);

  const detailIsJson = useMemo(() => {
    if (!item.detail || (item.status !== "done" && item.status !== "stalled")) return false;
    return tryFormatJson(item.detail).ok;
  }, [item.detail, item.status]);

  const isStalled = item.status === "stalled";

  return (
    <div className="flex min-w-0 items-start gap-2 text-xs">
      {isStalled ? (
        <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-amber-500" />
      ) : (
        toolStatusIcon(item.status)
      )}
      <div className="min-w-0 flex-1">
        <span className={cn("font-mono text-[11px]", isStalled ? "text-amber-500" : "text-foreground")}>
          {toolDisplayName(item.tool)}
        </span>
        {detailDisplay && (
          detailIsJson ? (
            <pre className="mt-0.5 max-h-60 overflow-auto rounded border border-primary/15 bg-black/20 p-2 font-mono whitespace-pre break-words text-[10px] leading-snug text-muted-foreground [overflow-wrap:anywhere]">
              {detailDisplay}
            </pre>
          ) : (
            <p className="mt-0.5 whitespace-pre-wrap break-words text-[10px] leading-snug text-muted-foreground [overflow-wrap:anywhere]">
              {detailDisplay}
            </p>
          )
        )}
        {item.tool === "run_script" && item.reasoning && (
          <ScriptDiffView diff={item.reasoning} />
        )}
      </div>
    </div>
  );
});

const ToolActivityPanel = memo(function ToolActivityPanel({
  items,
}: {
  items: ToolActivityItem[];
}) {
  if (items.length === 0) return null;

  return (
    <div className="mr-4 min-w-0 max-w-full overflow-hidden rounded-lg border border-primary/20 bg-primary/5 px-3 py-2">
      <p className="mb-2 text-[10px] font-medium uppercase tracking-wide text-primary">
        Looking up HAR data
      </p>
      <div className="space-y-1.5">
        {items.map((item) => {
          if (
            item.tool === "reasoning" &&
            (item.status === "streaming" || item.status === "done" || item.status === "reasoning") &&
            item.reasoning
          ) {
            return <ReasoningBlock key={item.id} item={item} />;
          }
          if (item.tool === "reasoning") return null;
          if (!item.tool.trim() && item.status === "error") return null;
          return <ToolRow key={item.id} item={item} />;
        })}
      </div>
    </div>
  );
});

const ChatMessageBubble = memo(function ChatMessageBubble({ msg }: { msg: ChatMessage }) {
  const parsed = useMemo(
    () => (msg.role === "assistant" ? parseAssistantContent(msg.content) : null),
    [msg.role, msg.content]
  );

  return (
    <div
      className={cn(
        "group/message relative min-w-0 max-w-full overflow-hidden rounded-lg px-3 py-2",
        msg.role === "user"
          ? "ml-8 bg-primary/15"
          : "mr-4 border border-border/50 bg-card/80"
      )}
    >
      <div className="mb-1 flex items-center gap-2">
        <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
          {msg.role === "user" ? "You" : "HARalyzer"}
        </p>
        <CopyButton
          text={msg.content}
          title="Copy message"
          size="sm"
          className="ml-auto opacity-0 transition-opacity group-hover/message:opacity-100 max-sm:opacity-100"
        />
      </div>
      {msg.role === "assistant" ? (
        <>
          {parsed?.reasoning && <ReasoningDropdown reasoning={parsed.reasoning} />}
          {parsed?.answer ? (
            <MarkdownContent content={parsed.answer} className="min-w-0" />
          ) : !parsed?.reasoning ? (
            <MarkdownContent content={msg.content} className="min-w-0" />
          ) : null}
        </>
      ) : (
        <p className="whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
          {msg.content}
        </p>
      )}
    </div>
  );
});

const StreamingBubble = memo(function StreamingBubble({
  draft,
  thinkingMode,
  thinkingModelConfigured,
}: {
  draft: StreamDraft;
  thinkingMode: boolean;
  thinkingModelConfigured: boolean;
}) {
  const { answer, planning } = useMemo(() => streamVisibleContent(draft), [draft]);

  const copyText = useMemo(() => {
    if (planning && answer) {
      return `${THINKING_PREFIX}${planning}${THINKING_SEPARATOR}${answer}`;
    }
    return answer || planning || draft.reasoning || draft.content;
  }, [answer, planning, draft.content, draft.reasoning]);

  return (
    <div className="group/message relative mr-4 min-w-0 max-w-full overflow-hidden rounded-lg border border-primary/30 bg-card/80 px-3 py-2">
      <div className="mb-1 flex items-center gap-2">
        <p className="text-[10px] font-medium uppercase tracking-wide text-primary">
          HARalyzer {thinkingMode && thinkingModelConfigured ? "· thinking" : ""}
        </p>
        {copyText && (
          <CopyButton
            text={copyText}
            title="Copy message"
            size="sm"
            className="ml-auto opacity-0 transition-opacity group-hover/message:opacity-100 max-sm:opacity-100"
          />
        )}
      </div>
      {planning && !looksLikeFinalAnswer(planning) && !answer && (
        <ReasoningDropdown reasoning={planning} streaming />
      )}
      {answer ? (
        <>
          <MarkdownContent content={answer} className="min-w-0" />
          <span className="ml-0.5 inline-block animate-pulse text-primary">▋</span>
        </>
      ) : !planning ? (
        <p className="text-sm text-muted-foreground">
          <Terminal className="mr-1.5 inline h-3.5 w-3.5" />
          Researching HAR data...
        </p>
      ) : null}
    </div>
  );
});

interface ChatMessageListProps {
  messages: ChatMessage[];
  streamDraft: StreamDraft | null;
  toolActivity: ToolActivityItem[];
  sending: boolean;
  thinkingMode: boolean;
  thinkingModelConfigured: boolean;
  agentPaused?: boolean;
}

export const ChatMessageList = memo(function ChatMessageList({
  messages,
  streamDraft,
  toolActivity,
  sending,
  thinkingMode,
  thinkingModelConfigured,
  agentPaused = false,
}: ChatMessageListProps) {
  const showToolPanel =
    !streamHasVisibleAnswer(streamDraft) &&
    (toolActivity.length > 0 || (sending && !streamDraft) || agentPaused);

  if (messages.length === 0 && !streamDraft && !sending) {
    return (
      <p className="text-center text-xs text-muted-foreground">
        Ask about endpoints, errors, auth, or JS fetch logic. The assistant will query the HAR
        session with tools before answering.
      </p>
    );
  }

  const lastUserIdx = (() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === "user") return i;
    }
    return -1;
  })();

  const renderedMessages: ReactNode[] = [];
  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i];
    renderedMessages.push(
      <ChatErrorBoundary key={msg.id} fallback={
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
          <p className="font-semibold">Failed to render message #{msg.id}</p>
          <p className="mt-1 text-xs opacity-80">This message may contain corrupted data or be too large to render.</p>
        </div>
      }>
        <ChatMessageBubble msg={msg} />
      </ChatErrorBoundary>
    );
    if (i === lastUserIdx && showToolPanel) {
      renderedMessages.push(
        toolActivity.length > 0 ? (
          <ToolActivityPanel key="tool-activity" items={toolActivity} />
        ) : (
          <div key="tool-activity" className="mr-4 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs text-muted-foreground">
            <Loader2 className="mr-1.5 inline h-3.5 w-3.5 animate-spin text-primary" />
            Starting HAR lookup…
          </div>
        )
      );
    }
  }

  return (
    <div className="space-y-4">
      {renderedMessages}
      {streamDraft && (streamDraft.content || streamDraft.reasoning) && (
        <StreamingBubble
          draft={streamDraft}
          thinkingMode={thinkingMode}
          thinkingModelConfigured={thinkingModelConfigured}
        />
      )}
    </div>
  );
});
