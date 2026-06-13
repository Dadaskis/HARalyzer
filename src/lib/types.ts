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
  deobfuscated_js?: string | null;
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
      deobfuscated_js: flat.deobfuscated_js ?? null,
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
    deobfuscated_js: data.deobfuscated_js ?? null,
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

export interface ModelCapabilities {
  tags: string[];
  code_focused: boolean;
  reasoning_focused: boolean;
  large_context: boolean;
  budget_tier: string;
}

export interface OpenRouterModel {
  id: string;
  name: string;
  context_length?: number;
  description?: string;
  architecture_modality?: string;
  architecture_tokenizer?: string;
  architecture_instruct_type?: string;
  pricing_prompt?: string;
  pricing_completion?: string;
  supported_parameters?: string[];
  capabilities?: ModelCapabilities;
}

export interface AgentLimitsSettings {
  max_tools_per_run: number;
  max_tools_per_step: number;
  tool_run_limit_boost: number;
  max_tool_messages_kept: number;
  max_premature_nudges: number;
  list_entries_max: number;
  http_response_default_chars: number;
  script_output_default_chars: number;
  script_code_max_chars: number;
  script_timeout_default_secs: number;
  script_timeout_max_secs: number;
  entry_body_preview_chars: number;
  entry_body_full_default_chars: number;
  absolute_tool_output_ceiling: number;
  agent_planning_timeout_secs: number;
  agent_stream_idle_timeout_secs: number;
  max_script_runs_per_reply: number;
  override_default_tool_output_chars: number;
  override_max_tool_output_chars: number;
}

export const DEFAULT_AGENT_LIMITS: AgentLimitsSettings = {
  max_tools_per_run: 150,
  max_tools_per_step: 20,
  tool_run_limit_boost: 150,
  max_tool_messages_kept: 24,
  max_premature_nudges: 3,
  list_entries_max: 100,
  http_response_default_chars: 32000,
  script_output_default_chars: 64000,
  script_code_max_chars: 48000,
  script_timeout_default_secs: 45,
  script_timeout_max_secs: 120,
  entry_body_preview_chars: 600,
  entry_body_full_default_chars: 48000,
  absolute_tool_output_ceiling: 512000,
  agent_planning_timeout_secs: 180,
  agent_stream_idle_timeout_secs: 35,
  max_script_runs_per_reply: 8,
  override_default_tool_output_chars: 0,
  override_max_tool_output_chars: 0,
};

export interface AgentLimitFieldDoc {
  key: string;
  label: string;
  description: string;
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
  agent_allow_code_execution: boolean;
  agent_python_venv_path: string;
  smart_model_routing: boolean;
  tier1_model: string;
  tier2_model: string;
  tier3_model: string;
  provider: string;
  agent_limits: AgentLimitsSettings;
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

export interface JsDeobfuscateStreamEvent {
  session_id: string;
  entry_index: number;
  content: string;
  done: boolean;
  error?: string | null;
}

export interface ChatToolEvent {
  session_id: string;
  id: string;
  step: number;
  tool: string;
  status: "running" | "done" | "error" | "thinking" | "reasoning" | string;
  detail: string;
  reasoning?: string;
}

export interface ChatContextBudgetEvent {
  session_id: string;
  context_tokens: number;
  hard_max_chars: number;
  summarize_trigger_chars: number;
  used_approx_chars: number;
  percent_used: number;
}

export interface ChatAgentLimitEvent {
  session_id: string;
  limit_kind?: string;
  steps_used: number;
  step_limit: number;
  tools_executed?: number;
  tool_run_limit?: number;
  next_tool_run_limit?: number;
}

export interface ChatSendResult {
  message?: ChatMessage;
  needs_continue: boolean;
  steps_used: number;
  step_limit: number;
  limit_kind?: "step" | "tool" | string;
  tools_executed?: number;
  tool_run_limit?: number;
  next_tool_run_limit?: number;
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

export interface LoadScriptResult {
  file_name: string;
  lines: number;
  revision: number;
}

export interface ToolStep {
  id: number;
  session_id: string;
  event_id: string;
  step: number;
  tool: string;
  status: string;
  detail: string;
  reasoning: string;
  created_at: string;
}

export interface ChatContext {
  context_type: string;
  entry_index?: number;
}
