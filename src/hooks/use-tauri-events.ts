import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useAppStore } from "@/store/app-store";
import { normalizeMarkdownReport } from "@/lib/utils";
import type { HarParseProgress, AnalysisProgress, LlmStreamChunk } from "@/lib/types";

export function useTauriEvents() {
  const {
    setParseProgress,
    setAnalysisProgress,
    addChunkSummary,
    setFinalSummary,
    setIsParsing,
    setIsAnalyzing,
  } = useAppStore();

  useEffect(() => {
    const unsubs: (() => void)[] = [];

    listen<HarParseProgress>("har-parse-progress", (event) => {
      setParseProgress(event.payload);
    }).then((u) => unsubs.push(u));

    listen("har-parse-complete", () => {
      setIsParsing(false);
      setParseProgress(null);
    }).then((u) => unsubs.push(u));

    listen<AnalysisProgress>("analysis-progress", (event) => {
      setAnalysisProgress(event.payload);
      if (event.payload.phase === "complete") {
        setIsAnalyzing(false);
      }
    }).then((u) => unsubs.push(u));

    listen<LlmStreamChunk>("llm-stream", (event) => {
      const { chunk_index, content } = event.payload;
      if (chunk_index === -1) {
        setFinalSummary(normalizeMarkdownReport(content));
      } else {
        addChunkSummary(chunk_index, content);
      }
    }).then((u) => unsubs.push(u));

    return () => unsubs.forEach((u) => u());
  }, [
    setParseProgress,
    setAnalysisProgress,
    addChunkSummary,
    setFinalSummary,
    setIsParsing,
    setIsAnalyzing,
  ]);
}
