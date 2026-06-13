import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Loader2, Sparkles, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { JsCodeViewer } from "@/components/entries/JsCodeViewer";
import { api } from "@/lib/api";
import type { JsDeobfuscateStreamEvent } from "@/lib/types";
import { cn } from "@/lib/utils";

interface EntryJsViewerProps {
  sessionId: string;
  entryIndex: number;
  source: string;
  deobfuscatedJs?: string | null;
  onDeobfuscated?: (code: string) => void;
}

type ViewMode = "original" | "deobfuscated";

export function EntryJsViewer({
  sessionId,
  entryIndex,
  source,
  deobfuscatedJs,
  onDeobfuscated,
}: EntryJsViewerProps) {
  const [view, setView] = useState<ViewMode>(
    deobfuscatedJs?.trim() ? "deobfuscated" : "original"
  );
  const [cached, setCached] = useState(deobfuscatedJs ?? "");
  const [streaming, setStreaming] = useState("");
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setCached(deobfuscatedJs ?? "");
    if (deobfuscatedJs?.trim()) {
      setView("deobfuscated");
    }
  }, [deobfuscatedJs, entryIndex]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    listen<JsDeobfuscateStreamEvent>("js-deobfuscate-stream", (event) => {
      const payload = event.payload;
      if (payload.session_id !== sessionId || payload.entry_index !== entryIndex) {
        return;
      }

      if (payload.error) {
        setError(payload.error);
        setIsRunning(false);
        setStreaming("");
        return;
      }

      if (payload.done) {
        setCached(payload.content);
        setStreaming("");
        setIsRunning(false);
        setError(null);
        setView("deobfuscated");
        onDeobfuscated?.(payload.content);
      } else {
        setStreaming(payload.content);
        setIsRunning(true);
        setError(null);
        setView("deobfuscated");
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, [sessionId, entryIndex, onDeobfuscated]);

  const handleDeobfuscate = useCallback(
    async (force = false) => {
      setError(null);
      setIsRunning(true);
      setStreaming("");
      setView("deobfuscated");
      try {
        await api.deobfuscateJs(sessionId, entryIndex, force);
      } catch (err) {
        setIsRunning(false);
        setError(String(err));
      }
    },
    [sessionId, entryIndex]
  );

  const displayCode =
    view === "deobfuscated" ? (isRunning ? streaming : cached) : source;

  const hasCached = !!cached.trim();

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 pt-2">
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <div className="inline-flex rounded-md border border-primary/20 bg-[oklch(0.14_0.05_290)] p-0.5">
          <button
            type="button"
            className={cn(
              "rounded px-2.5 py-1 text-[11px] font-medium transition-colors",
              view === "original"
                ? "bg-primary/20 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => setView("original")}
          >
            Original
          </button>
          <button
            type="button"
            className={cn(
              "rounded px-2.5 py-1 text-[11px] font-medium transition-colors",
              view === "deobfuscated"
                ? "bg-primary/20 text-foreground"
                : "text-muted-foreground hover:text-foreground",
              !hasCached && !isRunning && "opacity-60"
            )}
            onClick={() => setView("deobfuscated")}
            disabled={!hasCached && !isRunning}
          >
            Deobfuscated
          </button>
        </div>

        <Button
          size="sm"
          variant="outline"
          disabled={isRunning || !source.trim()}
          onClick={() => void handleDeobfuscate(hasCached)}
        >
          {isRunning ? (
            <>
              <Loader2 className="animate-spin" />
              Deobfuscating…
            </>
          ) : hasCached ? (
            <>
              <RefreshCw />
              Re-deobfuscate
            </>
          ) : (
            <>
              <Sparkles />
              Deobfuscate with AI
            </>
          )}
        </Button>

        {isRunning && (
          <span className="text-[10px] text-muted-foreground">Streaming from LLM…</span>
        )}
      </div>

      {error && (
        <p className="shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </p>
      )}

      <JsCodeViewer
        fill
        code={displayCode}
        label={view === "deobfuscated" ? "Deobfuscated JavaScript" : "Original JavaScript"}
        emptyLabel="No source available"
      />
    </div>
  );
}
