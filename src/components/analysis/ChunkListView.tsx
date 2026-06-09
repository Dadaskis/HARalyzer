import { memo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MarkdownContent } from "@/components/markdown/MarkdownContent";

interface ChunkListViewProps {
  chunkIndices: number[];
  chunkSummaries: Record<number, string>;
}

const ChunkCard = memo(function ChunkCard({
  index,
  content,
}: {
  index: number;
  content: string;
}) {
  return (
    <Card className="min-w-0 border-primary/10 bg-card/60">
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Chunk {index + 1}</CardTitle>
      </CardHeader>
      <CardContent className="min-w-0 overflow-hidden">
        <MarkdownContent content={content} className="text-xs" />
      </CardContent>
    </Card>
  );
});

export function ChunkListView({ chunkIndices, chunkSummaries }: ChunkListViewProps) {
  const parentRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: chunkIndices.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 280,
    overscan: 2,
  });

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
              key={idx}
              className="absolute left-0 top-0 w-full pb-3"
              style={{
                height: `${virtualRow.size}px`,
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              <ChunkCard index={idx} content={chunkSummaries[idx]} />
            </div>
          );
        })}
      </div>
    </div>
  );
}
