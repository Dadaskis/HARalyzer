import { useCallback, useEffect, useRef, useState } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { FileUp, Settings, Loader2, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SessionSidebar } from "@/components/sidebar/SessionSidebar";
import { EntryTable } from "@/components/entries/EntryTable";
import { ResourceFilterBar } from "@/components/entries/ResourceFilterBar";
import { EntryDetailLayout } from "@/components/entries/EntryDetailPanel";
import { AnalysisPanel } from "@/components/analysis/AnalysisPanel";
import { SettingsDialog } from "@/components/settings/SettingsDialog";
import { useAppStore, useSettingsStore } from "@/store/app-store";
import { useTauriEvents } from "@/hooks/use-tauri-events";
import { useHarFileOpen } from "@/hooks/use-har-file-open";
import { api } from "@/lib/api";
import { formatBytes, normalizeMarkdownReport } from "@/lib/utils";
function App() {
  useTauriEvents();

  const {
    sessions,
    activeSessionId,
    entries,
    selectedEntry,
    entryChatContext,
    chatFocus,
    parseProgress,
    analysisProgress,
    finalSummary,
    chunkSummaries,
    isParsing,
    isAnalyzing,
    settingsOpen,
    searchQuery,
    methodFilter,
    statusFilter,
    resourceFilter,
    setSessions,
    setActiveSessionId,
    setEntries,
    setSelectedEntry,
    setEntryChatContext,
    setChatFocus,
    setFinalSummary,
    setIsParsing,
    setIsAnalyzing,
    setSettingsOpen,
    setSearchQuery,
    setMethodFilter,
    setStatusFilter,
    setResourceFilter,
    resetAnalysis,
  } = useAppStore();

  const { setSettings } = useSettingsStore();
  const parsingRef = useRef(false);
  const [isDraggingHar, setIsDraggingHar] = useState(false);

  const loadSessions = useCallback(async () => {
    const list = await api.listSessions();
    setSessions(list);
  }, [setSessions]);

  const loadSession = useCallback(
    async (sessionId: string) => {
      setActiveSessionId(sessionId);
      setSelectedEntry(null);
      resetAnalysis();
      const [session, sessionEntries, chunks] = await Promise.all([
        api.getSession(sessionId),
        api.getSessionEntries(sessionId),
        api.getSessionChunks(sessionId),
      ]);
      setEntries(sessionEntries);
      if (session?.final_summary) {
        setFinalSummary(normalizeMarkdownReport(session.final_summary));
      }
      chunks.forEach((chunk) => {
        if (chunk.summary) {
          useAppStore.getState().addChunkSummary(chunk.chunk_index, chunk.summary);
        }
      });
    },
    [setActiveSessionId, setEntries, setFinalSummary, setSelectedEntry, resetAnalysis]
  );

  useEffect(() => {
    loadSessions();
    api.getSettings().then(setSettings).catch(console.error);
  }, [loadSessions, setSettings]);

  const parseHarFromPath = useCallback(
    async (filePath: string) => {
      if (parsingRef.current) return;
      parsingRef.current = true;
      setIsParsing(true);
      resetAnalysis();
      setSelectedEntry(null);
      try {
        const result = await api.parseHarFile(filePath);
        await loadSessions();
        await loadSession(result.session_id);
      } catch (err) {
        console.error(err);
        alert(String(err));
      } finally {
        parsingRef.current = false;
        setIsParsing(false);
      }
    },
    [loadSessions, loadSession, resetAnalysis, setIsParsing, setSelectedEntry]
  );

  useHarFileOpen(parseHarFromPath, setIsDraggingHar);

  const handleOpenFile = async () => {
    const filePath = await api.openHarFile();
    if (!filePath) return;
    await parseHarFromPath(filePath);
  };

  const handleSelectEntry = async (index: number) => {
    if (!activeSessionId) return;
    try {
      const detail = await api.getEntryDetail(activeSessionId, index);
      setSelectedEntry(detail);
    } catch (err) {
      console.error(err);
    }
  };

  const handleAskAiAboutEntry = (entry: NonNullable<typeof selectedEntry>) => {
    setEntryChatContext(entry);
    setChatFocus(true);
  };

  const handleStartAnalysis = async () => {
    if (!activeSessionId) return;
    setIsAnalyzing(true);
    try {
      await api.startAnalysis(activeSessionId);
      await loadSession(activeSessionId);
      await loadSessions();
    } catch (err) {
      console.error(err);
      alert(String(err));
    } finally {
      setIsAnalyzing(false);
    }
  };

  const handleFinalizeAnalysis = async () => {
    if (!activeSessionId) return;
    setIsAnalyzing(true);
    try {
      await api.finalizeAnalysis(activeSessionId);
      await loadSession(activeSessionId);
      await loadSessions();
    } catch (err) {
      console.error(err);
      alert(String(err));
    } finally {
      setIsAnalyzing(false);
    }
  };

  const handleResetAnalysis = async () => {
    if (!activeSessionId) return;
    if (
      !window.confirm(
        "Clear all chunk summaries and the final report? Chunk data will be kept but re-analyzed on next run."
      )
    ) {
      return;
    }
    try {
      await api.resetSessionAnalysis(activeSessionId);
      resetAnalysis();
      await loadSessions();
    } catch (err) {
      console.error(err);
      alert(String(err));
    }
  };

  const handleExport = async () => {
    if (!activeSessionId) return;
    try {
      await api.saveReport(activeSessionId);
    } catch (err) {
      console.error(err);
      alert(String(err));
    }
  };

  const handleDeleteSession = async (sessionId: string) => {
    await api.deleteSession(sessionId);
    if (activeSessionId === sessionId) {
      setActiveSessionId(null);
      setEntries([]);
      setSelectedEntry(null);
      resetAnalysis();
    }
    await loadSessions();
  };

  const activeSession = sessions.find((s) => s.id === activeSessionId);
  const parsePercent =
    parseProgress && parseProgress.total_bytes > 0
      ? Math.round((parseProgress.bytes_read / parseProgress.total_bytes) * 100)
      : 0;

  return (
    <div className="relative flex h-screen flex-col overflow-hidden">
      <div className="bloom-bg" aria-hidden />

      <div className="scanline-overlay relative z-10 flex min-h-0 flex-1 flex-col">
      {isDraggingHar && (
        <div className="pointer-events-none absolute inset-0 z-50 flex items-center justify-center border-2 border-dashed border-primary/60 bg-primary/10 backdrop-blur-[1px]">
          <p className="rounded-lg bg-background/80 px-4 py-2 text-sm font-medium text-primary shadow-lg">
            Drop HAR file to open
          </p>
        </div>
      )}
      <header className="bloom-header relative z-10 flex items-center gap-3 border-b px-4 py-2">        <div className="flex items-center gap-2">
          <div className="bloom-logo flex h-8 w-8 items-center justify-center rounded-lg text-sm font-bold text-white">
            H
          </div>
          <span className="bloom-glow-text text-lg font-semibold tracking-tight">
            HARalyzer
          </span>
        </div>

        <Button size="sm" onClick={handleOpenFile} disabled={isParsing}>
          {isParsing ? (
            <>
              <Loader2 className="animate-spin" />
              Parsing...
            </>
          ) : (
            <>
              <FileUp />
              Open HAR
            </>
          )}
        </Button>

        {activeSession && (
          <div className="text-xs text-muted-foreground">
            {activeSession.file_name} · {activeSession.total_entries.toLocaleString()} entries ·{" "}
            {formatBytes(activeSession.total_bytes)}
          </div>
        )}

        <Button
          size="icon"
          variant="ghost"
          className="ml-auto"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings className="h-4 w-4" />
        </Button>
      </header>

      {isParsing && parseProgress && (
        <div className="relative z-10 space-y-1 border-b border-primary/10 px-4 py-2">
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>
              {parseProgress.phase === "scanning" ? "Scanning file..." : "Parsing entries..."}
            </span>
            <span>
              {parseProgress.entries_parsed.toLocaleString()} entries · {parsePercent}%
            </span>
          </div>
          <Progress value={parsePercent} />
        </div>
      )}

      <PanelGroup direction="horizontal" className="relative z-10 flex-1">
        <Panel defaultSize={18} minSize={12} maxSize={30}>
          <SessionSidebar
            sessions={sessions}
            activeSessionId={activeSessionId}
            onSelect={loadSession}
            onDelete={handleDeleteSession}
          />
        </Panel>

        <PanelResizeHandle className="w-1 bg-primary/20 transition-colors hover:bg-primary/50" />

        <Panel defaultSize={52} minSize={30}>
          <div className="relative flex h-full flex-col">
            <div className="flex items-center gap-2 border-b border-primary/10 px-4 py-2">
              <div className="relative flex-1">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder="Search URLs..."
                  className="h-8 border-primary/15 bg-background/50 pl-8 text-xs"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                />
              </div>
              <Select value={methodFilter} onValueChange={setMethodFilter}>
                <SelectTrigger className="h-8 w-28 text-xs">
                  <SelectValue placeholder="Method" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All methods</SelectItem>
                  <SelectItem value="GET">GET</SelectItem>
                  <SelectItem value="POST">POST</SelectItem>
                  <SelectItem value="PUT">PUT</SelectItem>
                  <SelectItem value="DELETE">DELETE</SelectItem>
                  <SelectItem value="PATCH">PATCH</SelectItem>
                </SelectContent>
              </Select>
              <Select value={statusFilter} onValueChange={setStatusFilter}>
                <SelectTrigger className="h-8 w-28 text-xs">
                  <SelectValue placeholder="Status" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All status</SelectItem>
                  <SelectItem value="success">2xx</SelectItem>
                  <SelectItem value="redirect">3xx</SelectItem>
                  <SelectItem value="error">4xx/5xx</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <ResourceFilterBar value={resourceFilter} onChange={setResourceFilter} />
            <div className="min-h-0 flex-1 overflow-hidden">
              <EntryDetailLayout
                entry={selectedEntry}
                onClose={() => setSelectedEntry(null)}
                onAskAi={handleAskAiAboutEntry}
              >
                <EntryTable
                  entries={entries}
                  searchQuery={searchQuery}
                  methodFilter={methodFilter}
                  statusFilter={statusFilter}
                  resourceFilter={resourceFilter}
                  selectedIndex={selectedEntry?.summary.index ?? null}
                  onSelectEntry={handleSelectEntry}
                />              </EntryDetailLayout>
            </div>
          </div>
        </Panel>

        <PanelResizeHandle className="w-1 bg-primary/20 transition-colors hover:bg-primary/50" />

        <Panel defaultSize={30} minSize={20} maxSize={45}>
          <AnalysisPanel
            key={`${activeSessionId ?? "none"}-${chatFocus}`}
            sessionId={activeSessionId}
            finalSummary={finalSummary}
            chunkSummaries={chunkSummaries}
            analysisProgress={analysisProgress}
            isAnalyzing={isAnalyzing}
            entryContext={entryChatContext}
            onClearEntryContext={() => {
              setEntryChatContext(null);
              setChatFocus(false);
            }}
            onStartAnalysis={handleStartAnalysis}
            onFinalizeAnalysis={handleFinalizeAnalysis}
            onResetAnalysis={handleResetAnalysis}
            onExport={handleExport}
            chatFocus={chatFocus}
          />
        </Panel>
      </PanelGroup>

      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      </div>
    </div>
  );
}

export default App;
