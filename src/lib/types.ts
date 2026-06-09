export interface HarEntrySummary {
  index: number;
  method: string;
  url: string;
  status: number;
  mime_type: string;
  size: number;
  time_ms: number;
  started_at?: string;
  is_javascript: boolean;
  resource_type?: string;
}

export interface HeaderPair {
  name: string;
  value: string;
}

export interface HarEntryDetail {
  summary: HarEntrySummary;
  request_headers: HeaderPair[];
  response_headers: HeaderPair[];
  request_body: string;
  response_body: string;
  js_insights: string[];
}

export function normalizeEntryDetail(data: HarEntryDetail | null): HarEntryDetail | null {
  if (!data) return null;

  // Handle legacy flattened payloads from older builds
  if (!data.summary && "index" in data && "method" in data) {
    const flat = data as HarEntryDetail & HarEntrySummary;
    return {
      summary: {
        index: flat.index,
        method: flat.method,
        url: flat.url,
        status: flat.status,
        mime_type: flat.mime_type,
        size: flat.size,
        time_ms: flat.time_ms,
        started_at: flat.started_at,
        is_javascript: flat.is_javascript ?? false,
        resource_type: flat.resource_type,
      },
      request_headers: flat.request_headers ?? [],
      response_headers: flat.response_headers ?? [],
      request_body: flat.request_body ?? "",
      response_body: flat.response_body ?? "",
      js_insights: flat.js_insights ?? [],
    };
  }

  return {
    ...data,
    summary: {
      ...data.summary,
      is_javascript: data.summary.is_javascript ?? false,
      resource_type: data.summary.resource_type,
    },
    request_headers: data.request_headers ?? [],
    response_headers: data.response_headers ?? [],
    request_body: data.request_body ?? "",
    response_body: data.response_body ?? "",
    js_insights: data.js_insights ?? [],
  };
}

export interface HarParseProgress {
  bytes_read: number;
  total_bytes: number;
  entries_parsed: number;
  phase: string;
}

export interface HarParseComplete {
  session_id: string;
  file_path: string;
  file_name: string;
  total_entries: number;
  total_bytes: number;
}

export interface HarChunk {
  id: string;
  session_id: string;
  chunk_index: number;
  entry_count: number;
  estimated_tokens: number;
  payload: string;
  summary?: string;
  status: "pending" | "processing" | "done" | "error";
  chunk_type: string;
}

export interface AnalysisSession {
  id: string;
  file_path: string;
  file_name: string;
  total_entries: number;
  total_bytes: number;
  created_at: string;
  status: string;
  final_summary?: string;
}

export interface AppSettings {
  openrouter_api_key: string;
  default_model: string;
  thinking_model: string;
  chunk_max_tokens: number;
  filter_static_assets: boolean;
  max_concurrent_requests: number;
  analyze_javascript: boolean;
  chat_agent_max_steps: number;
}

export interface OpenRouterModel {
  id: string;
  name: string;
}

export interface AnalysisProgress {
  session_id: string;
  phase: string;
  chunks_done: number;
  chunks_total: number;
  current_chunk?: number;
  message: string;
  synthesis_done?: number;
  synthesis_total?: number;
  synthesis_round?: number;
}

export interface LlmStreamChunk {
  session_id: string;
  chunk_index: number;
  content: string;
  done: boolean;
}

export interface ChatStreamEvent {
  session_id: string;
  content: string;
  reasoning: string;
  done: boolean;
  message_id?: number;
}

export interface ChatToolEvent {
  session_id: string;
  id: string;
  step: number;
  tool: string;
  status: "running" | "done" | "error" | "thinking" | string;
  detail: string;
}

export interface ChatAgentLimitEvent {
  session_id: string;
  steps_used: number;
  step_limit: number;
}

export interface ChatSendResult {
  message?: ChatMessage;
  needs_continue: boolean;
  steps_used: number;
  step_limit: number;
}

export interface ChatMessage {
  id: number;
  session_id: string;
  role: string;
  content: string;
  context_type?: string;
  context_ref?: string;
  created_at: string;
}

export interface ChatContext {
  context_type: string;
  entry_index?: number;
}
