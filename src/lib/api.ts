import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisSession,
  AgentLimitFieldDoc,
  AppSettings,
  ChatContext,
  ChatMessage,
  ChatSendResult,
  HarChunk,
  HarEntryDetail,
  HarEntrySummary,
  HarParseComplete,
  OpenRouterModel,
  LoadScriptResult,
  ToolStep,
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
  getChatMessages: (sessionId: string, limit?: number, offset?: number) =>
    invoke<ChatMessage[]>("get_chat_messages", { sessionId, limit, offset }),
  getToolSteps: (sessionId: string, limit?: number) =>
    invoke<ToolStep[]>("get_tool_steps", { sessionId, limit }),
  loadAgentScript: (sessionId: string) =>
    invoke<LoadScriptResult>("load_agent_script", { sessionId }),
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
  cancelChatAgent: (sessionId: string, mode?: "abort" | "keep" | "finalize") =>
    invoke<void>("cancel_chat_agent", { sessionId, mode: mode ?? "abort" }),
  getEntryBodyFull: (sessionId: string, entryIndex: number) =>
    invoke<{ request_body: string; response_body: string }>("get_entry_body_full", {
      sessionId,
      entryIndex,
    }),
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
  deleteSessionEntries: (sessionId: string, entryIndices: number[]) =>
    invoke<HarEntrySummary[]>("delete_session_entries", { sessionId, entryIndices }),
  restoreSessionEntries: (sessionId: string, entries: HarEntryDetail[]) =>
    invoke<HarEntrySummary[]>("restore_session_entries", { sessionId, entries }),
  getSessionEntriesSnapshot: (sessionId: string) =>
    invoke<HarEntryDetail[]>("get_session_entries_snapshot", { sessionId }),
  saveHarFile: (sessionId: string) =>
    invoke<string | null>("save_har_file", { sessionId }),
  listOpenRouterModels: () => invoke<OpenRouterModel[]>("list_openrouter_models"),
  getAgentLimitDocs: () => invoke<AgentLimitFieldDoc[]>("get_agent_limit_docs"),
  deobfuscateJs: (sessionId: string, entryIndex: number, force?: boolean) =>
    invoke<void>("deobfuscate_js_entry", { sessionId, entryIndex, force: force ?? false }),
  appendHarFile: (targetSessionId: string) =>
    invoke<{ appended_count: number; total_entries: number }>("append_har_file", { targetSessionId }),
  listSessionIds: () =>
    invoke<[string, string][]>("list_session_ids"),
};
