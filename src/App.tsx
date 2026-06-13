import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Panel,
  PanelGroup,
  PanelResizeHandle,
  type ImperativePanelHandle,
} from "react-resizable-panels";
import { ChevronLeft, FileUp, Settings, Loader2, Search, Sparkles, X, FilePlus } from "lucide-react";
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
import { EntryEditToolbar } from "@/components/entries/EntryEditToolbar";
import { ResourceFilterBar } from "@/components/entries/ResourceFilterBar";
import { EntryDetailLayout } from "@/components/entries/EntryDetailPanel";
import { AnalysisPanel } from "@/components/analysis/AnalysisPanel";
import { SettingsDialog } from "@/components/settings/SettingsDialog";
import { useAppStore, useSettingsStore } from "@/store/app-store";
import { useTauriEvents } from "@/hooks/use-tauri-events";
import { useHarFileOpen } from "@/hooks/use-har-file-open";
import { useHarEdit } from "@/hooks/use-har-edit";
import { api } from "@/lib/api";
import { filterEntries } from "@/lib/entry-filters";
import { formatBytes, normalizeMarkdownReport, cn } from "@/lib/utils";
function App() {
  useTauriEvents();

  const analysisPanelRef = useRef<ImperativePanelHandle>(null);
  const [analysisCollapsed, setAnalysisCollapsed] = useState(() => {
    try {
      return localStorage.getItem("haralyzer-analysis-collapsed") === "true";
    } catch {
      return false;
    }
  });

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
    sessionChunks,
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
    setSessionChunks,
    resetAnalysis,
  } = useAppStore();

  const harEdit = useHarEdit(activeSessionId);
  const lastEditClickRef = useRef<number | null>(null);
  const [editMode, setEditMode] = useState(false);

  const { setSettings } = useSettingsStore();
  const parsingRef = useRef(false);
  const pendingOpenRef = useRef<string[]>([]);
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
      setSessionChunks(chunks);
      if (session?.final_summary) {
        setFinalSummary(normalizeMarkdownReport(session.final_summary));
      }
      chunks.forEach((chunk) => {
        if (chunk.summary) {
          useAppStore.getState().addChunkSummary(chunk.chunk_index, chunk.summary);
        }
      });
    },
    [setActiveSessionId, setEntries, setFinalSummary, setSelectedEntry, resetAnalysis, setSessionChunks]
  );

  const applyEntryUpdate = useCallback(
    async (updated: Awaited<ReturnType<typeof harEdit.deleteSelected>>) => {
      if (!updated) return;
      setEntries(updated);
      setSelectedEntry(null);
      if (activeSessionId) {
        await loadSessions();
      }
    },
    [activeSessionId, setEntries, setSelectedEntry, loadSessions]
  );

  const visibleEntries = useMemo(
    () =>
      filterEntries(entries, {
        searchQuery,
        methodFilter,
        statusFilter,
        resourceFilter,
      }),
    [entries, searchQuery, methodFilter, statusFilter, resourceFilter]
  );

  const allVisibleSelected =
    visibleEntries.length > 0 &&
    visibleEntries.every((e) => harEdit.selectedIndices.has(e.index));

  useEffect(() => {
    harEdit.resetEditState();
    lastEditClickRef.current = null;
    setEditMode(false);
  }, [activeSessionId]);

  useEffect(() => {
    if (!editMode) {
      harEdit.resetEditState();
      lastEditClickRef.current = null;
    }
  }, [editMode]);

  useEffect(() => {
    if (!editMode) return;

    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable) {
        return;
      }

      if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        void harEdit.deleteSelected().then(applyEntryUpdate);
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        void harEdit.undo().then(applyEntryUpdate);
        return;
      }

      if ((e.ctrlKey || e.metaKey) && (e.key === "y" || (e.key === "z" && e.shiftKey))) {
        e.preventDefault();
        void harEdit.redo().then(applyEntryUpdate);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [editMode, harEdit, applyEntryUpdate]);

  const handleEditRowClick = useCallback(
    (index: number, shiftKey: boolean) => {
      if (shiftKey && lastEditClickRef.current !== null) {
        const startIdx = visibleEntries.findIndex((e) => e.index === lastEditClickRef.current);
        const endIdx = visibleEntries.findIndex((e) => e.index === index);
        if (startIdx !== -1 && endIdx !== -1) {
          const [lo, hi] = startIdx < endIdx ? [startIdx, endIdx] : [endIdx, startIdx];
          harEdit.selectRange(visibleEntries.slice(lo, hi + 1).map((e) => e.index));
        }
      } else {
        harEdit.toggleSelection(index);
      }
      lastEditClickRef.current = index;
    },
    [visibleEntries, harEdit]
  );

  const handleToggleSelectAllVisible = useCallback(() => {
    harEdit.setAllSelected(
      visibleEntries.map((e) => e.index),
      !allVisibleSelected
    );
  }, [visibleEntries, allVisibleSelected, harEdit]);

  const handleDeleteUnselected = useCallback(async () => {
    const count = entries.length - harEdit.selectedIndices.size;
    if (count <= 0) return;
    if (
      !window.confirm(
        `Delete ${count.toLocaleString()} unselected ${count === 1 ? "entry" : "entries"}? This cannot be undone except via Undo.`
      )
    ) {
      return;
    }
    await applyEntryUpdate(await harEdit.deleteUnselected(entries));
  }, [entries, harEdit, applyEntryUpdate]);

  const handleExpandAnalysis = useCallback(() => {
    analysisPanelRef.current?.expand();
  }, []);

  const handleMinimizeAnalysis = useCallback(() => {
    analysisPanelRef.current?.collapse();
  }, []);

  const handleSaveHar = useCallback(async () => {
    if (!activeSessionId) return;
    try {
      const path = await api.saveHarFile(activeSessionId);
      if (path) alert(`HAR saved to:\n${path}`);
    } catch (err) {
      console.error(err);
      alert(String(err));
    }
  }, [activeSessionId]);

  useEffect(() => {
    if (chatFocus && analysisCollapsed) {
      analysisPanelRef.current?.expand();
    }
  }, [chatFocus, analysisCollapsed]);

  useEffect(() => {
    try {
      localStorage.setItem("haralyzer-analysis-collapsed", String(analysisCollapsed));
    } catch {}
  }, [analysisCollapsed]);

  useEffect(() => {
    loadSessions();
    api.getSettings().then(setSettings).catch(console.error);
  }, [loadSessions, setSettings]);

  const parseHarFromPath = useCallback(
    async (filePath: string) => {
      if (parsingRef.current) {
        if (!pendingOpenRef.current.includes(filePath)) {
          pendingOpenRef.current.push(filePath);
        }
        return;
      }
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
        const next = pendingOpenRef.current.shift();
        if (next) {
          void parseHarFromPath(next);
        }
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

  const handleAppendHar = async () => {
    if (!activeSessionId) return;
    try {
      const result = await api.appendHarFile(activeSessionId);
      alert(`Appended ${result.appended_count} entries. Total: ${result.total_entries}`);
      await loadSession(activeSessionId);
      await loadSessions();
    } catch (err) {
      console.error(err);
      alert(String(err));
    }
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

  const handleDeobfuscatedJs = async (entryIndex: number) => {
    if (!activeSessionId) return;
    try {
      const detail = await api.getEntryDetail(activeSessionId, entryIndex);
      setSelectedEntry(detail);
    } catch (err) {
      console.error(err);
    }
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
      <header className="bloom-header relative z-10 flex flex-wrap items-center gap-3 border-b px-4 py-2">
        <div className="flex items-center gap-2 bloom-brand">
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
          <Button size="sm" variant="outline" onClick={handleAppendHar} disabled={isParsing}>
            <FilePlus />
            Append HAR
          </Button>
        )}

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
            <div className="flex flex-wrap items-center gap-2 border-b border-primary/10 px-4 py-2">
              <div className="relative min-w-[140px] flex-1 basis-[200px]">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder="Search URLs..."
                  className="h-8 border-primary/15 bg-background/50 pl-8 pr-8 text-xs"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                />
                {searchQuery && (
                  <button
                    type="button"
                    className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Clear search"
                    onClick={() => setSearchQuery("")}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
              <Select value={methodFilter} onValueChange={setMethodFilter}>
                <SelectTrigger className="h-8 w-28 shrink-0 text-xs">
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
                <SelectTrigger className="h-8 w-28 shrink-0 text-xs">
                  <SelectValue placeholder="Status" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All status</SelectItem>
                  <SelectItem value="success">2xx</SelectItem>
                  <SelectItem value="redirect">3xx</SelectItem>
                  <SelectItem value="error">4xx/5xx</SelectItem>
                </SelectContent>
              </Select>
              <label className="flex shrink-0 cursor-pointer items-center gap-1.5 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={editMode}
                  onChange={(e) => setEditMode(e.target.checked)}
                  className="rounded border-input"
                />
                Edit mode
              </label>
            </div>
            <ResourceFilterBar
              value={resourceFilter}
              onChange={setResourceFilter}
              editMode={editMode}
              allVisibleSelected={allVisibleSelected}
              onToggleSelectAllVisible={handleToggleSelectAllVisible}
            />
            {editMode && (
              <EntryEditToolbar
                selectedCount={harEdit.selectedIndices.size}
                canUndo={harEdit.canUndo}
                canRedo={harEdit.canRedo}
                onUndo={() => void harEdit.undo().then(applyEntryUpdate)}
                onRedo={() => void harEdit.redo().then(applyEntryUpdate)}
                onDeleteSelected={() => void harEdit.deleteSelected().then(applyEntryUpdate)}
                onDeleteUnselected={handleDeleteUnselected}
                onSaveHar={handleSaveHar}
              />
            )}
            <div className="min-h-0 flex-1 overflow-hidden">
              <EntryDetailLayout
                sessionId={activeSessionId ?? ""}
                entry={editMode ? null : selectedEntry}
                onClose={() => setSelectedEntry(null)}
                onAskAi={handleAskAiAboutEntry}
                onDeobfuscated={handleDeobfuscatedJs}
              >
                <EntryTable
                  entries={entries}
                  searchQuery={searchQuery}
                  methodFilter={methodFilter}
                  statusFilter={statusFilter}
                  resourceFilter={resourceFilter}
                  selectedIndex={editMode ? null : selectedEntry?.summary.index ?? null}
                  onSelectEntry={handleSelectEntry}
                  editMode={editMode}
                  selectedIndices={harEdit.selectedIndices}
                  onEditRowClick={handleEditRowClick}
                />
              </EntryDetailLayout>
            </div>
          </div>
        </Panel>

        {!analysisCollapsed && (
          <PanelResizeHandle className="w-1 bg-primary/20 transition-colors hover:bg-primary/50" />
        )}

        <Panel
          ref={analysisPanelRef}
          collapsible
          collapsedSize={0}
          minSize={analysisCollapsed ? 0 : 20}
          defaultSize={30}
          maxSize={45}
          onCollapse={() => setAnalysisCollapsed(true)}
          onExpand={() => setAnalysisCollapsed(false)}
        >
          <AnalysisPanel
            key={`${activeSessionId ?? "none"}-${chatFocus}`}
            sessionId={activeSessionId}
            finalSummary={finalSummary}
            chunkSummaries={chunkSummaries}
            sessionChunks={sessionChunks}
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
            onMinimize={handleMinimizeAnalysis}
            chatFocus={chatFocus}
          />
        </Panel>

        {analysisCollapsed && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className={cn(
              "absolute right-0 top-1/2 z-20 h-auto -translate-y-1/2 rounded-l-md rounded-r-none",
              "border border-r-0 border-primary/25 bg-card/95 px-2 py-3 shadow-lg backdrop-blur-sm",
              "flex flex-col items-center gap-1.5 text-[10px] font-medium text-primary"
            )}
            onClick={handleExpandAnalysis}
            title="Show LLM Analysis"
          >
            <ChevronLeft className="h-4 w-4" />
            <Sparkles className="h-4 w-4" />
            <span className="[writing-mode:vertical-rl] rotate-180">LLM</span>
          </Button>
        )}
      </PanelGroup>

      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      </div>
    </div>
  );
}

export default App;
