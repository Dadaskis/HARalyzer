import { memo, useMemo } from "react";
import { CheckCircle2, ChevronDown, Loader2, Terminal, XCircle } from "lucide-react";
import { MarkdownContent } from "@/components/markdown/MarkdownContent";
import type { ChatMessage } from "@/lib/types";
import type { ToolActivityItem } from "@/components/analysis/ChatPanel";

export interface StreamDraft {
  content: string;
  reasoning: string;
}

const THINKING_PREFIX = "### Thinking\n\n";
const THINKING_SEPARATOR = "\n\n---\n\n";

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

interface ChatMessageListProps {
  messages: ChatMessage[];
  streamDraft: StreamDraft | null;
  toolActivity: ToolActivityItem[];
  sending: boolean;
  thinkingMode: boolean;
  thinkingModelConfigured: boolean;
}

function toolStatusIcon(status: string) {
  if (status === "running" || status === "thinking") {
    return <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-primary" />;
  }
  if (status === "error") {
    return <XCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-destructive" />;
  }
  return <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />;
}

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
      <ul className="space-y-1.5">
        {items.map((item) => (
          <li key={item.id} className="flex min-w-0 items-start gap-2 text-xs">
            {toolStatusIcon(item.status)}
            <div className="min-w-0 flex-1">
              <span className="font-mono text-[11px] text-foreground">
                {item.tool === "agent" ? "Choosing tools" : item.tool}
              </span>
              {item.detail && (
                <p className="mt-0.5 break-words text-[10px] leading-snug text-muted-foreground [overflow-wrap:anywhere]">
                  {item.tool === "agent" || item.status === "done"
                    ? item.detail
                    : item.detail.length > 100
                      ? `${item.detail.slice(0, 100)}…`
                      : item.detail}
                </p>
              )}
            </div>
          </li>
        ))}
      </ul>
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
      className={
        msg.role === "user"
          ? "ml-8 min-w-0 max-w-full overflow-hidden rounded-lg bg-primary/15 px-3 py-2"
          : "mr-4 min-w-0 max-w-full overflow-hidden rounded-lg border border-border/50 bg-card/80 px-3 py-2"
      }
    >
      <p className="mb-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {msg.role === "user" ? "You" : "HARalyzer"}
      </p>
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
  return (
    <div className="mr-4 min-w-0 max-w-full overflow-hidden rounded-lg border border-primary/30 bg-card/80 px-3 py-2">
      <p className="mb-1 text-[10px] font-medium uppercase tracking-wide text-primary">
        HARalyzer {thinkingMode && thinkingModelConfigured ? "· thinking" : ""}
      </p>
      {draft.reasoning && (
        <ReasoningDropdown reasoning={draft.reasoning} streaming={!draft.content} />
      )}
      {draft.content ? (
        <p className="whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
          {draft.content}
          <span className="ml-0.5 inline-block animate-pulse text-primary">▋</span>
        </p>
      ) : !draft.reasoning ? (
        <p className="text-sm text-muted-foreground">
          <Terminal className="mr-1.5 inline h-3.5 w-3.5" />
          Researching HAR data...
        </p>
      ) : null}
    </div>
  );
});

export const ChatMessageList = memo(function ChatMessageList({
  messages,
  streamDraft,
  toolActivity,
  sending,
  thinkingMode,
  thinkingModelConfigured,
}: ChatMessageListProps) {
  const showToolPanel = toolActivity.length > 0 || (sending && !streamDraft);

  return (
    <>
      {messages.length === 0 && !streamDraft && !sending && (
        <p className="text-center text-xs text-muted-foreground">
          Ask about endpoints, errors, auth, or JS fetch logic. The assistant will query the HAR
          session with tools before answering.
        </p>
      )}
      {messages.map((msg) => (
        <ChatMessageBubble key={msg.id} msg={msg} />
      ))}
      {showToolPanel && (
        toolActivity.length > 0 ? (
          <ToolActivityPanel items={toolActivity} />
        ) : (
          <div className="mr-4 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs text-muted-foreground">
            <Loader2 className="mr-1.5 inline h-3.5 w-3.5 animate-spin text-primary" />
            Starting HAR lookup…
          </div>
        )
      )}
      {streamDraft && (streamDraft.content || streamDraft.reasoning) && (
        <StreamingBubble
          draft={streamDraft}
          thinkingMode={thinkingMode}
          thinkingModelConfigured={thinkingModelConfigured}
        />
      )}
    </>
  );
});
