pub mod agent_state;
use agent_state::{ChatAgentState, CHAT_CANCELLED_ERROR};
use crate::db::Database;
use crate::har::js_analyzer::llm_body;
use crate::har::types::{AnalysisSession, AppSettings, HarEntryDetail, HarEntrySummary};
use crate::llm::{self, ChatRequestMessage};
use crate::AppState;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if ChatAgentState::is_cancelled(cancel) {
        Err(CHAT_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

const TOOL_RESULT_MAX_CHARS: usize = 6_000;
const LIST_ENTRIES_MAX: usize = 100;
const EXECUTE_RESPONSE_MAX: usize = 8_000;
pub const DEFAULT_AGENT_MAX_STEPS: usize = 10;
pub const AGENT_MAX_STEPS_CAP: usize = 50;

const FINALIZE_AFTER_LIMIT_PROMPT: &str = "[System] The HAR tool step limit was reached before you could finish. \
Do not call any more tools. Provide the best complete answer you can from the tool results already in this conversation. \
At the end, add a brief **Limit reached** section explaining what you were still trying to look up and why the task needed more tool steps than allowed.";

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
    },
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
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
                        "offset": { "type": "integer", "description": "Pagination offset (default 0)" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_entry",
                "description": "Fetch full details for one HAR entry by index: headers, request/response bodies (truncated), timing.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Zero-based entry index from list_entries" }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_js_analysis",
                "description": "Get JavaScript source excerpt and regex-detected network patterns (fetch, XHR, axios, etc.) for a JS entry.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Index of a JavaScript entry" }
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
                "parameters": { "type": "object", "properties": {} }
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
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "generate_curl",
                "description": "Build an equivalent curl command for a HAR entry (does not execute).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "execute_request",
                "description": "Replay the HAR entry as a live HTTP request (like curl). Use only when testing is needed. Returns status, headers, and response body preview.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "override_url": { "type": "string", "description": "Optional URL override" }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
    ]
}

pub fn execute_tool(
    db: &Database,
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

    let result = match tool_name {
        "list_entries" => tool_list_entries(db, session, &args),
        "get_entry" => tool_get_entry(db, session, &args),
        "get_js_analysis" => tool_get_js_analysis(db, session, &args),
        "get_session_overview" => tool_get_session_overview(session),
        "get_chunk_summaries" => tool_get_chunk_summaries(db, session, &args),
        "generate_curl" => tool_generate_curl(db, session, &args),
        "execute_request" => Err(
            "execute_request is handled asynchronously by the agent runner".to_string(),
        ),
        other => Err(format!("Unknown tool: {other}")),
    }?;

    Ok(truncate_tool_result(&result))
}

pub async fn run_session_agent<F, G>(
    state: &AppState,
    settings: &AppSettings,
    model: &str,
    session: &AnalysisSession,
    mut messages: Vec<ChatRequestMessage>,
    max_steps: usize,
    step_offset: usize,
    thinking_mode: bool,
    cancel: Arc<AtomicBool>,
    mut on_tool: F,
    mut on_stream: G,
) -> Result<AgentRunOutcome, String>
where
    F: FnMut(&str, usize, &str, &str, &str),
    G: FnMut(&str, &str),
{
    let tools = tool_definitions();
    let mut reasoning_accum = String::new();
    let mut steps_used = 0usize;
    let mut tools_executed = 0usize;
    let mut truncation_retries = 0usize;
    let max_output_tokens = llm::agent_max_output_tokens(thinking_mode);

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
        ensure_not_cancelled(&cancel)?;
        steps_used += 1;
        let display_step = step_offset + steps_used;

        let model_label = model.to_string();
        let timeout_secs = llm::AGENT_PLANNING_TIMEOUT_SECS;
        let force_answer = tools_executed >= llm::AGENT_MAX_TOOLS_PER_RUN;

        if force_answer {
            on_tool(
                "agent-planning",
                display_step,
                "agent",
                "thinking",
                &format!(
                    "Step {display_step}: wrapping up after {tools_executed} tool lookups — asking {model_label} for the final answer…"
                ),
            );
            messages.push(llm::ChatRequestMessage::text(
                "user",
                format!(
                    "[System] You have already executed {tools_executed} HAR tool lookups in this reply. \
                     Do not call any more tools. Deliver your complete final answer now \
                     (for this user: a working Python script using requests, based on the endpoints you found)."
                ),
            ));
        } else {
            on_tool(
                "agent-planning",
                display_step,
                "agent",
                "thinking",
                &format!(
                    "Step {display_step}: asking OpenRouter ({model_label}) which tools to call… \
                     ({tools_executed} tool lookups so far)"
                ),
            );
        }
        tokio::task::yield_now().await;

        let settings_for_llm = settings.clone();
        let messages_for_llm = messages.clone();
        let tools_for_llm = if force_answer {
            Vec::new()
        } else {
            tools.clone()
        };
        let mut llm_future = Box::pin(llm::complete_for_agent(
            &settings_for_llm,
            &model_label,
            messages_for_llm,
            tools_for_llm,
            Some(max_output_tokens),
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
                result = &mut llm_future => {
                    break result?;
                }
                _ = cancel_poll.tick() => {
                    ensure_not_cancelled(&cancel)?;
                }
                _ = heartbeat.tick() => {
                    ensure_not_cancelled(&cancel)?;
                    elapsed_secs += 1;
                    let detail = if elapsed_secs >= 20 {
                        format!(
                            "Step {display_step}: still waiting on OpenRouter ({model_label})… \
                             {elapsed_secs}s (times out at {timeout_secs}s). \
                             This is the model deciding which HAR tools to run."
                        )
                    } else {
                        format!(
                            "Step {display_step}: asking OpenRouter ({model_label}) which tools to call… \
                             {elapsed_secs}s"
                        )
                    };
                    on_tool("agent-planning", display_step, "agent", "thinking", &detail);
                }
            }
        };

        llm::enrich_agent_turn(&mut turn);

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
        );

        if !turn.reasoning.is_empty() {
            reasoning_accum = turn.reasoning.clone();
        }

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
                         Use the native function-calling tools (not <tool_call> tags). Keep reasoning brief and call tools promptly.",
                    ));
                    continue;
                }

                return Err(
                    "The model started a tool call in plain text but did not finish it (often caused by thinking models hitting output limits). \
                     Try again without thinking mode, or pick a model with native tool calling support."
                        .to_string(),
                );
            }

            let content = if turn.content.trim().is_empty() && !reasoning_accum.trim().is_empty() {
                reasoning_accum.clone()
            } else {
                turn.content
            };
            llm::emit_simulated_stream(
                &content,
                &reasoning_accum,
                |c, r| on_stream(c, r),
                || ChatAgentState::is_cancelled(&cancel),
            )
            .await?;
            return Ok(AgentRunOutcome::Complete {
                content,
                reasoning: reasoning_accum,
                steps_used,
            });
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
        if calls.len() > llm::AGENT_MAX_TOOLS_PER_STEP {
            calls.truncate(llm::AGENT_MAX_TOOLS_PER_STEP);
        }

        for call in &calls {
            on_tool(
                &call.id,
                display_step,
                &call.function.name,
                "running",
                &call.function.arguments,
            );
        }
        tokio::task::yield_now().await;

        for call in calls {
            ensure_not_cancelled(&cancel)?;
            let tool_name = call.function.name.clone();
            let result =
                execute_session_tool(state, session, &tool_name, &call.function.arguments).await;

            tools_executed += 1;

            let content = match result {
                Ok(text) => {
                    let preview = if text.len() > 120 {
                        format!("{}…", preview_chars(&text, 120))
                    } else {
                        text.clone()
                    };
                    on_tool(&call.id, display_step, &tool_name, "done", &preview);
                    text
                }
                Err(err) => {
                    on_tool(&call.id, display_step, &tool_name, "error", &err);
                    format!("Tool error: {err}")
                }
            };

            messages.push(llm::ChatRequestMessage::tool_result(call.id, content));
            tokio::task::yield_now().await;
        }
    }

    Ok(AgentRunOutcome::StepLimitReached {
        messages,
        reasoning: reasoning_accum,
        steps_used: step_offset + steps_used,
    })
}

pub async fn force_finalize_agent<G>(
    settings: &AppSettings,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    mut reasoning_accum: String,
    cancel: Arc<AtomicBool>,
    mut on_stream: G,
) -> Result<(String, String), String>
where
    G: FnMut(&str, &str),
{
    ensure_not_cancelled(&cancel)?;
    messages.push(llm::ChatRequestMessage::text(
        "user",
        FINALIZE_AFTER_LIMIT_PROMPT,
    ));

    let mut turn = llm::complete_for_agent(
        settings,
        model,
        messages,
        vec![],
        Some(llm::agent_max_output_tokens(false)),
    )
    .await?;

    llm::enrich_agent_turn(&mut turn);

    if !turn.reasoning.is_empty() {
        reasoning_accum = turn.reasoning;
    }

    let content = if turn.content.trim().is_empty() && !reasoning_accum.trim().is_empty() {
        reasoning_accum.clone()
    } else {
        turn.content
    };

    llm::emit_simulated_stream(
        &content,
        &reasoning_accum,
        |c, r| on_stream(c, r),
        || ChatAgentState::is_cancelled(&cancel),
    )
    .await?;

    Ok((content, reasoning_accum))
}

async fn execute_session_tool(
    state: &AppState,
    session: &AnalysisSession,
    tool_name: &str,
    arguments: &str,
) -> Result<String, String> {
    if tool_name == "execute_request" {
        let args: Value = serde_json::from_str(arguments)
            .map_err(|e| format!("Invalid tool arguments JSON: {e}"))?;
        let index = arg_entry_index(&args)?;
        let override_url = args
            .get("override_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let entry = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.get_entry_detail(&session.id, index)?
                .ok_or_else(|| format!("Entry index {index} not found"))?
        };

        return replay_har_entry(&entry, override_url.as_deref()).await;
    }

    let db = state.db.lock().map_err(|e| e.to_string())?;
    execute_tool(&db, session, tool_name, arguments)
}

pub async fn replay_har_entry(
    entry: &HarEntryDetail,
    override_url: Option<&str>,
) -> Result<String, String> {
    let index = entry.summary.index;
    let url = override_url.unwrap_or(&entry.summary.url);
    let curl_preview = build_curl_command(entry);

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let method = reqwest::Method::from_bytes(entry.summary.method.as_bytes())
        .map_err(|_| format!("Unsupported HTTP method: {}", entry.summary.method))?;

    let mut request = client.request(method, url);

    for h in &entry.request_headers {
        let lower = h.name.to_ascii_lowercase();
        if lower == "host"
            || lower == "content-length"
            || lower == "connection"
            || lower == "transfer-encoding"
        {
            continue;
        }
        request = request.header(&h.name, &h.value);
    }

    if !entry.request_body.is_empty()
        && entry.summary.method != "GET"
        && entry.summary.method != "HEAD"
    {
        request = request.body(entry.request_body.clone());
    }

    let started = std::time::Instant::now();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let status = response.status();
    let resp_headers: Vec<String> = response
        .headers()
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v.to_str().unwrap_or("<binary>")))
        .collect();

    let body_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let body_text = String::from_utf8_lossy(&body_bytes);
    let body_preview = if body_text.len() > EXECUTE_RESPONSE_MAX {
        format!(
            "{}\n\n[... response truncated at {EXECUTE_RESPONSE_MAX} chars, total {} bytes ...]",
            preview_chars(&body_text, EXECUTE_RESPONSE_MAX.saturating_sub(80)),
            body_bytes.len()
        )
    } else {
        body_text.into_owned()
    };

    Ok(truncate_tool_result(&format!(
        "Replayed entry [{index}] in {:.0}ms\nEquivalent curl:\n```\n{curl_preview}\n```\n\nLive response:\nHTTP {} {}\n\nResponse headers:\n{}\n\nResponse body preview:\n{}",
        elapsed_ms,
        status.as_u16(),
        status.canonical_reason().unwrap_or(""),
        resp_headers.join("\n"),
        body_preview
    )))
}

fn truncate_tool_result(text: &str) -> String {
    if text.len() <= TOOL_RESULT_MAX_CHARS {
        return text.to_string();
    }
    format!(
        "{}\n\n[... truncated at {TOOL_RESULT_MAX_CHARS} chars — use narrower filters or a specific entry_index ...]",
        preview_chars(text, TOOL_RESULT_MAX_CHARS.saturating_sub(120))
    )
}

fn preview_chars(text: &str, max: usize) -> String {
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn tool_list_entries(db: &Database, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let query = args.get("query").and_then(|v| v.as_str());
    let method = args.get("method").and_then(|v| v.as_str());
    let status_min = args.get("status_min").and_then(|v| v.as_u64()).map(|v| v as u16);
    let status_max = args.get("status_max").and_then(|v| v.as_u64()).map(|v| v as u16);
    let js_only = args.get("js_only").and_then(|v| v.as_bool()).unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(LIST_ENTRIES_MAX as u64) as usize;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

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

fn tool_get_entry(db: &Database, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = db
        .get_entry_detail(&session.id, index)?
        .ok_or_else(|| format!("Entry index {index} not found"))?;
    Ok(format_entry_detail(&entry))
}

fn tool_get_js_analysis(db: &Database, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = db
        .get_entry_detail(&session.id, index)?
        .ok_or_else(|| format!("Entry index {index} not found"))?;

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

    if entry.response_body.is_empty() {
        out.push_str("(No JS source body stored for this entry.)");
    } else {
        out.push_str("Source excerpt:\n```javascript\n");
        out.push_str(&llm_body(&entry.response_body));
        out.push_str("\n```");
    }

    Ok(out)
}

fn tool_get_session_overview(session: &AnalysisSession) -> Result<String, String> {
    let mut out = format!(
        "Session: {}\nFile: {}\nEntries: {}\nSize: {} bytes\nStatus: {}\n",
        session.id,
        session.file_name,
        session.total_entries,
        session.total_bytes,
        session.status
    );

    if let Some(summary) = &session.final_summary {
        out.push_str("\n## Final analysis summary\n");
        out.push_str(&llm_body(summary));
    } else {
        out.push_str("\n(No final report yet — run Analyze / Report first, or inspect raw entries with list_entries/get_entry.)\n");
    }

    Ok(out)
}

fn tool_get_chunk_summaries(db: &Database, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let chunks = db.get_session_chunks(&session.id)?;
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
        out.push_str(&format!(
            "### Chunk {} ({}, {} entries)\n",
            chunk.chunk_index + 1,
            chunk.chunk_type,
            chunk.entry_count
        ));
        out.push_str(chunk.summary.as_deref().unwrap_or("(empty)"));
        out.push_str("\n\n");
    }

    if out.trim().is_empty() || out == "Chunk analysis summaries:\n\n" {
        return Ok("No matching chunk summaries.".to_string());
    }

    Ok(out)
}

fn tool_generate_curl(db: &Database, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = db
        .get_entry_detail(&session.id, index)?
        .ok_or_else(|| format!("Entry index {index} not found"))?;
    Ok(build_curl_command(&entry))
}

fn arg_entry_index(args: &Value) -> Result<usize, String> {
    args.get("entry_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| "Missing required parameter: entry_index".to_string())
}

fn format_entry_line(e: &HarEntrySummary) -> String {
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

fn format_entry_detail(entry: &HarEntryDetail) -> String {
    let s = &entry.summary;
    let mut out = format!(
        "Entry [{}] {} {}\nStatus: {} · MIME: {} · Size: {} bytes · Time: {:.0}ms\n",
        s.index, s.method, s.url, s.status, s.mime_type, s.size, s.time_ms
    );

    if !entry.request_headers.is_empty() {
        out.push_str("\nRequest headers:\n");
        for h in &entry.request_headers {
            out.push_str(&format!("  {}: {}\n", h.name, h.value));
        }
    }

    if !entry.request_body.is_empty() {
        out.push_str("\nRequest body:\n```\n");
        out.push_str(&llm_body(&entry.request_body));
        out.push_str("\n```\n");
    }

    if !entry.response_headers.is_empty() {
        out.push_str("\nResponse headers:\n");
        for h in entry.response_headers.iter().take(40) {
            out.push_str(&format!("  {}: {}\n", h.name, h.value));
        }
    }

    if !entry.response_body.is_empty() {
        out.push_str("\nResponse body (from HAR capture):\n```\n");
        out.push_str(&llm_body(&entry.response_body));
        out.push_str("\n```\n");
    }

    if s.is_javascript && !entry.js_insights.is_empty() {
        out.push_str("\nJS patterns (use get_js_analysis for full source):\n");
        for insight in &entry.js_insights {
            out.push_str(&format!("  - {insight}\n"));
        }
    }

    out
}

fn shell_escape(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn build_curl_command(entry: &HarEntryDetail) -> String {
    let mut parts = vec![format!("curl -X {}", entry.summary.method)];

    for h in &entry.request_headers {
        if h.name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        parts.push(format!(
            "-H '{}: {}'",
            shell_escape(&h.name),
            shell_escape(&h.value)
        ));
    }

    if !entry.request_body.is_empty()
        && entry.summary.method != "GET"
        && entry.summary.method != "HEAD"
    {
        parts.push(format!("-d '{}'", shell_escape(&entry.request_body)));
    }

    parts.push(format!("'{}'", shell_escape(&entry.summary.url)));
    parts.join(" \\\n  ")
}
