import { memo, useState } from "react";
import { Send, Brain, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

interface ChatInputBarProps {
  sending: boolean;
  thinkingMode: boolean;
  thinkingModelConfigured: boolean;
  thinkingModelLabel: string;
  onThinkingModeChange: (value: boolean) => void;
  onSend: (text: string) => void;
  onStop?: () => void;
}

export const ChatInputBar = memo(function ChatInputBar({
  sending,
  thinkingMode,
  thinkingModelConfigured,
  thinkingModelLabel,
  onThinkingModeChange,
  onSend,
  onStop,
}: ChatInputBarProps) {
  const [input, setInput] = useState("");

  const handleSend = () => {
    const text = input.trim();
    if (!text || sending) return;
    setInput("");
    onSend(text);
  };

  return (
    <div className="space-y-2 border-t border-primary/10 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant={thinkingMode ? "default" : "outline"}
          className={cn(
            "h-8 gap-1.5 text-xs",
            thinkingMode && "shadow-[0_0_12px_oklch(0.6_0.22_295/0.35)]"
          )}
          onClick={() => onThinkingModeChange(!thinkingMode)}
          disabled={!thinkingModelConfigured}
          title={
            thinkingModelConfigured
              ? `Use thinking model (${thinkingModelLabel})`
              : "Set a Thinking Model in Settings first"
          }
        >
          <Brain className="h-3.5 w-3.5" />
          Thinking
        </Button>
        {!thinkingModelConfigured && (
          <span className="text-[10px] text-muted-foreground">
            Configure thinking model in Settings
          </span>
        )}
      </div>
      {thinkingMode && thinkingModelConfigured && (
        <p className="rounded-md border border-amber-500/25 bg-amber-500/10 px-2.5 py-1.5 text-[10px] leading-snug text-amber-100/85">
          Thinking mode uses your reasoning model without HAR tool lookups — better for open-ended
          analysis. Turn it off to let the agent query this capture with tools.
        </p>
      )}
      <div className="flex items-end gap-2">
        <Textarea
          placeholder="Ask a follow-up question… (Shift+Enter for new line)"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
          disabled={sending}
          rows={3}
          className="max-h-40 min-h-[72px] resize-y text-sm leading-relaxed"
        />
        <Button
          size="icon"
          className={cn("shrink-0", sending && "hidden")}
          onClick={handleSend}
          disabled={sending || !input.trim()}
        >
          <Send />
        </Button>
        {sending && (
          <Button
            size="icon"
            variant="destructive"
            className="shrink-0"
            onClick={onStop}
            title="Stop processing"
          >
            <Square className="h-3.5 w-3.5 fill-current" />
          </Button>
        )}
      </div>
    </div>
  );
});
