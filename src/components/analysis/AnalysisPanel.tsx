import { useEffect, useMemo, useRef, useState } from "react";
import { Sparkles, Download, Loader2, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MarkdownContent } from "@/components/markdown/MarkdownContent";
import { ChatPanel } from "@/components/analysis/ChatPanel";
import { ChunkListView } from "@/components/analysis/ChunkListView";
import type { AnalysisProgress, HarEntryDetail } from "@/lib/types";

interface AnalysisPanelProps {
  sessionId: string | null;
  finalSummary: string | null;
  chunkSummaries: Record<number, string>;
  analysisProgress: AnalysisProgress | null;
  isAnalyzing: boolean;
  entryContext: HarEntryDetail | null;
  onClearEntryContext: () => void;
  onStartAnalysis: () => void;
  onFinalizeAnalysis: () => void;
  onResetAnalysis: () => void;
  onExport: () => void;
  chatFocus?: boolean;
}

function formatEta(seconds: number): string | null {
  if (!Number.isFinite(seconds) || seconds <= 0) return null;
  if (seconds < 60) return `~${Math.ceil(seconds)}s remaining`;
  const minutes = Math.floor(seconds / 60);
  const secs = Math.ceil(seconds % 60);
  return `~${minutes}m ${secs}s remaining`;
}

export function AnalysisPanel({
  sessionId,
  finalSummary,
  chunkSummaries,
  analysisProgress,
  isAnalyzing,
  entryContext,
  onClearEntryContext,
  onStartAnalysis,
  onFinalizeAnalysis,
  onResetAnalysis,
  onExport,
  chatFocus,
}: AnalysisPanelProps) {
  const synthesisStartRef = useRef<number | null>(null);
  const [etaTick, setEtaTick] = useState(0);

  useEffect(() => {
    if (!isAnalyzing || analysisProgress?.phase !== "final") {
      synthesisStartRef.current = null;
      return;
    }

    if (synthesisStartRef.current === null) {
      synthesisStartRef.current = Date.now();
    }

    const timer = window.setInterval(() => setEtaTick((t) => t + 1), 1000);
    return () => window.clearInterval(timer);
  }, [isAnalyzing, analysisProgress?.phase]);

  const isFinalPhase = analysisProgress?.phase === "final";

  const progressPercent = useMemo(() => {
    if (!analysisProgress) return 0;
    if (isFinalPhase && analysisProgress.synthesis_total) {
      const done = analysisProgress.synthesis_done ?? 0;
      return Math.round((done / analysisProgress.synthesis_total) * 100);
    }
    if (analysisProgress.chunks_total > 0) {
      return Math.round(
        (analysisProgress.chunks_done / analysisProgress.chunks_total) * 100
      );
    }
    return 0;
  }, [analysisProgress, isFinalPhase]);

  const synthesisEta = useMemo(() => {
    if (!isFinalPhase || !analysisProgress?.synthesis_total) return null;
    const done = analysisProgress.synthesis_done ?? 0;
    const total = analysisProgress.synthesis_total;
    const started = synthesisStartRef.current;
    if (done <= 0 || !started) return null;
    const elapsedSec = (Date.now() - started) / 1000;
    const secPerStep = elapsedSec / done;
    return formatEta((total - done) * secPerStep);
  }, [analysisProgress, isFinalPhase, etaTick]);

  const progressLabel = useMemo(() => {
    if (!analysisProgress) return "";
    if (isFinalPhase && analysisProgress.synthesis_total) {
      return `${analysisProgress.synthesis_done ?? 0}/${analysisProgress.synthesis_total} steps`;
    }
    return `${analysisProgress.chunks_done}/${analysisProgress.chunks_total} chunks`;
  }, [analysisProgress, isFinalPhase]);

  const chunkIndices = useMemo(
    () =>
      Object.keys(chunkSummaries)
        .map(Number)
        .sort((a, b) => a - b),
    [chunkSummaries]
  );

  const [activeTab, setActiveTab] = useState(chatFocus ? "chat" : "summary");

  useEffect(() => {
    if (chatFocus) setActiveTab("chat");
  }, [chatFocus]);

  return (
    <div className="flex h-full min-w-0 flex-col border-l border-primary/25 bg-card/40 bloom-panel">
      <div className="flex items-center gap-2 border-b border-primary/10 px-4 py-3">
        <Sparkles className="h-4 w-4 text-primary" />
        <h2 className="text-sm font-semibold">LLM Analysis</h2>
        <div className="ml-auto flex gap-2">
          <Button size="sm" onClick={onStartAnalysis} disabled={!sessionId || isAnalyzing}>
            {isAnalyzing ? (
              <>
                <Loader2 className="animate-spin" />
                Analyzing...
              </>
            ) : (
              <>
                <Sparkles />
                Analyze
              </>
            )}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={onFinalizeAnalysis}
            disabled={!sessionId || isAnalyzing || chunkIndices.length === 0}
            title="Generate final report from existing chunk summaries"
          >
            {isAnalyzing ? <Loader2 className="animate-spin" /> : <Sparkles />}
            Report
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={onResetAnalysis}
            disabled={!sessionId || isAnalyzing || chunkIndices.length === 0}
            title="Clear all analysis results and start over"
          >
            <RotateCcw />
            Reset
          </Button>
          <Button size="sm" variant="outline" onClick={onExport} disabled={!sessionId || !finalSummary}>
            <Download />
            Export
          </Button>
        </div>
      </div>

      {isAnalyzing && analysisProgress && (
        <div className="space-y-2 border-b border-primary/10 px-4 py-3">
          <div className="flex flex-col gap-1 text-xs text-muted-foreground">
            <div className="flex justify-between gap-3">
              <span className="min-w-0 flex-1 leading-snug">{analysisProgress.message}</span>
              <span className="shrink-0 tabular-nums">{progressLabel}</span>
            </div>
            {synthesisEta && (
              <span className="text-primary/90 tabular-nums">{synthesisEta}</span>
            )}
          </div>
          <Progress value={progressPercent} />
        </div>
      )}

      <Tabs
        value={activeTab}
        onValueChange={setActiveTab}
        className="flex flex-1 flex-col overflow-hidden"
      >
        <TabsList className="mx-4 mt-2 w-auto justify-start">
          <TabsTrigger value="summary">Summary</TabsTrigger>
          <TabsTrigger value="chunks">
            Chunks {chunkIndices.length > 0 && `(${chunkIndices.length})`}
          </TabsTrigger>
          <TabsTrigger value="chat">Chat</TabsTrigger>
        </TabsList>

        <TabsContent value="summary" className="mt-0 min-w-0 flex-1 overflow-hidden">
          {activeTab === "summary" ? (
            <ScrollArea className="h-full min-w-0 px-4 pb-4">
              {!sessionId ? (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  Select or import a HAR file to begin analysis
                </p>
              ) : finalSummary ? (
                <Card className="mt-2 min-w-0 border-primary/10 bg-card/60">
                  <CardHeader className="pb-2">
                    <CardTitle className="text-sm">Final Report</CardTitle>
                  </CardHeader>
                  <CardContent className="min-w-0 overflow-hidden">
                    <MarkdownContent content={finalSummary} />
                  </CardContent>
                </Card>
              ) : chunkIndices.length > 0 ? (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  {chunkIndices.length} chunk
                  {chunkIndices.length === 1 ? "" : "s"} analyzed — click Report to generate the
                  final summary
                </p>
              ) : (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  Click Analyze to run parallel chunked LLM analysis via OpenRouter
                </p>
              )}
            </ScrollArea>
          ) : null}
        </TabsContent>

        <TabsContent value="chunks" className="mt-0 min-w-0 flex-1 overflow-hidden">
          {activeTab === "chunks" ? (
            <ChunkListView chunkIndices={chunkIndices} chunkSummaries={chunkSummaries} />
          ) : null}
        </TabsContent>

        <TabsContent value="chat" className="mt-0 min-w-0 flex-1 overflow-hidden">
          {activeTab === "chat" ? (
            <ChatPanel
              sessionId={sessionId}
              entryContext={entryContext}
              onClearEntryContext={onClearEntryContext}
            />
          ) : null}
        </TabsContent>
      </Tabs>
    </div>
  );
}
