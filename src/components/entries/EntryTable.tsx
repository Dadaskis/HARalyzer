import { useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { HarEntrySummary } from "@/lib/types";
import { filterEntries, type ResourceFilterId } from "@/lib/entry-filters";
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
  editMode?: boolean;
  selectedIndices?: Set<number>;
  onEditRowClick?: (index: number, shiftKey: boolean) => void;
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
  editMode = false,
  selectedIndices,
  onEditRowClick,
}: EntryTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(
    () =>
      filterEntries(entries, {
        searchQuery,
        methodFilter,
        statusFilter,
        resourceFilter,
      }),
    [entries, searchQuery, methodFilter, statusFilter, resourceFilter]
  );

  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 40,
    overscan: 20,
  });

  const handleRowClick = (entry: HarEntrySummary, shiftKey: boolean) => {
    if (editMode && onEditRowClick) {
      onEditRowClick(entry.index, shiftKey);
      return;
    }
    onSelectEntry(entry.index);
  };

  const gridCols = editMode
    ? "grid-cols-[28px_60px_70px_1fr_70px_100px_80px_80px]"
    : "grid-cols-[60px_70px_1fr_70px_100px_80px_80px]";

  if (entries.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        Open a HAR file to view entries
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div
        className={cn(
          "grid shrink-0 gap-2 border-b border-primary/10 px-4 py-2 text-xs font-medium text-muted-foreground",
          gridCols
        )}
      >
        {editMode && <span />}
        <span>#</span>
        <span>Method</span>
        <span>URL</span>
        <span>Status</span>
        <span>Type</span>
        <span>Size</span>
        <span>Time</span>
      </div>
      <div ref={parentRef} className={cn("min-h-0 flex-1 overflow-auto", editMode && "select-none")}>
        <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const entry = filtered[virtualRow.index];
            const isSelected = editMode
              ? selectedIndices?.has(entry.index)
              : selectedIndex === entry.index;
            return (
              <div
                key={entry.index}
                role="button"
                tabIndex={0}
                onMouseDown={(e) => {
                  if (editMode && e.shiftKey) {
                    e.preventDefault();
                  }
                }}
                onClick={(e) => handleRowClick(entry, e.shiftKey)}
                onKeyDown={(e) => e.key === "Enter" && handleRowClick(entry, e.shiftKey)}
                className={cn(
                  "absolute left-0 top-0 grid w-full cursor-pointer gap-2 border-b border-border/30 px-4 py-2 text-xs transition-colors hover:bg-primary/10",
                  gridCols,
                  editMode && "select-none",
                  isSelected && "bg-primary/15 ring-1 ring-inset ring-primary/30"
                )}
                style={{
                  height: `${virtualRow.size}px`,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                {editMode && (
                  <input
                    type="checkbox"
                    checked={selectedIndices?.has(entry.index) ?? false}
                    readOnly
                    className="mt-0.5 h-3.5 w-3.5 accent-primary"
                    tabIndex={-1}
                  />
                )}
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
        Showing {filtered.length.toLocaleString()} of {entries.length.toLocaleString()} entries
        {editMode
          ? selectedIndices && selectedIndices.size > 0
            ? ` · ${selectedIndices.size} selected`
            : " · click rows to select · Shift+click for range"
          : " · click a row for details"}
      </div>
    </div>
  );
}

export { filterEntries };
