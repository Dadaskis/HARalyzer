import { useMemo, useState } from "react";
import hljs from "highlight.js/lib/core";
import javascript from "highlight.js/lib/languages/javascript";
import { CopyButton } from "@/components/ui/copy-button";
import { CodeScrollArea } from "@/components/entries/CodeScrollArea";
import { cn } from "@/lib/utils";

hljs.registerLanguage("javascript", javascript);

interface JsCodeViewerProps {
  code: string;
  label?: string;
  emptyLabel?: string;
  className?: string;
  showCopy?: boolean;
  /** When true, fills the parent flex column and owns the only scroll region. */
  fill?: boolean;
}

export function JsCodeViewer({
  code,
  label = "JavaScript",
  emptyLabel = "No source",
  className,
  showCopy = true,
  fill = false,
}: JsCodeViewerProps) {
  const [wrapText, setWrapText] = useState(false);

  const highlighted = useMemo(() => {
    if (!code) return "";
    try {
      return hljs.highlight(code, { language: "javascript" }).value;
    } catch {
      return hljs.highlightAuto(code).value;
    }
  }, [code]);

  if (!code) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border border-primary/25 bg-[oklch(0.11_0.04_290)]",
        fill && "flex min-h-0 flex-1 flex-col",
        className
      )}
    >
      <div className="flex shrink-0 items-center gap-2 border-b border-primary/15 bg-[oklch(0.13_0.05_292)] px-3 py-1.5">
        <span className="text-[10px] font-medium uppercase tracking-wide text-primary/80">
          {label}
        </span>
        <label className="ml-auto flex cursor-pointer items-center gap-1 text-[10px] text-muted-foreground">
          <input
            type="checkbox"
            checked={wrapText}
            onChange={(e) => setWrapText(e.target.checked)}
            className="rounded border-input"
          />
          Wrap
        </label>
        {showCopy && (
          <div className="shrink-0">
            <CopyButton text={code} title="Copy JavaScript" size="sm" />
          </div>
        )}
      </div>
      <CodeScrollArea fill={fill} forceWrap={wrapText}>
        <pre className="p-3 text-[11px] leading-relaxed">
          <code
            className={cn(
              "hljs block font-mono",
              wrapText ? "whitespace-pre-wrap [overflow-wrap:anywhere]" : "whitespace-pre [overflow-wrap:normal]"
            )}
            dangerouslySetInnerHTML={{ __html: highlighted }}
          />
        </pre>
      </CodeScrollArea>
    </div>
  );
}
