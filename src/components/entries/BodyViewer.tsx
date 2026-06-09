import { useMemo } from "react";
import hljs from "highlight.js/lib/core";
import json from "highlight.js/lib/languages/json";
import { cn } from "@/lib/utils";

hljs.registerLanguage("json", json);

interface BodyViewerProps {
  body: string;
  mimeType?: string;
  emptyLabel?: string;
  className?: string;
}

function tryParseJson(text: string): { ok: true; formatted: string } | { ok: false } {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
    return { ok: false };
  }
  try {
    const parsed = JSON.parse(trimmed);
    return { ok: true, formatted: JSON.stringify(parsed, null, 2) };
  } catch {
    return { ok: false };
  }
}

function isJsonMime(mime?: string) {
  return !!mime && (mime.includes("json") || mime.includes("+json"));
}

export function BodyViewer({
  body,
  mimeType,
  emptyLabel = "No body",
  className,
}: BodyViewerProps) {
  const { isJson, highlighted } = useMemo(() => {
    if (!body) {
      return { isJson: false, highlighted: "" };
    }

    const jsonResult = tryParseJson(body);
    const treatAsJson = jsonResult.ok || isJsonMime(mimeType);

    if (jsonResult.ok) {
      const formatted = jsonResult.formatted;
      return {
        isJson: true,
        highlighted: hljs.highlight(formatted, { language: "json" }).value,
      };
    }

    if (treatAsJson) {
      return {
        isJson: true,
        highlighted: hljs.highlight(body, { language: "json" }).value,
      };
    }

    return {
      isJson: false,
      highlighted: hljs.highlightAuto(body).value,
    };
  }, [body, mimeType]);

  if (!body) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border border-primary/25 bg-[oklch(0.11_0.04_290)]",
        className
      )}
    >
      <div className="flex items-center gap-2 border-b border-primary/15 bg-[oklch(0.13_0.05_292)] px-3 py-1.5">
        <span className="text-[10px] font-medium uppercase tracking-wide text-primary/80">
          {isJson ? "JSON" : "Raw"}
        </span>
        {mimeType && (
          <span className="truncate font-mono text-[10px] text-muted-foreground">{mimeType}</span>
        )}
      </div>
      <pre className="max-h-56 overflow-auto p-3 text-[11px] leading-relaxed">
        <code
          className={cn(
            "hljs block whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
            !isJson && "font-mono"
          )}
          dangerouslySetInnerHTML={{ __html: highlighted }}
        />
      </pre>
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
