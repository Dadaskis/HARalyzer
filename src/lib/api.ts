import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisSession,
  AppSettings,
  ChatContext,
  ChatMessage,
  ChatSendResult,
  HarChunk,
  HarEntryDetail,
  HarEntrySummary,
  HarParseComplete,
  OpenRouterModel,
} from "./types";
import { normalizeEntryDetail } from "./types";

export const CHAT_CANCELLED_MESSAGE = "Chat cancelled by user";

export const api = {
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<void>("save_settings", { settings }),
  listSessions: () => invoke<AnalysisSession[]>("list_sessions"),
  getSession: (sessionId: string) =>
    invoke<AnalysisSession | null>("get_session", { sessionId }),
  getSessionEntries: (sessionId: string) =>
    invoke<HarEntrySummary[]>("get_session_entries", { sessionId }),
  getEntryDetail: async (sessionId: string, entryIndex: number) =>
    normalizeEntryDetail(
      await invoke<HarEntryDetail | null>("get_entry_detail", { sessionId, entryIndex })
    ),
  getSessionChunks: (sessionId: string) =>
    invoke<HarChunk[]>("get_session_chunks", { sessionId }),
  getChatMessages: (sessionId: string) =>
    invoke<ChatMessage[]>("get_chat_messages", { sessionId }),
  clearChatMessages: (sessionId: string) =>
    invoke<void>("clear_chat_messages", { sessionId }),
  sendChatMessage: (
    sessionId: string,
    message: string,
    context?: ChatContext,
    thinkingMode?: boolean
  ) =>
    invoke<ChatSendResult>("send_chat_message", {
      sessionId,
      message,
      context,
      thinkingMode: thinkingMode ?? false,
    }),
  continueChatAgent: (sessionId: string, thinkingMode?: boolean) =>
    invoke<ChatSendResult>("continue_chat_agent", {
      sessionId,
      thinkingMode: thinkingMode ?? false,
    }),
  finalizeChatAgent: (sessionId: string, thinkingMode?: boolean) =>
    invoke<ChatSendResult>("finalize_chat_agent", {
      sessionId,
      thinkingMode: thinkingMode ?? false,
    }),
  cancelChatAgent: (sessionId: string) =>
    invoke<void>("cancel_chat_agent", { sessionId }),
  openHarFile: () => invoke<string | null>("open_har_file"),
  takePendingHarFiles: () => invoke<string[]>("take_pending_har_files"),
  ackPendingHarFiles: (paths: string[]) =>
    invoke<void>("ack_pending_har_files", { paths }),
  notifyFrontendReady: () => invoke<void>("notify_frontend_ready"),
  parseHarFile: (filePath: string) =>
    invoke<HarParseComplete>("parse_har_file", { filePath }),
  buildChunks: (sessionId: string) => invoke<HarChunk[]>("build_chunks", { sessionId }),
  startAnalysis: (sessionId: string) => invoke<string>("start_analysis", { sessionId }),
  finalizeAnalysis: (sessionId: string) => invoke<string>("finalize_analysis", { sessionId }),
  resetSessionAnalysis: (sessionId: string) =>
    invoke<void>("reset_session_analysis", { sessionId }),
  exportReport: (sessionId: string) => invoke<string>("export_report", { sessionId }),
  saveReport: (sessionId: string) =>
    invoke<string | null>("save_report", { sessionId }),
  deleteSession: (sessionId: string) => invoke<void>("delete_session", { sessionId }),
  listOpenRouterModels: () => invoke<OpenRouterModel[]>("list_openrouter_models"),
};
