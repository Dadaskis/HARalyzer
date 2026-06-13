import { memo, useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MarkdownContent } from "@/components/markdown/MarkdownContent";
import type { HarChunk } from "@/lib/types";

interface ChunkListViewProps {
  chunkIndices: number[];
  chunkSummaries: Record<number, string>;
  sessionChunks: HarChunk[];
}

function extractEntryIndices(payload: string): number[] {
  const indices: number[] = [];
  for (const line of payload.split("\n")) {
    const trimmed = line.trimStart();
    if (!trimmed.startsWith("[")) continue;
    const end = trimmed.indexOf("]");
    if (end <= 1) continue;
    const num = Number.parseInt(trimmed.slice(1, end), 10);
    if (!Number.isNaN(num)) indices.push(num);
  }
  return indices;
}

function formatCoverage(indices: number[]): string {
  if (indices.length === 0) return "No entry indices parsed from payload";
  if (indices.length <= 6) return indices.join(", ");
  return `${indices[0]}–${indices[indices.length - 1]} (${indices.length} entries)`;
}

const ChunkCard = memo(function ChunkCard({
  index,
  content,
  chunk,
}: {
  index: number;
  content: string;
  chunk?: HarChunk;
}) {
  const entryIndices = useMemo(
    () => (chunk ? extractEntryIndices(chunk.payload) : []),
    [chunk]
  );
  const payloadPreview = useMemo(() => {
    if (!chunk?.payload) return null;
    return chunk.payload.split("\n").slice(0, 2).join(" · ");
  }, [chunk]);

  return (
    <Card className="min-w-0 border-primary/10 bg-card/60">
      <CardHeader className="space-y-2 pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <CardTitle className="text-xs">Chunk {index + 1}</CardTitle>
          {chunk && (
            <>
              <Badge variant="outline" className="text-[10px]">
                {chunk.chunk_type}
              </Badge>
              <Badge variant="secondary" className="text-[10px]">
                {chunk.entry_count} entries
              </Badge>
              <Badge variant="secondary" className="text-[10px]">
                {chunk.status}
              </Badge>
            </>
          )}
        </div>
        {chunk && (
          <div className="space-y-1 text-[10px] text-muted-foreground">
            <p>
              <span className="font-medium text-foreground/80">Analyzed entries:</span>{" "}
              {formatCoverage(entryIndices)}
            </p>
            {payloadPreview && (
              <p className="break-words font-mono leading-snug [overflow-wrap:anywhere]">
                {payloadPreview}
              </p>
            )}
          </div>
        )}
      </CardHeader>
      <CardContent className="min-w-0 overflow-hidden">
        <MarkdownContent content={content} className="text-xs" />
      </CardContent>
    </Card>
  );
});

export function ChunkListView({
  chunkIndices,
  chunkSummaries,
  sessionChunks,
}: ChunkListViewProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const chunkByIndex = useMemo(() => {
    const map = new Map<number, HarChunk>();
    for (const chunk of sessionChunks) {
      map.set(chunk.chunk_index, chunk);
    }
    return map;
  }, [sessionChunks]);

  const virtualizer = useVirtualizer({
    count: chunkIndices.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 280,
    overscan: 2,
    getItemKey: (index) => chunkIndices[index],
  });

  useEffect(() => {
    virtualizer.measure();
  }, [chunkIndices, chunkSummaries, sessionChunks, virtualizer]);

  if (chunkIndices.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        Chunk summaries will appear here during analysis
      </p>
    );
  }

  return (
    <div ref={parentRef} className="h-full min-w-0 overflow-auto px-4 pb-4 pt-2">
      <div
        className="relative min-w-0 max-w-full"
        style={{ height: `${virtualizer.getTotalSize()}px` }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const idx = chunkIndices[virtualRow.index];
          return (
            <div
              key={virtualRow.key}
              data-index={virtualRow.index}
              ref={virtualizer.measureElement}
              className="absolute left-0 top-0 w-full pb-3"
              style={{
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              <ChunkCard
                index={idx}
                content={chunkSummaries[idx]}
                chunk={chunkByIndex.get(idx)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
