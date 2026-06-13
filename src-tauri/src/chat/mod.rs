pub mod deobfuscate;
pub mod agent_state;
pub mod embeds;
pub mod knowledge;
mod entry_format;
mod extra_tools;
mod http_tools;
mod live_http_log;
mod output_limits;
mod python_runtime;
mod script_quality;
pub(crate) mod script_workspace;
use agent_state::{ChatAgentState, ChatCancelMode, EmbedOverrides, CHAT_CANCELLED_ERROR};
use crate::db::{self, Database};
use crate::AppState;
use crate::har::js_analyzer::llm_body;
use crate::har::types::{AnalysisSession, AppSettings, HarEntryDetail, HarEntrySummary};
use crate::llm::{self, ChatRequestMessage};
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tokio::time;

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if ChatAgentState::is_cancelled(cancel) {
        Err(CHAT_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

fn agent_cancel_checkpoint(
    state: &AppState,
    session_id: &str,
    cancel: &AtomicBool,
    messages: Vec<ChatRequestMessage>,
    reasoning: String,
    steps_used: usize,
    tools_executed: usize,
    tool_run_limit: usize,
) -> Result<Option<AgentRunOutcome>, String> {
    if !ChatAgentState::is_cancelled(cancel) {
        return Ok(None);
    }
    match state.chat_agents.take_cancel_mode(session_id) {
        ChatCancelMode::KeepProgress => Ok(Some(AgentRunOutcome::Paused {
            messages,
            reasoning,
            steps_used,
            tools_executed,
            tool_run_limit,
        })),
        ChatCancelMode::FinalizePartial => Ok(Some(AgentRunOutcome::NeedsFinalize {
            messages,
            reasoning,
            steps_used,
        })),
        ChatCancelMode::Abort => Err(CHAT_CANCELLED_ERROR.to_string()),
    }
}

pub const DEFAULT_AGENT_MAX_STEPS: usize = 10;
pub const AGENT_MAX_STEPS_CAP: usize = 50;

pub const FINALIZE_AFTER_LIMIT_PROMPT: &str = "[System] The HAR tool step limit was reached before you could finish. \
Do not call any more tools. Provide the best complete answer you can from the tool results already in this conversation. \
Use the required answer structure (Observed in this HAR / Inferred / Self-check). \
If a Python prototype was run via run_script and never succeeded (including mock/simulated output), say clearly that it **failed** — use `{{script}}` only (never paste a new or \"fixed\" script in prose). \
Do not present a broken or mocked script as working. \
At the end, add a brief **Limit reached** section explaining what you were still trying to look up and why the task needed more tool steps than allowed.";

pub const CANCEL_PARTIAL_PROMPT: &str = "[System] The user stopped agent tool planning mid-run. \
Do not call any more tools. Write the best partial answer you can from tool results and reasoning already in this thread. \
Begin with one sentence noting that planning was interrupted and the answer may be incomplete. \
Use the usual answer structure (Observed / Inferred / Self-check) where possible.";

pub fn finalize_assistant_reply(
    state: &AppState,
    session: &AnalysisSession,
    content: &str,
    reasoning: &str,
    thinking_mode: bool,
    embed_overrides: &EmbedOverrides,
) -> Result<String, String> {
    embed_overrides.restore(state, &session.id);
    let content = embeds::reconcile_answer_scripts(content, state, session, embed_overrides);
    let reasoning =
        embeds::reconcile_answer_scripts(reasoning, state, session, embed_overrides);
    let content = embeds::expand_embeds(&content, state, session, embed_overrides)?;
    let reasoning = embeds::expand_embeds(&reasoning, state, session, embed_overrides)?;
    Ok(llm::format_chat_reply(&content, &reasoning, thinking_mode))
}

pub fn finalize_assistant_reply_for_session(
    state: &AppState,
    session_id: &str,
    content: &str,
    reasoning: &str,
    thinking_mode: bool,
    embed_overrides: &EmbedOverrides,
) -> Result<String, String> {
    let session = {
        let db = db::lock_db(&state.db)?;
        db
            .get_session(session_id)?
            .ok_or_else(|| "Session not found".to_string())?
    };
    finalize_assistant_reply(
        state,
        &session,
        content,
        reasoning,
        thinking_mode,
        embed_overrides,
    )
}

pub fn resolve_agent_max_steps(settings: &AppSettings) -> usize {
    settings
        .chat_agent_max_steps
        .clamp(1, AGENT_MAX_STEPS_CAP)
}

#[derive(Debug, Clone)]
pub enum AgentRunOutcome {
    Complete {
        content: String,
        reasoning: String,
        steps_used: usize,
    },
    StepLimitReached {
        messages: Vec<ChatRequestMessage>,
        reasoning: String,
        steps_used: usize,
        tools_executed: usize,
        tool_run_limit: usize,
    },
    ToolRunLimitReached {
        messages: Vec<ChatRequestMessage>,
        reasoning: String,
        steps_used: usize,
        tools_executed: usize,
        tool_run_limit: usize,
    },
    Paused {
        messages: Vec<ChatRequestMessage>,
        reasoning: String,
        steps_used: usize,
        tools_executed: usize,
        tool_run_limit: usize,
    },
    NeedsFinalize {
        messages: Vec<ChatRequestMessage>,
        reasoning: String,
        steps_used: usize,
    },
}

pub fn tool_definitions() -> Vec<Value> {
    let mut tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "list_entries",
                "description": "List HAR entries (minimal metadata). Use first to discover endpoints before fetching details.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Substring to match in URL (case-insensitive)" },
                        "method": { "type": "string", "description": "HTTP method filter, e.g. GET, POST" },
                        "status_min": { "type": "integer", "description": "Minimum HTTP status code" },
                        "status_max": { "type": "integer", "description": "Maximum HTTP status code" },
                        "js_only": { "type": "boolean", "description": "Only JavaScript file entries" },
                        "limit": { "type": "integer", "description": "Max rows (default 50, max 100)" },
                        "offset": { "type": "integer", "description": "Pagination offset (default 0)" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query. When provided, results come from that session instead of the current one." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_entry",
                "description": "Fetch one HAR entry by index: headers, timing, and short body previews (default). Use detail=full only when previews are insufficient; prefer get_entry_part(mode=summary) to understand large JSON/HTML without loading full bodies.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Zero-based entry index from list_entries" },
                        "detail": {
                            "type": "string",
                            "enum": ["overview", "full"],
                            "description": "overview (default): headers + ~600-char body previews. full: larger body slices (scale with model context; pass max_output_chars to raise)."
                        },
                        "max_output_chars": {
                            "type": "integer",
                            "description": "Optional cap for body text returned (full detail / large captures). Defaults scale with the chat model context window."
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query. When provided, the entry is fetched from that session." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_js_analysis",
                "description": "Get JavaScript source excerpt and regex-detected network patterns (fetch, XHR, axios, etc.) for a JS entry. Check deobfuscated availability; prefer get_deobfuscated_js or get_js_snippet for readable code.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Index of a JavaScript entry" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_session_overview",
                "description": "Session metadata and analysis summary (if report was generated).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_chunk_summaries",
                "description": "Get LLM chunk analysis summaries from the parallel analysis phase.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "chunk_indices": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Optional chunk indices; omit to list all available summaries"
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "generate_curl",
                "description": "Build a curl command from a HAR entry, optionally stripped down (omit headers, override URL/body). Does not execute. Use with minimize_http_request or execute_http_request for live iteration.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "url": { "type": "string", "description": "Override URL" },
                        "method": { "type": "string" },
                        "body": { "type": "string" },
                        "omit_body": { "type": "boolean" },
                        "header_names_to_omit": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Header names to exclude from the HAR capture"
                        },
                        "include_headers_only": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "If set, keep only these headers"
                        },
                        "headers": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "value": { "type": "string" }
                                }
                            },
                            "description": "Replace all headers with this list"
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query. When provided, the entry template is fetched from that session." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "execute_http_request",
                "description": "Send a live HTTP request. Use entry_index as golden template, then omit headers or override fields to iterate toward a minimal working call. Returns status, headers, body preview, and equivalent curl.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Optional HAR entry template" },
                        "method": { "type": "string" },
                        "url": { "type": "string" },
                        "headers": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "value": { "type": "string" }
                                }
                            }
                        },
                        "header_names_to_omit": { "type": "array", "items": { "type": "string" } },
                        "include_headers_only": { "type": "array", "items": { "type": "string" } },
                        "body": { "type": "string" },
                        "omit_body": { "type": "boolean" },
                        "max_output_chars": {
                            "type": "integer",
                            "description": "Max response body chars returned (default scales with model context; raise for crawlers / large HTML)."
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "minimize_http_request",
                "description": "From a working HAR entry (golden baseline), live-probe stripped variants and return the most minimal curl that still meets success criteria.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "body_contains": { "type": "string", "description": "Response body must contain this substring" },
                        "expect_status": { "type": "integer", "description": "Required HTTP status (default: any 2xx)" },
                        "expect_status_min": { "type": "integer" },
                        "expect_status_max": { "type": "integer" },
                        "max_attempts": { "type": "integer", "description": "Live probes budget (default 35, max 50)" }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "check_python_environment",
                "description": "Inspect the user's Python runtime (version, pip packages). Call before run_script when imports are needed. Omit packages for a truncated pip list; pass package names to verify they are installed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "packages": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional PyPI package names to check (e.g. requests, httpx)"
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "run_script",
                "description": "Run a Python prototype (preferred) or PowerShell fallback. Keeps a per-session script workspace: first call uses code= (full script). Every later change MUST pass non-empty append_code and/or replacements — calling with no edits is rejected. Use re_run=true only to execute the unchanged script (e.g. after pip install). Syntax-checked before execution.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "language": { "type": "string", "enum": ["python", "powershell"], "description": "Script runtime (default: python)" },
                        "code": { "type": "string", "description": "Full script source — first run only (creates workspace)" },
                        "append_code": { "type": "string", "description": "Non-empty string of lines to append to the workspace script (required when adding/fixing code after the first run)" },
                        "re_run": { "type": "boolean", "description": "Execute the workspace script without code edits (after pip install, or to retry with same source)" },
                        "replacements": {
                            "type": "array",
                            "description": "Search/replace edits applied in order on the workspace script",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "find": { "type": "string" },
                                    "replace": { "type": "string" }
                                },
                                "required": ["find", "replace"]
                            }
                        },
                        "args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "CLI arguments passed after the script path (Python: available as sys.argv[1:])"
                        },
                        "env": {
                            "type": "object",
                            "additionalProperties": { "type": "string" },
                            "description": "Extra environment variables for the script process (Python: os.environ)"
                        },
                        "reset": { "type": "boolean", "description": "Clear workspace before applying edits" },
                        "force": { "type": "boolean", "description": "Re-run after materially editing the script when a prior identical failure triggered a block" },
                        "timeout_secs": { "type": "integer", "description": "Max runtime (default 45, max 120)" },
                        "skip_quality_checks": { "type": "boolean", "description": "Skip mock/stub/demo detection for intermediate test iterations. Use when iterating on a script — the agent will re-run with quality checks on the final submission." },
                        "max_output_chars": {
                            "type": "integer",
                            "description": "Max combined stdout+stderr returned (default scales with model context). Use 80000–150000 for web crawlers that print page text."
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_live_http_requests",
                "description": "List live HTTP requests/responses recorded during this chat session (execute_http_request, execute_request, minimize probes). Use when auth tokens rotated, a request failed after a refresh, or you need to recall prior live results.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Max rows (default 20, max 50)" },
                        "offset": { "type": "integer", "description": "Pagination offset (default 0)" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_live_http_request",
                "description": "Full request/response preview for one recorded live HTTP exchange by id from list_live_http_requests.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Live request id (#N from the log)" }
                    },
                    "required": ["id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_live_auth_state",
                "description": "Latest auth tokens/cookies captured from live HTTP responses in this chat (refresh_token, access_token, Set-Cookie, Authorization). Use before the next execute_http_request or run_script after a token rotation.",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "execute_request",
                "description": "Replay the HAR entry as a live HTTP request (shortcut for execute_http_request with entry_index only). Returns status, headers, and response body preview.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "override_url": { "type": "string", "description": "Optional URL override" },
                        "max_output_chars": {
                            "type": "integer",
                            "description": "Max response body chars returned (default scales with model context)."
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR. When provided, the entry template is fetched from that session." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
    ];
    tools.extend(extra_tools::extra_tool_definitions());
    tools
}

pub(super) fn with_db<T, F>(state: &AppState, f: F) -> Result<T, String>
where
    F: FnOnce(&Database) -> Result<T, String>,
{
    let db = db::lock_db(&state.db)?;
    f(&db)
}

pub fn execute_tool(
    state: &AppState,
    session: &AnalysisSession,
    tool_name: &str,
    arguments: &str,
) -> Result<String, String> {
    let args: Value = if arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(arguments)
            .map_err(|e| format!("Invalid tool arguments JSON: {e}"))?
    };

    let resolved = resolve_target_session(state, session, tool_name, &args)?;
    let effective = resolved.as_ref().unwrap_or(session);

    let result = match tool_name {
        "list_entries" => tool_list_entries(state, effective, &args),
        "get_entry" => tool_get_entry(state, effective, &args),
        "get_js_analysis" => tool_get_js_analysis(state, effective, &args),
        "get_session_overview" => tool_get_session_overview(state, effective),
        "get_chunk_summaries" => tool_get_chunk_summaries(state, effective, &args),
        "generate_curl" => tool_generate_curl(state, effective, &args),
        "list_live_http_requests" => tool_list_live_http_requests(state, effective, &args),
        "get_live_http_request" => tool_get_live_http_request(state, effective, &args),
        "get_live_auth_state" => tool_get_live_auth_state(state, effective),
        "execute_request" | "execute_http_request" | "minimize_http_request" | "run_script"
        | "check_python_environment" | "get_deobfuscated_js" => {
            Err(format!(
                "{tool_name} is handled asynchronously by the agent runner"
            ))
        }
        other => extra_tools::execute_extra_tool(state, effective, other, &args),
    }?;

    Ok(truncate_tool_result(
        state,
        &session.id,
        &result,
        output_limits::parse_max_output_chars(&args),
    ))
}

fn resolve_target_session(
    state: &AppState,
    bound: &AnalysisSession,
    tool_name: &str,
    args: &Value,
) -> Result<Option<AnalysisSession>, String> {
    match tool_name {
        "list_entries"
        | "get_entry"
        | "get_js_analysis"
        | "get_session_overview"
        | "get_chunk_summaries"
        | "generate_curl"
        | "get_entry_part"
        | "summarize_entries"
        | "trace_cookies"
        | "trace_storage"
        | "list_js_scripts"
        | "get_js_call_map"
        | "get_chunk_details"
        | "list_endpoints"
        | "search_bodies"
        | "compare_entries"
        | "get_auth_flow"
        | "decode_jwt"
        | "get_js_snippet"
        | "execute_request"
        | "get_deobfuscated_js"
        | "walk_json"
        | "walk_html" => {
            if let Some(target_id) = args.get("session_id").and_then(|v| v.as_str()) {
                if target_id == bound.id {
                    return Ok(None);
                }
                let db = db::lock_db(&state.db)?;
                db.get_session(target_id)?
                    .map(Some)
                    .ok_or_else(|| format!("Session '{target_id}' not found"))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

fn format_agent_planning_detail(
    display_step: usize,
    model_label: &str,
    elapsed_secs: u64,
    timeout_secs: u64,
    tools_executed: usize,
    streaming: bool,
    retried: bool,
    reasoning_chars: usize,
) -> String {
    let phase = if retried && !streaming {
        "Retrying without streaming after a stall or timeout (more reliable)."
    } else if streaming && reasoning_chars > 0 {
        "Streaming model reasoning and tool selection from OpenRouter…"
    } else if streaming {
        "Connected — waiting for first reasoning tokens from OpenRouter…"
    } else {
        "Waiting for OpenRouter to finish tool planning (non-streaming)…"
    };

    let why_slow = if tools_executed >= 6 {
        "Many live HTTP tool results are in context, so this step often takes 30–90 seconds."
    } else if tools_executed >= 3 {
        "Several tool results are in context — planning may take 15–60 seconds."
    } else {
        "The model is reading HAR tool schemas plus chat history."
    };

    format!(
        "Step {display_step}: Choosing tools via {model_label}\n\
         {phase}\n\
         {why_slow}\n\
         Elapsed {elapsed_secs}s / {timeout_secs}s timeout · \
         {tools_executed} tool lookup(s) this reply · \
         {reasoning_chars} chars of reasoning streamed so far."
    )
}

async fn agent_planning_request(
    settings: &AppSettings,
    model: &str,
    messages: Vec<ChatRequestMessage>,
    tools: Vec<Value>,
    max_output_tokens: u32,
    streaming: bool,
    cancel: Arc<AtomicBool>,
    thinking_tx: tokio::sync::watch::Sender<String>,
) -> Result<llm::AssistantTurn, String> {
    if streaming {
        llm::complete_for_agent_streaming(
            settings,
            model,
            messages,
            tools,
            Some(max_output_tokens),
            move || ChatAgentState::is_cancelled(&cancel),
            move |content, reasoning| {
                let thinking = llm::combine_planning_text(reasoning, content);
                if !thinking.is_empty() {
                    let _ = thinking_tx.send(thinking);
                }
            },
        )
        .await
    } else {
        let turn = llm::complete_for_agent(
            settings,
            model,
            messages,
            tools,
            Some(max_output_tokens),
        )
        .await?;
        let thinking = llm::combine_planning_text(&turn.reasoning, &turn.content);
        if !thinking.is_empty() {
            let _ = thinking_tx.send(thinking);
        }
        Ok(turn)
    }
}

pub async fn run_session_agent<F, G>(
    state: &AppState,
    settings: &AppSettings,
    _model: &str,
    session: &AnalysisSession,
    mut messages: Vec<ChatRequestMessage>,
    max_steps: usize,
    step_offset: usize,
    mut tools_executed: usize,
    tool_run_limit: usize,
    thinking_mode: bool,
    cancel: Arc<AtomicBool>,
    mut on_tool: F,
    mut on_stream: G,
) -> Result<AgentRunOutcome, String>
where
    F: FnMut(&str, usize, &str, &str, &str, &str),
    G: FnMut(&str, &str),
{
    let tools = tool_definitions();
    let mut reasoning_accum = String::new();
    let mut steps_used = 0usize;
    let mut truncation_retries = 0usize;
    let mut premature_nudges = 0usize;
    let mut script_run_attempted = false;
    let limits = llm::resolve_agent_limits(settings);
    let max_premature_nudges = limits.max_premature_nudges;
    let max_tools_per_step = limits.max_tools_per_step;
    let max_output_tokens = llm::agent_max_output_tokens(thinking_mode);

    state
        .chat_agents
        .set_agent_limits(&session.id, limits.clone());

    let models = llm::list_models(&settings.openrouter_api_key)
        .await
        .unwrap_or_default();

    let mut routing = llm::AgentRoutingContext {
        user_wants_script: llm::user_wants_script_from_messages(&messages),
        ..Default::default()
    };
    let mut last_tool_name: Option<String> = None;

    if thinking_mode {
        if let Some(first) = messages.first_mut() {
            if first.role == "system" {
                if let Some(content) = first.content.as_mut() {
                    content.push_str(
                        "\n\nThinking mode: invoke tools only through the native function-calling API. \
                         Do not emit <tool_call> XML or markdown tool blocks. Keep internal reasoning brief.",
                    );
                }
            }
        }
    }

    while steps_used < max_steps {
        if let Some(outcome) = agent_cancel_checkpoint(
            state,
            &session.id,
            &cancel,
            messages.clone(),
            reasoning_accum.clone(),
            step_offset + steps_used,
            tools_executed,
            tool_run_limit,
        )? {
            return Ok(outcome);
        }

        if tools_executed >= tool_run_limit {
            return Ok(AgentRunOutcome::ToolRunLimitReached {
                messages,
                reasoning: reasoning_accum,
                steps_used: step_offset + steps_used,
                tools_executed,
                tool_run_limit,
            });
        }

        steps_used += 1;
        let display_step = step_offset + steps_used;

        routing.step = steps_used;
        routing.tools_executed = tools_executed;
        routing.script_run_attempted = script_run_attempted;
        routing.last_tool_name = last_tool_name.clone();
        routing.estimated_context_chars = llm::estimate_context_chars(&messages);
        routing.user_wants_script = llm::user_wants_script_from_messages(&messages);
        if let Some(status) = state.chat_agents.get_script_run_status(&session.id) {
            routing.script_last_failed = !status.success;
            routing.script_stub_detected = status.stub_detected;
        }

        let (step_model, tier_label) = if thinking_mode {
            (llm::resolve_chat_model(settings, true), "thinking mode")
        } else {
            llm::select_agent_model(settings, &models, &routing)
        };
        let api_model = settings.resolve_api_model(&step_model);
        let model_label = format!("{step_model} ({tier_label})");
        let timeout_secs = limits.agent_planning_timeout_secs;

        on_tool(
            "agent-planning",
            display_step,
            "agent",
            "thinking",
            &format!(
                "Step {display_step}: preparing tool planning via {model_label} \
                 ({tools_executed}/{tool_run_limit} tool call(s) in this reply)…"
            ),
            "",
        );
        tokio::task::yield_now().await;

        let context_budget = llm::ensure_model_context_for_settings(
            &settings.openrouter_api_key,
            &api_model,
            settings,
        )
        .await;
        state
            .chat_agents
            .set_context_budget(&session.id, context_budget);

        let msg_chars = llm::estimate_context_chars(&messages);
        let pct = if context_budget.hard_max_chars > 0 {
            ((msg_chars as u64 * 100) / context_budget.hard_max_chars as u64).min(100) as u32
        } else {
            0
        };

        let _ = on_tool(
            "context-budget",
            display_step,
            "budget",
            "ok",
            &format!("{}/{}", context_budget.context_tokens / 1000, msg_chars),
            &format!("{}", pct),
        );

        if llm::should_summarize_messages(&messages, context_budget) {
            on_tool(
                "context-compact",
                display_step,
                "context-summarize",
                "running",
                &format!(
                    "Summarizing earlier chat & tool results ({}K context model)…",
                    context_budget.context_tokens / 1000
                ),
                "",
            );
            tokio::task::yield_now().await;

            match llm::compact_messages_if_needed(settings, &api_model, &mut messages).await {
                Ok(Some(report)) => {
                    on_tool(
                        "context-compact",
                        display_step,
                        "context-summarize",
                        "done",
                        &format!(
                            "Compressed ~{} chars into a summary ({}K context). Your question(s) were kept verbatim.",
                            report.removed_chars,
                            report.context_tokens / 1000
                        ),
                        "",
                    );
                }
                Ok(None) => {
                    on_tool(
                        "context-compact",
                        display_step,
                        "context-summarize",
                        "done",
                        "Summarization skipped — not enough compressible context.",
                        "",
                    );
                }
                Err(err) => {
                    on_tool(
                        "context-compact",
                        display_step,
                        "context-summarize",
                        "error",
                        &err,
                        "",
                    );
                }
            }
        }

        let reasoning_id = format!("reasoning-{display_step}");
        on_tool(
            &reasoning_id,
            display_step,
            "reasoning",
            "streaming",
            "",
            "",
        );

        let settings_for_llm = settings.clone();
        let messages_for_llm = messages.clone();
        let tools_for_llm = tools.clone();
        let cancel_for_planning = cancel.clone();
        let (thinking_tx, mut thinking_rx) = tokio::sync::watch::channel(String::new());
        let mut planning_streaming = true;
        let mut planning_retried = false;

        let mut planning_fut = Box::pin(time::timeout(
            Duration::from_secs(timeout_secs),
            agent_planning_request(
                &settings_for_llm,
                &api_model,
                messages_for_llm.clone(),
                tools_for_llm.clone(),
                max_output_tokens,
                planning_streaming,
                cancel_for_planning.clone(),
                thinking_tx.clone(),
            ),
        ));

        let mut elapsed_secs = 0u64;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        heartbeat.tick().await;
        let mut cancel_poll = tokio::time::interval(std::time::Duration::from_millis(250));
        cancel_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        cancel_poll.tick().await;

        let mut turn = loop {
            tokio::select! {
                biased;
                _ = heartbeat.tick() => {
                    if let Some(outcome) = agent_cancel_checkpoint(
                        state,
                        &session.id,
                        &cancel,
                        messages.clone(),
                        reasoning_accum.clone(),
                        step_offset + steps_used,
                        tools_executed,
                        tool_run_limit,
                    )? {
                        return Ok(outcome);
                    }
                    elapsed_secs += 1;
                    let reasoning_chars = thinking_rx.borrow().len();
                    let detail = format_agent_planning_detail(
                        display_step,
                        &model_label,
                        elapsed_secs,
                        timeout_secs,
                        tools_executed,
                        planning_streaming,
                        planning_retried,
                        reasoning_chars,
                    );
                    on_tool("agent-planning", display_step, "agent", "thinking", &detail, "");
                }
                _ = cancel_poll.tick() => {
                    if let Some(outcome) = agent_cancel_checkpoint(
                        state,
                        &session.id,
                        &cancel,
                        messages.clone(),
                        reasoning_accum.clone(),
                        step_offset + steps_used,
                        tools_executed,
                        tool_run_limit,
                    )? {
                        return Ok(outcome);
                    }
                }
                thinking = thinking_rx.changed() => {
                    if thinking.is_ok() {
                        let text = thinking_rx.borrow().clone();
                        if !text.is_empty() {
                            on_tool(
                                &reasoning_id,
                                display_step,
                                "reasoning",
                                "streaming",
                                "",
                                &text,
                            );
                        }
                    }
                }
                result = &mut planning_fut => {
                    match result {
                        Ok(Ok(turn)) => break turn,
                        Ok(Err(err)) if !planning_retried && llm::stream_error_retriable(&err) => {
                            planning_retried = true;
                            planning_streaming = false;
                            on_tool(
                                "agent-planning",
                                display_step,
                                "agent",
                                "thinking",
                                "OpenRouter returned an error — retrying once without streaming…",
                                "",
                            );
                            planning_fut = Box::pin(time::timeout(
                                Duration::from_secs(timeout_secs),
                                agent_planning_request(
                                    &settings_for_llm,
                                    &api_model,
                                    messages_for_llm.clone(),
                                    tools_for_llm.clone(),
                                    max_output_tokens,
                                    false,
                                    cancel_for_planning.clone(),
                                    thinking_tx.clone(),
                                ),
                            ));
                        }
                        Ok(Err(err)) => {
                            on_tool("agent-planning", display_step, "agent", "error", &err, "");
                            return Err(err);
                        }
                        Err(_) if !planning_retried => {
                            planning_retried = true;
                            planning_streaming = false;
                            on_tool(
                                &format!("agent-stall-{display_step}"),
                                display_step,
                                "agent",
                                "stalled",
                                &format!(
                                    "OpenRouter planning exceeded {timeout_secs}s — \
                                     retrying once without streaming (often completes faster)…"
                                ),
                                "",
                            );
                            planning_fut = Box::pin(time::timeout(
                                Duration::from_secs(timeout_secs),
                                agent_planning_request(
                                    &settings_for_llm,
                                    &api_model,
                                    messages_for_llm.clone(),
                                    tools_for_llm.clone(),
                                    max_output_tokens,
                                    false,
                                    cancel_for_planning.clone(),
                                    thinking_tx.clone(),
                                ),
                            ));
                        }
                        Err(_) => {
                            let err = format!(
                                "OpenRouter planning timed out after {timeout_secs}s \
                                 (including one non-streaming retry). \
                                 Tap Stop, start a new chat, or try a different model."
                            );
                            on_tool("agent-planning", display_step, "agent", "error", &err, "");
                            return Err(err);
                        }
                    }
                }
            }
        };

        llm::enrich_agent_turn(&mut turn);

        let total_msg_chars: usize = messages.iter().map(|m| m.content.as_deref().map(str::len).unwrap_or(0)).sum();
        state.chat_agents.add_token_input(&session.id, total_msg_chars);

        let total_msg_chars: usize = messages.iter().map(|m| m.content.as_deref().map(str::len).unwrap_or(0)).sum();
        state.chat_agents.add_token_input(&session.id, total_msg_chars);

        let planning_done = if turn.tool_calls.is_empty() {
            "Model ready to answer".to_string()
        } else {
            format!("Model chose {} tool(s)", turn.tool_calls.len())
        };
        on_tool(
            "agent-planning",
            display_step,
            "agent",
            "done",
            &planning_done,
            "",
        );

        let step_thinking = llm::combine_planning_text(&turn.reasoning, &turn.content);
        let output_chars = step_thinking.len() + turn.content.len();
        state.chat_agents.add_token_output(&session.id, output_chars);

        if turn.tool_calls.is_empty() {
            let incomplete = llm::has_incomplete_tool_call(&turn.content)
                || llm::has_incomplete_tool_call(&turn.reasoning);

            if incomplete {
                if turn.finish_reason.as_deref() == Some("length") && truncation_retries < 1 {
                    truncation_retries += 1;
                    steps_used = steps_used.saturating_sub(1);
                    messages.push(llm::ChatRequestMessage::text(
                        "user",
                        "[System] Your previous response hit the output token limit before the tool call finished. \
                         Use the native function-calling API only — never <tool_call> tags, DSML markup, or plain-text invoke blocks. Keep reasoning brief and call tools promptly.",
                    ));
                    continue;
                }

                return Err(
                    "The model tried to call a tool in plain text (DSML/<tool_call> markup) instead of native function calling. \
                     The attempt was not executed. Try again without thinking mode, or switch to a model with reliable tool support."
                        .to_string(),
                );
            }

            if premature_nudges < max_premature_nudges {
                let script_succeeded =
                    state.chat_agents.script_last_run_succeeded(&session.id);
                if let Some(nudge) = llm::agent_continue_nudge(
                    &messages,
                    &turn,
                    script_run_attempted,
                    script_succeeded,
                ) {
                    premature_nudges += 1;
                        let final_content = llm::resolve_final_agent_content(&turn);
                        let planning_only = llm::planning_text_for_display(&turn, &final_content);
                        if !planning_only.is_empty() {
                            reasoning_accum.push_str(&planning_only);
                            reasoning_accum.push('\n');
                            on_tool(
                                &format!("reasoning-{display_step}-nudge{premature_nudges}"),
                                display_step,
                                "reasoning",
                                "done",
                                "",
                                &planning_only,
                            );
                        }
                        if !final_content.trim().is_empty() {
                            messages.push(llm::ChatRequestMessage::text(
                                "assistant",
                                final_content,
                            ));
                        }
                        messages.push(llm::ChatRequestMessage::text("user", nudge));
                        on_tool(
                            "agent-planning",
                            display_step,
                            "agent",
                            "thinking",
                            &format!(
                                "Model stopped before calling tools — continuing… (retry {}/{})",
                                premature_nudges, max_premature_nudges
                            ),
                            "",
                        );
                        steps_used = steps_used.saturating_sub(1);
                        continue;
                }
            }

            let script_succeeded = state.chat_agents.script_last_run_succeeded(&session.id);
            let script_pending =
                llm::script_delivery_pending(&messages, script_run_attempted);
            if script_pending {
                return Err(
                    "The model stopped without calling run_script for the requested Python script. \
                     Try sending the message again, or switch to a model with reliable tool calling."
                        .to_string(),
                );
            }

            let final_content = llm::resolve_final_agent_content(&turn);
            if llm::script_success_pending(&messages, script_run_attempted, script_succeeded) {
                if !llm::looks_like_honest_script_failure_text(&final_content)
                    && (llm::looks_like_false_script_success(&final_content)
                        || llm::answer_text_score(&final_content) >= 120)
                {
                    return Err(
                        "The model finished without a successful run_script but did not report failure honestly. \
                         Retry the chat or switch models — the agent must either deliver a working script or explain why it cannot."
                            .to_string(),
                    );
                }
            }

            let planning_only = llm::planning_text_for_display(&turn, &final_content);
            reasoning_accum = planning_only.clone();

            if !planning_only.is_empty() {
                on_tool(
                    &reasoning_id,
                    display_step,
                    "reasoning",
                    "done",
                    "",
                    &planning_only,
                );
            } else {
                on_tool(&reasoning_id, display_step, "reasoning", "done", "", "");
            }

            llm::emit_simulated_stream(
                &final_content,
                "",
                |c, r| on_stream(c, r),
                || ChatAgentState::is_cancelled(&cancel),
            )
            .await?;
            return Ok(AgentRunOutcome::Complete {
                content: final_content,
                reasoning: planning_only,
                steps_used,
            });
        }

        if !step_thinking.is_empty() {
            reasoning_accum = step_thinking.clone();
            on_tool(
                &reasoning_id,
                display_step,
                "reasoning",
                "done",
                "",
                &step_thinking,
            );
        } else {
            on_tool(&reasoning_id, display_step, "reasoning", "done", "", "");
        }

        messages.push(llm::ChatRequestMessage::assistant_tool_calls(
            turn.tool_calls.clone(),
            if turn.content.trim().is_empty() {
                None
            } else {
                Some(turn.content)
            },
        ));

        let mut calls = turn.tool_calls;
        if calls.len() > max_tools_per_step {
            calls.truncate(max_tools_per_step);
        }

        for call in &calls {
            let running_detail = if call.function.name == "run_script" {
                "Preparing script (syntax check, then run)…".to_string()
            } else {
                call.function.arguments.clone()
            };
            on_tool(
                &call.id,
                display_step,
                &call.function.name,
                "running",
                &running_detail,
                "",
            );
        }
        tokio::task::yield_now().await;

        for call in calls {
            if let Some(outcome) = agent_cancel_checkpoint(
                state,
                &session.id,
                &cancel,
                messages.clone(),
                reasoning_accum.clone(),
                step_offset + steps_used,
                tools_executed,
                tool_run_limit,
            )? {
                return Ok(outcome);
            }
            let tool_name = call.function.name.clone();
            let arguments = call.function.arguments.clone();
            let call_id = call.id.clone();

            let result = if tool_name == "run_script" {
                let mut script_fut = Box::pin(execute_session_tool(
                    state,
                    session,
                    &tool_name,
                    &arguments,
                ));
                let started = Instant::now();
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker.tick().await;
                loop {
                    tokio::select! {
                        r = script_fut.as_mut() => break r,
                        _ = ticker.tick() => {
                            let secs = started.elapsed().as_secs();
                            on_tool(
                                &call_id,
                                display_step,
                                &tool_name,
                                "running",
                                &format!("Running script locally… {secs}s"),
                                "",
                            );
                        }
                    }
                }
            } else {
                execute_session_tool(state, session, &tool_name, &arguments).await
            };

            if tool_name == "run_script" {
                script_run_attempted = true;
            }
            last_tool_name = Some(tool_name.clone());

            let content = match result {
                Ok(text) => {
                    let text = if tool_name == "check_python_environment" {
                        truncate_tool_result(
                            state,
                            &session.id,
                            &text,
                            output_limits::parse_max_output_chars(
                                &serde_json::from_str::<Value>(&arguments).unwrap_or(json!({})),
                            ),
                        )
                    } else {
                        text
                    };
                    let is_script_tool = tool_name == "run_script" || tool_name == "edit_script";
                    let preview = if is_script_tool {
                        text.clone()
                    } else if text.len() > 120 {
                        format!("{}…", preview_chars(&text, 120))
                    } else {
                        text.clone()
                    };
                    let script_diff = if is_script_tool {
                        state.chat_agents.get_last_script_diff(&session.id)
                    } else {
                        None
                    };
                    on_tool(
                        &call_id,
                        display_step,
                        &tool_name,
                        "done",
                        &preview,
                        script_diff.as_deref().unwrap_or(""),
                    );
                    text
                }
                Err(err) => {
                    let script_diff = if tool_name == "run_script" {
                        state.chat_agents.get_last_script_diff(&session.id)
                    } else {
                        None
                    };
                    on_tool(
                        &call_id,
                        display_step,
                        &tool_name,
                        "error",
                        &err,
                        script_diff.as_deref().unwrap_or(""),
                    );
                    format!("Tool error: {err}")
                }
            };

            tools_executed += 1;
            messages.push(llm::ChatRequestMessage::tool_result(call.id, content));
            if tools_executed >= tool_run_limit {
                return Ok(AgentRunOutcome::ToolRunLimitReached {
                    messages,
                    reasoning: reasoning_accum,
                    steps_used: step_offset + steps_used,
                    tools_executed,
                    tool_run_limit,
                });
            }
            tokio::task::yield_now().await;
        }
    }

    Ok(AgentRunOutcome::StepLimitReached {
        messages,
        reasoning: reasoning_accum,
        steps_used: step_offset + steps_used,
        tools_executed,
        tool_run_limit,
    })
}

pub async fn force_finalize_agent<G>(
    settings: &AppSettings,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    mut _reasoning_accum: String,
    cancel: Arc<AtomicBool>,
    mut on_stream: G,
    prompt: &str,
) -> Result<(String, String), String>
where
    G: FnMut(&str, &str),
{
    ensure_not_cancelled(&cancel)?;
    messages.push(llm::ChatRequestMessage::text("user", prompt));

    let mut turn = llm::complete_for_agent_streaming(
        settings,
        model,
        messages,
        vec![],
        Some(llm::agent_max_output_tokens(false)),
        || ChatAgentState::is_cancelled(&cancel),
        |c, r| on_stream(c, r),
    )
    .await?;

    llm::enrich_agent_turn(&mut turn);

    let final_content = llm::resolve_final_agent_content(&turn);
    let planning_only = llm::planning_text_for_display(&turn, &final_content);

    Ok((final_content, planning_only))
}

fn script_exit_success(output: &str) -> bool {
    output
        .split("Script finished (exit code ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .is_some_and(|code| code.trim() == "0")
}

fn script_stderr_excerpt(output: &str) -> String {
    let stderr = output
        .split("--- stderr ---")
        .nth(1)
        .unwrap_or("")
        .trim();
    if stderr.is_empty() {
        return String::new();
    }
    preview_chars(stderr, 800)
}

fn script_failure_signature(output: &str, stub_detected: bool) -> Option<String> {
    if stub_detected {
        return Some("stub_or_mock_detected".to_string());
    }
    if script_exit_success(output) {
        return None;
    }
    if output.contains("SCRIPT_BLOCKED") {
        return Some("missing_package".to_string());
    }
    let stderr = output.split("--- stderr ---").nth(1).unwrap_or(output);
    let sample: String = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    if sample.is_empty() {
        Some("nonzero_exit_no_stderr".to_string())
    } else {
        Some(sample.chars().take(240).collect())
    }
}

async fn execute_session_tool(
    state: &AppState,
    session: &AnalysisSession,
    tool_name: &str,
    arguments: &str,
) -> Result<String, String> {
    let args: Value = if arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(arguments)
            .map_err(|e| format!("Invalid tool arguments JSON: {e}"))?
    };

    match tool_name {
        "execute_request" | "execute_http_request" | "minimize_http_request" | "run_script"
        | "check_python_environment" | "get_deobfuscated_js" => {
            let settings = {
                let db = db::lock_db(&state.db)?;
                db.get_settings()?
            };

            match tool_name {
                "check_python_environment" => {
                    let packages: Vec<String> = args
                        .get("packages")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    python_runtime::check_python_environment(&settings, &packages).await
                }
                    "get_deobfuscated_js" => {
                    let index = arg_entry_index(&args)?;
                    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                    let resolved = resolve_target_session(state, session, "get_deobfuscated_js", &args)?;
                    let effective = resolved.as_ref().unwrap_or(session);
                    match deobfuscate::ensure_entry_deobfuscated(
                        state,
                        &settings,
                        effective,
                        index,
                        force,
                    )
                    .await
                    {
                        Ok(code) => Ok(format!(
                            "Deobfuscated JavaScript for entry [{index}] (saved to session):\n\n```javascript\n{}\n```",
                            llm_body(&code)
                        )),
                        Err(err) => {
                            let entry = {
                                let db = db::lock_db(&state.db)?;
                                db.get_entry_detail(&effective.id, index)?
                                    .ok_or_else(|| format!("Entry {index} not found"))?
                            };
                            Ok(format!(
                                "Deobfuscation failed: {err}\n\nFalling back to raw JavaScript source for entry [{index}]:\n\n```javascript\n{}\n```\n\nUse get_js_snippet or get_js_analysis to inspect specific sections.",
                                llm_body(&entry.response_body)
                            ))
                        }
                    }
                }
                "execute_request" => {
                    let index = arg_entry_index(&args)?;
                    let resolved = resolve_target_session(state, session, "execute_request", &args)?;
                    let effective = resolved.as_ref().unwrap_or(session);
                    let entry = {
                        let db = db::lock_db(&state.db)?;
                        db.get_entry_detail(&effective.id, index)?
                            .ok_or_else(|| format!("Entry index {index} not found"))?
                    };
                    let override_url = args.get("override_url").and_then(|v| v.as_str());
                    replay_har_entry(
                        state,
                        &session.id,
                        &entry,
                        override_url,
                        output_limits::parse_max_output_chars(&args),
                    )
                    .await
                }
                "execute_http_request" => execute_http_request_tool(state, session, &args).await,
                "minimize_http_request" => minimize_http_request_tool(state, session, &args).await,
                "run_script" => {
                    if !settings.agent_allow_code_execution {
                        return Err(
                            "Agent script execution is disabled. Enable it in Settings → Allow agent to run scripts."
                                .to_string(),
                        );
                    }
                    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                    let skip_checks = args
                        .get("skip_quality_checks")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let default_lang = args
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| http_tools::suggested_script_language());
                    let current = state.chat_agents.get_script(&session.id);
                    let (script, diff) = script_workspace::resolve_script_edit(
                        current.as_ref(),
                        &args,
                        default_lang,
                    )?;
                    let re_run_only = diff.is_none();
                    if re_run_only
                        && current.is_some()
                        && !script_workspace::allows_rerun_without_edit(&args, force)
                        && !skip_checks
                    {
                        let err = script_workspace::format_no_edit_error(&script);
                        let panel = script_workspace::format_script_tool_panel(
                            None,
                            &script,
                            true,
                            false,
                            &err,
                        );
                        state
                            .chat_agents
                            .set_last_script_diff(&session.id, panel);
                        return Err(err);
                    }
                    if diff.is_some() {
                        state
                            .chat_agents
                            .set_script(&session.id, script.clone());
                        state.chat_agents.push_script_to_history(&session.id, script.clone());
                    }
                    if !skip_checks {
                        if let Some(block_msg) = state.chat_agents.should_block_script_run(
                            &session.id,
                            force,
                            script.revision,
                        ) {
                            let panel = script_workspace::format_script_tool_panel(
                                diff.as_deref(),
                                &script,
                                re_run_only,
                                false,
                                &block_msg,
                            );
                            state
                                .chat_agents
                                .set_last_script_diff(&session.id, panel);
                            return Err(block_msg);
                        }
                    }
                    state
                        .chat_agents
                        .set_script(&session.id, script.clone());
                    state.chat_agents.push_script_to_history(&session.id, script.clone());
                    if !skip_checks {
                        if let Some(stub_reason) = script_quality::detect_stub_code(&script.code) {
                            let err = script_quality::format_stub_rejection(&stub_reason);
                            state.chat_agents.set_script_run_status(
                                &session.id,
                                agent_state::ScriptRunStatus {
                                    revision: script.revision,
                                    success: false,
                                    stderr_excerpt: format!("[stub rejected: {stub_reason}]"),
                                    stub_detected: true,
                                },
                            );
                            if let Some(hint) = state.chat_agents.record_script_run(
                                &session.id,
                                script.revision,
                                false,
                                Some("stub_code"),
                            ) {
                                eprintln!("HARalyzer: script stub rejection: {hint}");
                            }
                            let panel = script_workspace::format_script_tool_panel(
                                diff.as_deref(),
                                &script,
                                re_run_only,
                                false,
                                &err,
                            );
                            state.chat_agents.set_last_script_diff(&session.id, panel);
                            return Err(err);
                        }
                        let quality_issues = script_quality::detect_quality_issues(&script.code);
                        if !quality_issues.is_empty() {
                            eprintln!("HARalyzer: script quality warnings (rev {}): {:?}", script.revision, quality_issues);
                        }
                    }
                    if !skip_checks {
                        if let Some(security_threat) = script_quality::detect_malicious_code(&script.code) {
                            let err = script_quality::format_security_rejection(&security_threat);
                            state.chat_agents.set_script_run_status(
                                &session.id,
                                agent_state::ScriptRunStatus {
                                    revision: script.revision,
                                    success: false,
                                    stderr_excerpt: format!("[security blocked: {security_threat}]"),
                                    stub_detected: false,
                                },
                            );
                            let panel = script_workspace::format_script_tool_panel(
                                diff.as_deref(),
                                &script,
                                re_run_only,
                                false,
                                &format!("[SECURITY] {security_threat}"),
                            );
                            state
                                .chat_agents
                                .set_last_script_diff(&session.id, panel);
                            return Err(err);
                        }
                    }
                    if !skip_checks {
                        let auth_log = state.chat_agents.get_live_http_log(&session.id);
                        if let Some(stale_msg) = live_http_log::check_script_auth_staleness(
                            &script.code,
                            auth_log.auth_state(),
                        ) {
                            if !force {
                                let err =
                                    live_http_log::format_script_staleness_warning(&stale_msg);
                                let panel = script_workspace::format_script_tool_panel(
                                    diff.as_deref(),
                                    &script,
                                    re_run_only,
                                    false,
                                    &err,
                                );
                                state
                                    .chat_agents
                                    .set_last_script_diff(&session.id, panel);
                                return Err(err);
                            }
                        }
                    }
                    let limits = state.chat_agents.get_agent_limits(&session.id);
                    let timeout = args
                        .get("timeout_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(limits.script_timeout_default_secs)
                        .min(limits.script_timeout_max_secs);
                    let (cli_args, env) = http_tools::parse_script_run_options(&args);
                    let header = script_workspace::script_workspace_header(&script);
                    let budget = state.chat_agents.get_context_budget(&session.id);
                    let output_limit = output_limits::effective_output_limit(
                        &budget,
                        output_limits::parse_max_output_chars(&args),
                    );
                    let result = http_tools::run_agent_script(
                        &settings,
                        &script.code,
                        &script.language,
                        timeout,
                        &cli_args,
                        &env,
                        output_limit,
                        limits.script_code_max_chars,
                        limits.script_timeout_max_secs,
                    )
                    .await?;
                    let exit_ok = script_exit_success(&result);
                    let stub_reason = if skip_checks {
                        None
                    } else {
                        script_quality::detect_stub_output(&result)
                    };
                    let stub_detected = stub_reason.is_some();
                    let success = exit_ok && !stub_detected;
                    let stderr = if stub_detected {
                        format!("[stub detected: {}]", stub_reason.as_deref().unwrap_or("mock output"))
                    } else {
                        script_stderr_excerpt(&result)
                    };
                    state.chat_agents.set_script_run_status(
                        &session.id,
                        agent_state::ScriptRunStatus {
                            revision: script.revision,
                            success,
                            stderr_excerpt: stderr.clone(),
                            stub_detected,
                        },
                    );
                    let panel = script_workspace::format_script_tool_panel(
                        diff.as_deref(),
                        &script,
                        re_run_only,
                        success,
                        &stderr,
                    );
                    state
                        .chat_agents
                        .set_last_script_diff(&session.id, panel);
                    let failure_sig = script_failure_signature(&result, stub_detected);
                    let mut out = format!(
                        "{header}Use append_code/replacements for next edit — do not resend full code.\n\n{result}"
                    );
                    if let Some(stub_reason) = stub_reason {
                        out.push_str(&script_quality::format_stub_output_warning(&stub_reason));
                    }
                    if let Some(hint) = state.chat_agents.record_script_run(
                        &session.id,
                        script.revision,
                        success,
                        failure_sig.as_deref(),
                    ) {
                        out.push_str("\n\n[Script retry guidance] ");
                        out.push_str(&hint);
                    }
                    Ok(out)
                }
                _ => unreachable!(),
            }
        }
        _ => execute_tool(state, session, tool_name, arguments),
    }
}

async fn execute_http_request_tool(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let mut spec = if let Some(index) = args.get("entry_index").and_then(|v| v.as_u64()) {
        let index = index as usize;
        let db = db::lock_db(&state.db)?;
        let entry = db
            .get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry index {index} not found"))?;
        http_tools::spec_from_entry(&entry)
    } else {
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Provide entry_index or method+url".to_string())?;
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Provide entry_index or method+url".to_string())?;
        http_tools::HttpRequestSpec {
            method: method.to_ascii_uppercase(),
            url: url.to_string(),
            headers: args
                .get("headers")
                .and_then(|v| v.as_array())
                .map(|a| http_tools::parse_header_list(a))
                .unwrap_or_default(),
            body: args
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }
    };

    http_tools::apply_request_overrides(&mut spec, args);
    let budget = state.chat_agents.get_context_budget(&session.id);
    let max_output = output_limits::parse_max_output_chars(args);
    let body_limit = output_limits::effective_output_limit(&budget, max_output);
    let result = http_tools::execute_http_spec_with_limit(&spec, body_limit).await?;
    let formatted = http_tools::format_http_result("Live HTTP request", &spec, &result);
    let with_auth = append_live_http_notes(state, &session.id, "execute_http_request", &spec, &result, &formatted);
    Ok(truncate_tool_result(
        state,
        &session.id,
        &with_auth,
        max_output,
    ))
}

async fn minimize_http_request_tool(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = {
        let db = db::lock_db(&state.db)?;
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry index {index} not found"))?
    };
    let criteria = http_tools::success_criteria_from_args(args);
    let max_attempts = http_tools::default_max_minimize_attempts(args);
    let (report, probes) =
        http_tools::minimize_http_request(&entry, &criteria, max_attempts).await?;
    for probe in probes {
        record_live_http_exchange(
            state,
            &session.id,
            &probe.label,
            &probe.spec,
            &probe.result,
        );
    }
    Ok(truncate_tool_result(
        state,
        &session.id,
        &report,
        output_limits::parse_max_output_chars(args),
    ))
}

pub async fn replay_har_entry(
    state: &AppState,
    session_id: &str,
    entry: &HarEntryDetail,
    override_url: Option<&str>,
    max_output: Option<usize>,
) -> Result<String, String> {
    let index = entry.summary.index;
    let mut spec = http_tools::spec_from_entry(entry);
    if let Some(url) = override_url {
        spec.url = url.to_string();
    }
    let budget = state.chat_agents.get_context_budget(session_id);
    let body_limit = output_limits::effective_output_limit(&budget, max_output);
    let result = http_tools::execute_http_spec_with_limit(&spec, body_limit).await?;
    let formatted = format!(
        "Replayed entry [{index}]\n{}",
        http_tools::format_http_result("Live replay", &spec, &result)
    );
    let with_auth = append_live_http_notes(state, session_id, "execute_request", &spec, &result, &formatted);
    Ok(truncate_tool_result(
        state,
        session_id,
        &with_auth,
        max_output,
    ))
}

fn record_live_http_exchange(
    state: &AppState,
    session_id: &str,
    source_tool: &str,
    spec: &http_tools::HttpRequestSpec,
    result: &http_tools::HttpExecuteResult,
) -> live_http_log::LiveHttpRecord {
    state
        .chat_agents
        .with_live_http_log(session_id, |log| log.record_exchange(source_tool, spec, result))
}

fn append_live_http_notes(
    state: &AppState,
    session_id: &str,
    source_tool: &str,
    spec: &http_tools::HttpRequestSpec,
    result: &http_tools::HttpExecuteResult,
    base: &str,
) -> String {
    let record = record_live_http_exchange(state, session_id, source_tool, spec, result);
    let notes = live_http_log::LiveHttpSessionLog::format_warnings_for_result(&record);
    if notes.is_empty() {
        base.to_string()
    } else {
        format!("{base}{notes}")
    }
}

fn tool_list_live_http_requests(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 50) as usize;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let log = state.chat_agents.get_live_http_log(&session.id);
    Ok(log.list_records(limit, offset))
}

fn tool_get_live_http_request(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "Missing id (live request # from list_live_http_requests)".to_string())?
        as u32;
    let log = state.chat_agents.get_live_http_log(&session.id);
    log.get_record(id)
        .ok_or_else(|| format!("Live HTTP request #{id} not found in this session log."))
}

fn tool_get_live_auth_state(state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let log = state.chat_agents.get_live_http_log(&session.id);
    Ok(log.auth_state().format_summary())
}

fn truncate_tool_result(
    state: &AppState,
    session_id: &str,
    text: &str,
    requested: Option<usize>,
) -> String {
    let budget = state.chat_agents.get_context_budget(session_id);
    let max = output_limits::effective_output_limit(&budget, requested)
        .min(budget.tool_result_max_chars);
    output_limits::truncate_output(
        text,
        max,
        "Pass max_output_chars on the tool call for a larger slice, or paginate in run_script.",
    )
}

pub(super) fn preview_chars(text: &str, max: usize) -> String {
    safe_byte_slice(text, 0, max).to_string()
}

pub(super) fn safe_byte_slice(text: &str, start: usize, end: usize) -> &str {
    let mut start = start.min(text.len());
    let mut end = end.min(text.len());
    while start < end && !text.is_char_boundary(start) {
        start += 1;
    }
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[start..end]
}

fn tool_list_entries(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let limits = state.chat_agents.get_agent_limits(&session.id);
    let query = args.get("query").and_then(|v| v.as_str());
    let method = args.get("method").and_then(|v| v.as_str());
    let status_min = args.get("status_min").and_then(|v| v.as_u64()).map(|v| v as u16);
    let status_max = args.get("status_max").and_then(|v| v.as_u64()).map(|v| v as u16);
    let js_only = args.get("js_only").and_then(|v| v.as_bool()).unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(limits.list_entries_max as u64) as usize;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let (entries, total) = with_db(state, |db| {
        let entries = db.search_entries(
            &session.id,
            query,
            method,
            status_min,
            status_max,
            js_only,
            limit,
            offset,
        )?;
        let total = db.count_entries(&session.id, query, method, status_min, status_max, js_only)?;
        Ok((entries, total))
    })?;

    if entries.is_empty() {
        return Ok(format!(
            "No entries matched (total matching: {total}, offset: {offset})."
        ));
    }

    let mut out = format!(
        "Showing {} of {total} matching entries (offset {offset}):\n\n",
        entries.len()
    );
    for e in &entries {
        out.push_str(&format_entry_line(e));
        out.push('\n');
    }
    Ok(out)
}

fn tool_get_entry(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let level = entry_format::EntryDetailLevel::parse(args.get("detail").and_then(|v| v.as_str()));
    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry index {index} not found"))
    })?;
    let budget = state.chat_agents.get_context_budget(&session.id);
    let body_limit = if level == entry_format::EntryDetailLevel::Full {
        Some(output_limits::effective_output_limit(
            &budget,
            output_limits::parse_max_output_chars(args),
        ))
    } else {
        None
    };
    Ok(entry_format::format_entry_detail(&entry, level, body_limit))
}

fn tool_get_js_analysis(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry index {index} not found"))
    })?;

    if !entry.summary.is_javascript {
        return Ok(format!(
            "Entry [{index}] is not a JavaScript file (mime: {}). Use get_entry for HTTP details.",
            entry.summary.mime_type
        ));
    }

    let mut out = format!(
        "JavaScript entry [{index}] {} {}\nStatus: {} · {} bytes\n\n",
        entry.summary.method, entry.summary.url, entry.summary.status, entry.summary.size
    );

    if entry.js_insights.is_empty() {
        out.push_str("No network patterns detected by static analysis.\n\n");
    } else {
        out.push_str("Detected patterns:\n");
        for insight in &entry.js_insights {
            out.push_str(&format!("- {insight}\n"));
        }
        out.push('\n');
    }

    if entry.deobfuscated_js.as_ref().is_some_and(|c| !c.trim().is_empty()) {
        out.push_str(
            "Deobfuscated version available — use get_deobfuscated_js for full readable source, or get_js_snippet to quote line ranges.\n\n",
        );
    } else {
        out.push_str(
            "No deobfuscated version cached. User can run \"Deobfuscate with AI\" in the JS tab, or use get_js_snippet with use_deobfuscated=false for raw source.\n\n",
        );
    }

    if entry.response_body.is_empty() {
        out.push_str("(No JS source body stored for this entry.)");
    } else {
        out.push_str("Source excerpt:\n```javascript\n");
        out.push_str(&llm_body(&entry.response_body));
        out.push_str("\n```");
    }

    Ok(out)
}

fn tool_get_session_overview(state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let chunks = with_db(state, |db| db.get_session_chunks(&session.id))?;
    let summarized = chunks.iter().filter(|c| c.summary.is_some()).count();
    let total_chunks = chunks.len();

    let mut out = format!(
        "Session: {}\nFile: {}\nEntries: {}\nSize: {} bytes\nStatus: {}\n",
        session.id,
        session.file_name,
        session.total_entries,
        session.total_bytes,
        session.status
    );

    out.push_str(&format!(
        "\n## Analysis coverage\n\
         Chunks: {summarized}/{total_chunks} summarized\n\
         Final report: {}\n",
        if session.final_summary.is_some() {
            "available"
        } else {
            "not generated yet"
        }
    ));

    if summarized == 0 && session.final_summary.is_none() {
        out.push_str(
            "\nNo LLM chunk summaries or final report yet. You can still analyze raw entries with list_entries, \
             get_entry_part, trace_cookies, etc. Mention to the user that running Analyze would add chunk-level context.\n",
        );
    } else if summarized < total_chunks {
        out.push_str(
            "\nPartial chunk analysis — some chunks lack summaries. Use get_chunk_details for coverage per chunk.\n",
        );
    }

    if let Some(summary) = &session.final_summary {
        out.push_str("\n## Final analysis summary\n");
        out.push_str(&llm_body(summary));
    }

    Ok(out)
}

fn tool_get_chunk_summaries(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let chunks = with_db(state, |db| db.get_session_chunks(&session.id))?;
    let with_summary: Vec<_> = chunks.iter().filter(|c| c.summary.is_some()).collect();

    if with_summary.is_empty() {
        return Ok("No chunk summaries available yet.".to_string());
    }

    let filter: Option<Vec<usize>> = args.get("chunk_indices").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|n| n.as_u64().map(|x| x as usize))
                .collect()
        })
    });

    let mut out = String::from("Chunk analysis summaries:\n\n");
    for chunk in with_summary {
        if let Some(ref indices) = filter {
            if !indices.contains(&chunk.chunk_index) {
                continue;
            }
        }

        let entry_indices: Vec<usize> = chunk
            .payload
            .lines()
            .filter_map(|line| {
                line.trim_start()
                    .strip_prefix('[')
                    .and_then(|rest| rest.split(']').next())
                    .and_then(|num| num.parse().ok())
            })
            .collect();
        let index_preview = if entry_indices.is_empty() {
            String::new()
        } else if entry_indices.len() <= 8 {
            format!("entry indices: {}", entry_indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "))
        } else {
            format!(
                "entry indices: {}…{} ({} entries)",
                entry_indices.first().unwrap_or(&0),
                entry_indices.last().unwrap_or(&0),
                entry_indices.len()
            )
        };

        let payload_preview = preview_chars(
            &chunk.payload.lines().take(3).collect::<Vec<_>>().join(" | "),
            200,
        );

        out.push_str(&format!(
            "### Chunk {} ({}, {} entries, status: {})\n",
            chunk.chunk_index + 1,
            chunk.chunk_type,
            chunk.entry_count,
            chunk.status
        ));
        if !index_preview.is_empty() {
            out.push_str(&format!("Coverage: {index_preview}\n"));
        }
        out.push_str(&format!("Payload preview: {payload_preview}\n\n"));
        out.push_str(chunk.summary.as_deref().unwrap_or("(empty)"));
        out.push_str("\n\n");
    }

    if out.trim().is_empty() || out == "Chunk analysis summaries:\n\n" {
        return Ok("No matching chunk summaries.".to_string());
    }

    Ok(out)
}

fn tool_generate_curl(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry index {index} not found"))
    })?;
    let mut spec = http_tools::spec_from_entry(&entry);
    http_tools::apply_request_overrides(&mut spec, args);
    Ok(http_tools::build_curl_from_spec(&spec))
}

pub(super) fn arg_entry_index(args: &Value) -> Result<usize, String> {
    args.get("entry_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| "Missing required parameter: entry_index".to_string())
}

pub(super) fn format_entry_line(e: &HarEntrySummary) -> String {
    format!(
        "[{}] {} {} -> {} ({}, {} bytes, {:.0}ms{})",
        e.index,
        e.method,
        e.url,
        e.status,
        e.mime_type,
        e.size,
        e.time_ms,
        if e.is_javascript { ", JS" } else { "" }
    )
}

