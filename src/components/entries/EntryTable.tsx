import { useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { HarEntrySummary } from "@/lib/types";
import { matchesResourceFilter, type ResourceFilterId } from "@/lib/entry-filters";
import { formatBytes, formatDuration, cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";

interface EntryTableProps {
  entries: HarEntrySummary[];
  searchQuery: string;
  methodFilter: string;
  statusFilter: string;
  resourceFilter: ResourceFilterId;
  selectedIndex: number | null;
  onSelectEntry: (index: number) => void;
}

function statusVariant(status: number): "success" | "destructive" | "secondary" {
  if (status >= 200 && status < 300) return "success";
  if (status >= 400) return "destructive";
  return "secondary";
}

export function EntryTable({
  entries,
  searchQuery,
  methodFilter,
  statusFilter,
  resourceFilter,
  selectedIndex,
  onSelectEntry,
}: EntryTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const q = searchQuery.toLowerCase();
    return entries.filter((e) => {
      if (methodFilter !== "all" && e.method !== methodFilter) return false;
      if (statusFilter === "success" && (e.status < 200 || e.status >= 300)) return false;
      if (statusFilter === "error" && e.status < 400) return false;
      if (statusFilter === "redirect" && (e.status < 300 || e.status >= 400)) return false;
      if (!matchesResourceFilter(e, resourceFilter)) return false;
      if (q && !e.url.toLowerCase().includes(q) && !e.method.toLowerCase().includes(q))
        return false;
      return true;
    });
  }, [entries, searchQuery, methodFilter, statusFilter, resourceFilter]);

  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 40,
    overscan: 20,
  });

  if (entries.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        Open a HAR file to view entries
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="grid shrink-0 grid-cols-[60px_70px_1fr_70px_100px_80px_80px] gap-2 border-b border-primary/10 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>#</span>
        <span>Method</span>
        <span>URL</span>
        <span>Status</span>
        <span>Type</span>
        <span>Size</span>
        <span>Time</span>
      </div>
      <div ref={parentRef} className="min-h-0 flex-1 overflow-auto">
        <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const entry = filtered[virtualRow.index];
            return (
              <div
                key={entry.index}
                role="button"
                tabIndex={0}
                onClick={() => onSelectEntry(entry.index)}
                onKeyDown={(e) => e.key === "Enter" && onSelectEntry(entry.index)}
                className={cn(
                  "absolute left-0 top-0 grid w-full cursor-pointer grid-cols-[60px_70px_1fr_70px_100px_80px_80px] gap-2 border-b border-border/30 px-4 py-2 text-xs transition-colors hover:bg-primary/10",
                  selectedIndex === entry.index && "bg-primary/15 ring-1 ring-inset ring-primary/30"
                )}
                style={{
                  height: `${virtualRow.size}px`,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <span className="text-muted-foreground">{entry.index}</span>
                <Badge variant="outline" className="w-fit font-mono">
                  {entry.method}
                </Badge>
                <span className="flex items-center gap-1 truncate font-mono" title={entry.url}>
                  {entry.is_javascript && (
                    <span className="shrink-0 rounded bg-amber-500/20 px-1 text-[9px] text-amber-300">
                      JS
                    </span>
                  )}
                  {entry.url}
                </span>
                <Badge variant={statusVariant(entry.status)} className="w-fit">
                  {entry.status}
                </Badge>
                <span className="truncate text-muted-foreground">{entry.mime_type || "—"}</span>
                <span className="text-muted-foreground">{formatBytes(entry.size)}</span>
                <span className="text-muted-foreground">{formatDuration(entry.time_ms)}</span>
              </div>
            );
          })}
        </div>
      </div>
      <div className="shrink-0 border-t border-primary/10 px-4 py-1.5 text-xs text-muted-foreground">
        Showing {filtered.length.toLocaleString()} of {entries.length.toLocaleString()} entries · click a row for details
      </div>
    </div>
  );
}
