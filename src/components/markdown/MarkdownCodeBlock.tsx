import { useMemo, useState, type ReactNode } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { highlightCode, normalizeHighlightLanguage } from "@/lib/highlight";
import { tryFormatJson } from "@/lib/format-json";
import { cn } from "@/lib/utils";

function extractText(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(extractText).join("");
  if (node && typeof node === "object" && "props" in node) {
    const element = node as { props: { children?: ReactNode } };
    return extractText(element.props.children);
  }
  return "";
}

interface MarkdownCodeBlockProps {
  children: ReactNode;
  wrap?: boolean;
  language?: string;
}

function extractLanguage(children: ReactNode): string | undefined {
  if (!children || typeof children !== "object" || !("props" in children)) return undefined;
  const code = children as { props?: { className?: string } };
  const className = code.props?.className ?? "";
  const match = className.match(/language-([\w-]+)/);
  return match?.[1];
}

export function MarkdownCodeBlock({ children, wrap = true, language }: MarkdownCodeBlockProps) {
  const [copied, setCopied] = useState(false);
  const rawText = useMemo(() => extractText(children).replace(/\n$/, ""), [children]);
  const detectedLanguage = language ?? extractLanguage(children);

  const text = useMemo(() => {
    const lang = normalizeHighlightLanguage(detectedLanguage);
    if (lang === "json") {
      const formatted = tryFormatJson(rawText);
      if (formatted.ok) return formatted.formatted;
    }
    if (!detectedLanguage) {
      const formatted = tryFormatJson(rawText);
      if (formatted.ok) return formatted.formatted;
    }
    return rawText;
  }, [rawText, detectedLanguage]);

  const highlighted = useMemo(
    () => highlightCode(text, detectedLanguage),
    [text, detectedLanguage]
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy code:", err);
    }
  };

  return (
    <div className="group/code relative my-3 max-w-full">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className={cn(
          "absolute right-1.5 top-1.5 z-10 h-7 w-7",
          "border border-border/40 bg-background/80 text-muted-foreground backdrop-blur-sm",
          "opacity-70 transition-opacity hover:bg-accent hover:text-foreground hover:opacity-100",
          "group-hover/code:opacity-100"
        )}
        onClick={handleCopy}
        title={copied ? "Copied!" : "Copy code"}
        aria-label={copied ? "Copied" : "Copy code"}
      >
        {copied ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
      </Button>
      <pre
        className={cn(
          "max-w-full rounded-lg border border-border/60 bg-black/40 p-3 pr-10 pt-9 text-xs",
          wrap
            ? "whitespace-pre-wrap [overflow-wrap:anywhere] [word-break:break-word]"
            : "overflow-x-auto whitespace-pre"
        )}
      >
        <code
          className={cn("hljs font-mono", wrap && "[overflow-wrap:anywhere] [word-break:break-word]")}
          dangerouslySetInnerHTML={{ __html: highlighted }}
        />
      </pre>
    </div>
  );
}
