import { useMemo, useState } from "react";
import hljs from "highlight.js/lib/core";
import json from "highlight.js/lib/languages/json";
import javascript from "highlight.js/lib/languages/javascript";
import { CopyButton } from "@/components/ui/copy-button";
import { Button } from "@/components/ui/button";
import { CodeScrollArea } from "@/components/entries/CodeScrollArea";
import { formatJsonForDisplay, isTruncatedBody } from "@/lib/format-json";
import { isJavaScriptMime } from "@/lib/mime";
import { cn } from "@/lib/utils";

hljs.registerLanguage("json", json);
hljs.registerLanguage("javascript", javascript);

interface BodyViewerProps {
  body: string;
  mimeType?: string;
  emptyLabel?: string;
  className?: string;
  isJavaScript?: boolean;
  fill?: boolean;
  sessionId?: string;
  entryIndex?: number;
  bodyField?: "request" | "response";
}

export function BodyViewer({
  body,
  mimeType,
  emptyLabel = "No body",
  className,
  isJavaScript,
  fill = false,
  sessionId,
  entryIndex,
  bodyField,
}: BodyViewerProps) {
  const [expandedBody, setExpandedBody] = useState<string | null>(null);
  const [loadingFull, setLoadingFull] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const activeBody = expandedBody ?? body;
  const truncated = expandedBody === null && isTruncatedBody(body);
  const canLoadFull =
    truncated && sessionId != null && entryIndex != null && bodyField != null;

  const { kind, highlighted, truncatedNote } = useMemo(() => {
    if (!activeBody) {
      return {
        kind: "empty" as const,
        highlighted: "",
        truncatedNote: null as string | null,
      };
    }

    const { displayText, isJson: asJson } = formatJsonForDisplay(activeBody, mimeType);
    const truncatedNote =
      expandedBody === null && activeBody.includes("[truncated")
        ? (activeBody.match(/\[truncated[^\]]*\]/)?.[0] ?? null)
        : null;

    const asJs = isJavaScript ?? isJavaScriptMime(mimeType);

    if (asJson) {
      return {
        kind: "json" as const,
        highlighted: hljs.highlight(displayText, { language: "json" }).value,
        truncatedNote,
      };
    }

    if (asJs) {
      try {
        return {
          kind: "javascript" as const,
          highlighted: hljs.highlight(displayText, { language: "javascript" }).value,
          truncatedNote,
        };
      } catch {
        return {
          kind: "javascript" as const,
          highlighted: hljs.highlightAuto(displayText).value,
          truncatedNote,
        };
      }
    }

    return {
      kind: "raw" as const,
      highlighted: hljs.highlightAuto(displayText).value,
      truncatedNote,
    };
  }, [activeBody, mimeType, isJavaScript, expandedBody]);

  const handleLoadFull = async () => {
    if (!canLoadFull) return;
    setLoadingFull(true);
    setLoadError(null);
    try {
      const { api } = await import("@/lib/api");
      const full = await api.getEntryBodyFull(sessionId!, entryIndex!);
      setExpandedBody(bodyField === "request" ? full.request_body : full.response_body);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoadingFull(false);
    }
  };

  if (!body) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }

  const label =
    kind === "json" ? "JSON" : kind === "javascript" ? "JavaScript" : "Raw";

  const preserveLines = fill || kind !== "raw";

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border border-primary/25 bg-[oklch(0.11_0.04_290)]",
        fill && "flex min-h-0 flex-1 flex-col",
        className
      )}
    >
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-primary/15 bg-[oklch(0.13_0.05_292)] px-3 py-1.5">
        <span className="text-[10px] font-medium uppercase tracking-wide text-primary/80">
          {label}
        </span>
        {mimeType && (
          <span className="truncate font-mono text-[10px] text-muted-foreground">{mimeType}</span>
        )}
        <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">
          {truncatedNote && (
            <span className="text-[10px] text-amber-400/90">{truncatedNote}</span>
          )}
          {canLoadFull && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-6 px-2 text-[10px]"
              disabled={loadingFull}
              onClick={handleLoadFull}
            >
              {loadingFull ? "Loading…" : "Show full body"}
            </Button>
          )}
          {expandedBody && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[10px]"
              onClick={() => setExpandedBody(null)}
            >
              Show preview
            </Button>
          )}
          {(kind === "json" || kind === "javascript") && (
            <CopyButton
              text={activeBody}
              title={kind === "json" ? "Copy JSON" : "Copy JavaScript"}
              size="sm"
            />
          )}
        </div>
      </div>
      {loadError && (
        <p className="border-b border-destructive/20 bg-destructive/10 px-3 py-1 text-[10px] text-destructive">
          {loadError}
        </p>
      )}
      {preserveLines ? (
        <CodeScrollArea fill={fill}>
          <pre className="p-3 text-[11px] leading-relaxed">
            <code
              className="hljs block whitespace-pre font-mono [overflow-wrap:normal]"
              dangerouslySetInnerHTML={{ __html: highlighted }}
            />
          </pre>
        </CodeScrollArea>
      ) : (
        <pre
          className={cn(
            "max-h-56 overflow-auto p-3 text-[11px] leading-relaxed",
            "whitespace-pre-wrap break-words [overflow-wrap:anywhere]"
          )}
        >
          <code
            className="hljs block whitespace-pre-wrap break-words font-mono"
            dangerouslySetInnerHTML={{ __html: highlighted }}
          />
        </pre>
      )}
    </div>
  );
}

export function PlainBodyViewer({
  body,
  emptyLabel = "No body",
  className,
}: {
  body: string;
  emptyLabel?: string;
  className?: string;
}) {
  if (!body) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }

  return (
    <pre
      className={cn(
        "max-h-56 overflow-auto rounded-lg border border-primary/25 bg-[oklch(0.11_0.04_290)] p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
        className
      )}
    >
      {body}
    </pre>
  );
}
