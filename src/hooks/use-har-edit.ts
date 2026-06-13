import { useCallback, useRef, useState } from "react";
import { api } from "@/lib/api";
import type { HarEntryDetail, HarEntrySummary } from "@/lib/types";

const MAX_UNDO = 50;

export function useHarEdit(sessionId: string | null) {
  const [selectedIndices, setSelectedIndices] = useState<Set<number>>(new Set());
  const undoStackRef = useRef<HarEntryDetail[][]>([]);
  const redoStackRef = useRef<HarEntryDetail[][]>([]);
  const [historyTick, setHistoryTick] = useState(0);

  const bumpHistory = useCallback(() => setHistoryTick((t) => t + 1), []);

  const clearSelection = useCallback(() => setSelectedIndices(new Set()), []);

  const pushUndoSnapshot = useCallback(async () => {
    if (!sessionId) return;
    const snapshot = await api.getSessionEntriesSnapshot(sessionId);
    undoStackRef.current = [...undoStackRef.current.slice(-(MAX_UNDO - 1)), snapshot];
    redoStackRef.current = [];
    bumpHistory();
  }, [sessionId, bumpHistory]);

  const deleteIndices = useCallback(
    async (indices: number[]): Promise<HarEntrySummary[] | null> => {
      if (!sessionId || indices.length === 0) return null;
      await pushUndoSnapshot();
      const updated = await api.deleteSessionEntries(sessionId, indices);
      clearSelection();
      return updated;
    },
    [sessionId, pushUndoSnapshot, clearSelection]
  );

  const deleteSelected = useCallback(async () => {
    return deleteIndices([...selectedIndices]);
  }, [deleteIndices, selectedIndices]);

  const deleteUnselected = useCallback(
    async (allEntries: HarEntrySummary[]) => {
      const toDelete = allEntries
        .filter((e) => !selectedIndices.has(e.index))
        .map((e) => e.index);
      return deleteIndices(toDelete);
    },
    [deleteIndices, selectedIndices]
  );

  const undo = useCallback(async (): Promise<HarEntrySummary[] | null> => {
    if (!sessionId || undoStackRef.current.length === 0) return null;
    const current = await api.getSessionEntriesSnapshot(sessionId);
    const previous = undoStackRef.current.pop()!;
    redoStackRef.current.push(current);
    bumpHistory();
    clearSelection();
    return api.restoreSessionEntries(sessionId, previous);
  }, [sessionId, bumpHistory, clearSelection]);

  const redo = useCallback(async (): Promise<HarEntrySummary[] | null> => {
    if (!sessionId || redoStackRef.current.length === 0) return null;
    const current = await api.getSessionEntriesSnapshot(sessionId);
    const next = redoStackRef.current.pop()!;
    undoStackRef.current.push(current);
    bumpHistory();
    clearSelection();
    return api.restoreSessionEntries(sessionId, next);
  }, [sessionId, bumpHistory, clearSelection]);

  const toggleSelection = useCallback((index: number) => {
    setSelectedIndices((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }, []);

  const selectRange = useCallback((indices: number[]) => {
    setSelectedIndices((prev) => {
      const next = new Set(prev);
      for (const i of indices) next.add(i);
      return next;
    });
  }, []);

  const setAllSelected = useCallback((indices: number[], selected: boolean) => {
    if (selected) {
      setSelectedIndices(new Set(indices));
    } else {
      setSelectedIndices(new Set());
    }
  }, []);

  const resetEditState = useCallback(() => {
    clearSelection();
    undoStackRef.current = [];
    redoStackRef.current = [];
    bumpHistory();
  }, [clearSelection, bumpHistory]);

  return {
    selectedIndices,
    canUndo: historyTick >= 0 && undoStackRef.current.length > 0,
    canRedo: historyTick >= 0 && redoStackRef.current.length > 0,
    toggleSelection,
    selectRange,
    setAllSelected,
    clearSelection,
    deleteSelected,
    deleteUnselected,
    undo,
    redo,
    resetEditState,
  };
}
