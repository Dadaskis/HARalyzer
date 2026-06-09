import { create } from "zustand";
import type {
  AnalysisSession,
  AppSettings,
  HarEntryDetail,
  HarEntrySummary,
  HarParseProgress,
  AnalysisProgress,
} from "@/lib/types";
import type { ResourceFilterId } from "@/lib/entry-filters";

interface AppStore {
  sessions: AnalysisSession[];
  activeSessionId: string | null;
  entries: HarEntrySummary[];
  selectedEntry: HarEntryDetail | null;
  entryChatContext: HarEntryDetail | null;
  chatFocus: boolean;
  parseProgress: HarParseProgress | null;
  analysisProgress: AnalysisProgress | null;
  finalSummary: string | null;
  chunkSummaries: Record<number, string>;
  isParsing: boolean;
  isAnalyzing: boolean;
  settingsOpen: boolean;
  searchQuery: string;
  methodFilter: string;
  statusFilter: string;
  resourceFilter: ResourceFilterId;

  setSessions: (sessions: AnalysisSession[]) => void;
  setActiveSessionId: (id: string | null) => void;
  setEntries: (entries: HarEntrySummary[]) => void;
  setSelectedEntry: (entry: HarEntryDetail | null) => void;
  setEntryChatContext: (entry: HarEntryDetail | null) => void;
  setChatFocus: (v: boolean) => void;
  setParseProgress: (progress: HarParseProgress | null) => void;
  setAnalysisProgress: (progress: AnalysisProgress | null) => void;
  setFinalSummary: (summary: string | null) => void;
  addChunkSummary: (index: number, summary: string) => void;
  clearChunkSummaries: () => void;
  setIsParsing: (v: boolean) => void;
  setIsAnalyzing: (v: boolean) => void;
  setSettingsOpen: (v: boolean) => void;
  setSearchQuery: (q: string) => void;
  setMethodFilter: (m: string) => void;
  setStatusFilter: (s: string) => void;
  setResourceFilter: (filter: ResourceFilterId) => void;
  resetAnalysis: () => void;
}

export const useAppStore = create<AppStore>((set) => ({
  sessions: [],
  activeSessionId: null,
  entries: [],
  selectedEntry: null,
  entryChatContext: null,
  chatFocus: false,
  parseProgress: null,
  analysisProgress: null,
  finalSummary: null,
  chunkSummaries: {},
  isParsing: false,
  isAnalyzing: false,
  settingsOpen: false,
  searchQuery: "",
  methodFilter: "all",
  statusFilter: "all",
  resourceFilter: "all" as ResourceFilterId,

  setSessions: (sessions) => set({ sessions }),
  setActiveSessionId: (id) => set({ activeSessionId: id }),
  setEntries: (entries) => set({ entries }),
  setSelectedEntry: (entry) => set({ selectedEntry: entry }),
  setEntryChatContext: (entry) => set({ entryChatContext: entry, chatFocus: !!entry }),
  setChatFocus: (v) => set({ chatFocus: v }),
  setParseProgress: (progress) => set({ parseProgress: progress }),
  setAnalysisProgress: (progress) => set({ analysisProgress: progress }),
  setFinalSummary: (summary) => set({ finalSummary: summary }),
  addChunkSummary: (index, summary) =>
    set((s) => ({ chunkSummaries: { ...s.chunkSummaries, [index]: summary } })),
  clearChunkSummaries: () => set({ chunkSummaries: {} }),
  setIsParsing: (v) => set({ isParsing: v }),
  setIsAnalyzing: (v) => set({ isAnalyzing: v }),
  setSettingsOpen: (v) => set({ settingsOpen: v }),
  setSearchQuery: (q) => set({ searchQuery: q }),
  setMethodFilter: (m) => set({ methodFilter: m }),
  setStatusFilter: (s) => set({ statusFilter: s }),
  setResourceFilter: (resourceFilter) => set({ resourceFilter }),
  resetAnalysis: () =>
    set({
      finalSummary: null,
      chunkSummaries: {},
      analysisProgress: null,
      entryChatContext: null,
      chatFocus: false,
    }),
}));

export interface SettingsStore {
  settings: AppSettings;
  models: { id: string; name: string }[];
  setSettings: (s: AppSettings) => void;
  setModels: (m: { id: string; name: string }[]) => void;
}

export const useSettingsStore = create<SettingsStore>((set) => ({
  settings: {
    openrouter_api_key: "",
    default_model: "openai/gpt-4o-mini",
    thinking_model: "",
    chunk_max_tokens: 3000,
    filter_static_assets: true,
    max_concurrent_requests: 4,
    analyze_javascript: true,
    chat_agent_max_steps: 10,
  },
  models: [],
  setSettings: (settings) => set({ settings }),
  setModels: (models) => set({ models }),
}));
