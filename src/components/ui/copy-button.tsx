import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface CopyButtonProps {
  text: string;
  title?: string;
  className?: string;
  size?: "icon" | "sm";
}

export function CopyButton({ text, title = "Copy", className, size = "icon" }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  };

  if (size === "sm") {
    return (
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className={cn("h-6 gap-1 px-2 text-[10px] text-muted-foreground", className)}
        onClick={handleCopy}
        title={copied ? "Copied!" : title}
      >
        {copied ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
        {copied ? "Copied" : "Copy"}
      </Button>
    );
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className={cn(
        "h-7 w-7 border border-border/40 bg-background/80 text-muted-foreground backdrop-blur-sm",
        "opacity-70 transition-opacity hover:bg-accent hover:text-foreground hover:opacity-100",
        className
      )}
      onClick={handleCopy}
      title={copied ? "Copied!" : title}
      aria-label={copied ? "Copied" : title}
    >
      {copied ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
    </Button>
  );
}
