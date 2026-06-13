mod agent_limits;
mod context_compact;
mod model_context;
mod model_metadata;
mod model_router;

pub use agent_limits::LIMIT_FIELD_DOCS;
pub use crate::har::types::AgentLimitsSettings;
pub use context_compact::{
    compact_messages_if_needed, prepare_agent_messages, should_summarize_messages, CompactReport,
};
pub use model_context::{
    budget_for_model, budget_for_model_and_settings, ensure_model_context,
    ensure_model_context_for_settings, ContextBudget,
};
pub use model_router::{
    estimate_context_chars, select_agent_model, user_wants_script_from_messages, AgentRoutingContext,
    ModelTier,
};

use crate::har::types::AppSettings;
use futures::StreamExt;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const REQUEST_TIMEOUT_SECS: u64 = 180;
/// Timeout for chat agent tool-planning calls (non-streaming).
pub const AGENT_PLANNING_TIMEOUT_SECS: u64 = 180;
/// Abort agent SSE if the model sends no tokens/tool deltas for this long.
const AGENT_STREAM_IDLE_TIMEOUT_SECS: u64 = 35;
const CHAT_STREAM_IDLE_TIMEOUT_SECS: u64 = 90;
const AGENT_MAX_TOOL_MESSAGES: usize = 24;
pub const AGENT_MAX_TOOLS_PER_STEP: usize = 20;
pub const AGENT_MAX_TOOLS_PER_RUN: usize = 150;
pub const AGENT_TOOL_RUN_LIMIT_BOOST: usize = 150;

pub fn default_tool_run_limit(settings: &AppSettings) -> usize {
    agent_limits::default_tool_run_limit(settings)
}

pub fn boosted_tool_run_limit(settings: &AppSettings, current: usize) -> usize {
    agent_limits::boosted_tool_run_limit(settings, current)
}

pub fn resolve_agent_limits(settings: &AppSettings) -> AgentLimitsSettings {
    agent_limits::resolve(settings)
}
const MAX_RETRIES: u32 = 3;
/// Hard cap on total message chars sent to OpenRouter (~15k tokens at 3 chars/token).
const HARD_MAX_REQUEST_CHARS: usize = 45_000;

#[derive(Debug, serde::Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatRequestMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatRequestMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>, content: Option<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AssistantTurn {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

const AGENT_TOOL_NAMES: &[&str] = &[
    "list_entries",
    "get_entry",
    "get_entry_part",
    "get_js_analysis",
    "get_deobfuscated_js",
    "get_js_snippet",
    "get_session_overview",
    "get_chunk_summaries",
    "get_chunk_details",
    "summarize_entries",
    "trace_cookies",
    "trace_storage",
    "list_js_scripts",
    "get_js_call_map",
    "list_endpoints",
    "search_bodies",
    "compare_entries",
    "get_auth_flow",
    "decode_jwt",
    "generate_curl",
    "list_live_http_requests",
    "get_live_http_request",
    "get_live_auth_state",
    "execute_request",
    "execute_http_request",
    "minimize_http_request",
    "check_python_environment",
    "run_script",
];

pub fn agent_max_output_tokens(thinking_mode: bool) -> u32 {
    if thinking_mode {
        8192
    } else {
        4096
    }
}

fn extract_json_object(s: &str) -> Option<(String, usize)> {
    let s = s.trim_start();
    if !s.starts_with('{') {
        return None;
    }

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let chars: Vec<char> = s.chars().collect();

    for (i, ch) in chars.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if *ch == '\\' {
                escape = true;
            } else if *ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let json_str: String = chars[..=i].iter().collect();
                    return Some((json_str, i + 1));
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_embedded_tool_call(value: &Value) -> Option<ToolCall> {
    let name = value.get("name").and_then(|v| v.as_str())?;
    if !AGENT_TOOL_NAMES.contains(&name) {
        return None;
    }

    let arguments = match value.get("arguments") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    };

    Some(ToolCall {
        id: format!("call_{}", uuid::Uuid::new_v4()),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: name.to_string(),
            arguments: normalize_function_arguments(&arguments),
        },
    })
}

/// OpenRouter / strict providers (e.g. Alibaba Qwen) require `function.arguments` to be a JSON object string.
pub fn normalize_function_arguments(raw: &str) -> String {
    let mut current = raw.trim().to_string();
    if current.is_empty() {
        return "{}".to_string();
    }

    for _ in 0..2 {
        match serde_json::from_str::<Value>(&current) {
            Ok(Value::String(inner)) if !inner.trim().is_empty() => {
                current = inner;
                continue;
            }
            Ok(Value::Object(map)) => {
                return serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
            }
            Ok(_) => return "{}".to_string(),
            Err(_) => break,
        }
    }

    if let Some((json_str, _)) = extract_json_object(&current) {
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&json_str) {
            return serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
        }
    }

    "{}".to_string()
}

pub fn normalize_tool_calls(calls: &mut [ToolCall]) {
    for call in calls.iter_mut() {
        call.function.arguments = normalize_function_arguments(&call.function.arguments);
    }
}

pub fn sanitize_outbound_messages(messages: &mut [ChatRequestMessage]) {
    for msg in messages.iter_mut() {
        if let Some(calls) = msg.tool_calls.as_mut() {
            normalize_tool_calls(calls);
        }
    }
}

fn dsml_invoke_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)<[^>]*DSML[^>]*invoke\s+name="([^"]+)"[^>]*>(.*?)</[^>]*DSML[^>]*invoke>"#)
            .expect("dsml invoke regex")
    })
}

fn dsml_param_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)<[^>]*DSML[^>]*parameter\s+name="([^"]+)"[^>]*>(.*?)</[^>]*DSML[^>]*parameter>"#)
            .expect("dsml param regex")
    })
}

fn dsml_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)<[^>]*DSML[^>]*>.*?</[^>]*DSML[^>]*>"#).expect("dsml block regex")
    })
}

fn contains_dsml_tool_markup(text: &str) -> bool {
    text.contains("DSML") && text.contains("invoke")
}

/// DeepSeek / DSML-style plain-text tool calls some models emit instead of native function calling.
pub fn extract_dsml_tool_calls(text: &str) -> (String, Vec<ToolCall>) {
    let mut calls = Vec::new();
    let invoke_re = dsml_invoke_re();
    let param_re = dsml_param_re();

    for cap in invoke_re.captures_iter(text) {
        let Some(name) = cap.get(1).map(|m| m.as_str()) else {
            continue;
        };
        if !AGENT_TOOL_NAMES.contains(&name) {
            continue;
        }
        let body = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let mut args_map = serde_json::Map::new();
        for pcap in param_re.captures_iter(body) {
            let Some(pname) = pcap.get(1).map(|m| m.as_str()) else {
                continue;
            };
            let pval = pcap.get(2).map(|m| m.as_str()).unwrap_or("");
            args_map.insert(pname.to_string(), Value::String(pval.to_string()));
        }
        calls.push(ToolCall {
            id: format!("call_{}", uuid::Uuid::new_v4()),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: normalize_function_arguments(&Value::Object(args_map).to_string()),
            },
        });
    }

    let mut cleaned = text.to_string();
    if !calls.is_empty() {
        cleaned = dsml_block_re().replace_all(&cleaned, "").into_owned();
    }
    (cleaned.trim().to_string(), calls)
}

pub fn sanitize_agent_visible_text(text: &str) -> String {
    let (without_tool_call, _) = extract_embedded_tool_calls(text);
    let (without_dsml, _) = extract_dsml_tool_calls(&without_tool_call);
    strip_trailing_partial_tool_call(&without_dsml)
}

pub fn extract_embedded_tool_calls(text: &str) -> (String, Vec<ToolCall>) {
    let mut calls = Vec::new();
    let mut cleaned = text.to_string();

    loop {
        let Some(start) = cleaned.find("<tool_call>") else {
            break;
        };

        let after_tag = start + "<tool_call>".len();
        let rest = &cleaned[after_tag..];
        let Some(json_start) = rest.find('{') else {
            break;
        };

        let Some((json_str, json_len)) = extract_json_object(&rest[json_start..]) else {
            break;
        };

        let Ok(value) = serde_json::from_str::<Value>(&json_str) else {
            break;
        };

        let Some(call) = parse_embedded_tool_call(&value) else {
            break;
        };

        calls.push(call);

        let mut block_end = after_tag + json_start + json_len;
        let remainder = cleaned.get(block_end..).unwrap_or("");
        let trim_offset = remainder.len() - remainder.trim_start().len();
        if remainder.trim_start().starts_with("</tool_call>") {
            block_end += trim_offset + "</tool_call>".len();
        }

        cleaned.replace_range(start..block_end, "");
    }

    (cleaned.trim().to_string(), calls)
}

pub fn strip_trailing_partial_tool_call(text: &str) -> String {
    let mut cut = text.len();
    for marker in [
        "<tool_call>",
        "<｜DSML",
        "DSML｜",
        "｜tool_calls",
        "｜invoke",
        "DSML｜｜invoke",
    ] {
        if let Some(idx) = text.find(marker) {
            cut = cut.min(idx);
        }
    }
    if cut < text.len() {
        text[..cut].trim_end().to_string()
    } else {
        text.trim().to_string()
    }
}

pub fn has_incomplete_tool_call(text: &str) -> bool {
    let incomplete_xml =
        text.contains("<tool_call>") && extract_embedded_tool_calls(text).1.is_empty();
    let incomplete_dsml =
        contains_dsml_tool_markup(text) && extract_dsml_tool_calls(text).1.is_empty();
    incomplete_xml || incomplete_dsml
}

pub fn enrich_agent_turn(turn: &mut AssistantTurn) {
    if turn.tool_calls.is_empty() {
        for field in [&mut turn.content, &mut turn.reasoning] {
            let (cleaned, extracted) = extract_embedded_tool_calls(field);
            *field = cleaned;
            if !extracted.is_empty() {
                turn.tool_calls = extracted;
                break;
            }
        }
    }
    if turn.tool_calls.is_empty() {
        for field in [&mut turn.content, &mut turn.reasoning] {
            let (cleaned, extracted) = extract_dsml_tool_calls(field);
            *field = cleaned;
            if !extracted.is_empty() {
                turn.tool_calls = extracted;
                break;
            }
        }
    } else {
        turn.content = sanitize_agent_visible_text(&turn.content);
        turn.reasoning = sanitize_agent_visible_text(&turn.reasoning);
    }

    turn.content = sanitize_agent_visible_text(&turn.content);
    turn.reasoning = sanitize_agent_visible_text(&turn.reasoning);
    turn.tool_calls
        .retain(|c| !c.function.name.trim().is_empty());
    normalize_tool_calls(&mut turn.tool_calls);
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_modality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_tokenizer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_instruct_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_completion: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub capabilities: model_metadata::ModelCapabilities,
}

fn build_http_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

pub fn stream_error_retriable(err: &str) -> bool {
    err.contains("stalled")
        || err.contains("timed out")
        || err.contains("timeout")
        || err.contains("Stream read failed")
        || err.contains("OpenRouter error")
}

async fn next_stream_chunk<S, B>(stream: &mut S, idle_timeout_secs: u64) -> Result<Option<B>, String>
where
    S: futures::Stream<Item = Result<B, reqwest::Error>> + Unpin,
{
    match time::timeout(Duration::from_secs(idle_timeout_secs), stream.next()).await {
        Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        Ok(Some(Err(e))) => Err(format!("Stream read failed: {e}")),
        Ok(None) => Ok(None),
        Err(_) => Err(format!(
            "OpenRouter stream stalled (no data for {idle_timeout_secs}s). \
             Tap Stop and try again, or switch models."
        )),
    }
}

pub(super) fn http_client() -> Result<Client, String> {
    static CLIENT: std::sync::OnceLock<Result<Client, String>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| build_http_client(REQUEST_TIMEOUT_SECS))
        .clone()
}

fn agent_http_client() -> Result<Client, String> {
    static CLIENT: std::sync::OnceLock<Result<Client, String>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| build_http_client(AGENT_PLANNING_TIMEOUT_SECS))
        .clone()
}

pub(super) fn preview_body(body: &str, max: usize) -> String {
    let mut end = max.min(body.len());
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let preview = &body[..end];
    if body.len() > max {
        format!("{preview}…")
    } else {
        preview.to_string()
    }
}

fn extract_text_from_content(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Array(parts) => {
            let mut chunks = Vec::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                } else if let Some(text) = part.as_str() {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
            if chunks.is_empty() {
                None
            } else {
                Some(chunks.join("\n"))
            }
        }
        _ => None,
    }
}

fn format_api_error(error: &Value) -> String {
    if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
        return message.to_string();
    }
    error.to_string()
}

fn extract_assistant_content(payload: &Value) -> Result<String, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!("OpenRouter API error: {}", format_api_error(error)));
    }

    let message = payload
        .pointer("/choices/0/message")
        .ok_or_else(|| "OpenRouter response missing choices[0].message".to_string())?;

    if let Some(content) = message.get("content").and_then(extract_text_from_content) {
        return Ok(content);
    }

    if let Some(reasoning) = message.get("reasoning").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            return Ok(reasoning.to_string());
        }
    }

    if let Some(refusal) = message.get("refusal").and_then(|v| v.as_str()) {
        if !refusal.is_empty() {
            return Ok(refusal.to_string());
        }
    }

    Err(
        "OpenRouter returned no assistant content (content was null, empty, or unsupported format)"
            .to_string(),
    )
}

fn parse_chat_response(body: &str) -> Result<String, String> {
    if body.trim().is_empty() {
        return Err("OpenRouter returned an empty response body".to_string());
    }

    let payload: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "Failed to parse OpenRouter JSON: {e}. Preview: {}",
            preview_body(body, 400)
        )
    })?;

    extract_assistant_content(&payload)
}

pub fn message_content_len(m: &ChatRequestMessage) -> usize {
    m.content.as_ref().map(|s| s.len()).unwrap_or(0)
}

fn truncate_for_synthesis(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    format!(
        "{}\n\n[... truncated for synthesis ...]",
        preview_body(text, max_chars.saturating_sub(40))
    )
}

fn truncate_chat_content(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    format!(
        "{}\n\n[... truncated ...]",
        preview_body(text, max_chars.saturating_sub(20))
    )
}

/// Trim chat/agent payloads without destroying the user's latest question.
fn clamp_chat_messages(messages: &mut [ChatRequestMessage], hard_max_chars: usize) {
    const MIN_LAST_USER_CHARS: usize = 2_000;

    loop {
        let total: usize = messages.iter().map(message_content_len).sum();
        if total <= hard_max_chars {
            return;
        }

        let last_user_idx = messages.iter().rposition(|m| m.role == "user");

        // Prefer shrinking oldest tool results first — they are usually the largest.
        let tool_idx = messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.role == "tool" && message_content_len(m) > 400)
            .map(|(i, _)| i);

        if let Some(idx) = tool_idx {
            if let Some(content) = messages[idx].content.as_mut() {
                let target = (content.len() * 2 / 3).max(400);
                *content = truncate_chat_content(content, target);
            }
            continue;
        }

        // Then trim older assistant/history messages (never system or latest user).
        let trim_idx = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                if *i == 0 {
                    return false;
                }
                if last_user_idx == Some(*i) {
                    return false;
                }
                message_content_len(m) > 400
            })
            .max_by_key(|(_, m)| message_content_len(m))
            .map(|(i, _)| i);

        if let Some(idx) = trim_idx {
            if let Some(content) = messages[idx].content.as_mut() {
                let target = (content.len() * 2 / 3).max(400);
                *content = truncate_chat_content(content, target);
            }
            continue;
        }

        // Last resort: shrink the latest user message but keep a readable minimum.
        if let Some(idx) = last_user_idx {
            let others: usize = messages
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .map(|(_, m)| message_content_len(m))
                .sum();
            let budget = hard_max_chars
                .saturating_sub(others)
                .max(MIN_LAST_USER_CHARS);
            if let Some(content) = messages[idx].content.as_mut() {
                if content.len() > budget {
                    *content = truncate_chat_content(content, budget);
                }
            }
        }
        return;
    }
}

/// Keep recent tool output small so agent steps stay responsive after many lookups.
pub fn prune_agent_messages(messages: &mut [ChatRequestMessage], budget: ContextBudget) {
    let mut tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();

    while tool_indices.len() > budget.max_tool_messages_kept {
        let idx = tool_indices.remove(0);
        if let Some(content) = messages[idx].content.as_mut() {
            *content =
                "[Earlier tool result omitted to save context — use recent tool output above.]"
                    .to_string();
        }
    }

    for message in messages.iter_mut() {
        if message.role == "tool" {
            if let Some(content) = message.content.as_mut() {
                *content = truncate_chat_content(content, budget.tool_result_max_chars);
            }
        }
    }

    clamp_chat_messages(messages, budget.hard_max_chars);
}

fn clamp_messages(messages: &mut [ChatRequestMessage]) {
    let total: usize = messages.iter().map(message_content_len).sum();
    if total <= HARD_MAX_REQUEST_CHARS {
        return;
    }

    if let Some(idx) = messages.iter().rposition(|m| m.role == "user") {
        let others: usize = messages
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, m)| message_content_len(m))
            .sum();
        let budget = HARD_MAX_REQUEST_CHARS.saturating_sub(others);
        if let Some(content) = messages[idx].content.as_mut() {
            *content = truncate_for_synthesis(content, budget);
        }
    }
}

async fn post_chat(
    client: &Client,
    api_key: &str,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    clamp_messages(&mut messages);
    sanitize_outbound_messages(&mut messages);

    let request = ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens,
        stream: None,
        tools: None,
        tool_choice: None,
    };

    let resp = client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("HTTP-Referer", "https://haralyzer.app")
        .header("X-Title", "HARalyzer")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenRouter request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read OpenRouter response body: {e}"))?;

    if !status.is_success() {
        if let Ok(payload) = serde_json::from_str::<Value>(&body) {
            if let Some(error) = payload.get("error") {
                return Err(format!(
                    "OpenRouter error ({status}): {}",
                    format_api_error(error)
                ));
            }
        }
        return Err(format!(
            "OpenRouter error ({status}): {}",
            preview_body(&body, 800)
        ));
    }

    parse_chat_response(&body)
}

pub(super) async fn post_chat_with_retry(
    client: &Client,
    api_key: &str,
    model: &str,
    messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    let mut last_error = String::new();

    for attempt in 0..MAX_RETRIES {
        match post_chat(
            client,
            api_key,
            model,
            messages.clone(),
            max_tokens,
        )
        .await
        {
            Ok(content) => return Ok(content),
            Err(err) => {
                last_error = err.clone();
                let retryable = err.contains("empty response")
                    || err.contains("Failed to read OpenRouter response body")
                    || err.contains("OpenRouter request failed")
                    || err.contains("error decoding")
                    || err.contains("connection")
                    || err.contains("timeout");

                if !retryable || attempt + 1 >= MAX_RETRIES {
                    return Err(err);
                }

                let delay_ms = 1000u64 * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    Err(last_error)
}

fn extract_stream_delta(delta: &Value) -> (String, String) {
    let mut content = String::new();
    let mut reasoning = String::new();

    if let Some(text) = delta.get("content").and_then(extract_text_from_content) {
        content.push_str(&text);
    }
    if let Some(text) = delta
        .get("reasoning")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        reasoning.push_str(text);
    } else if let Some(text) = delta
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        reasoning.push_str(text);
    }

    (content, reasoning)
}

/// Merge model reasoning and pre-tool content for display during agent planning.
pub fn combine_planning_text(reasoning: &str, content: &str) -> String {
    let r = reasoning.trim();
    let c = sanitize_agent_visible_text(content).trim().to_string();
    match (r.is_empty(), c.is_empty()) {
        (true, true) => String::new(),
        (false, false) if r != c => format!("{r}\n\n{c}"),
        (false, _) => r.to_string(),
        (true, false) => c.to_string(),
    }
}

const ANSWER_SECTION_MARKERS: &[&str] = &[
    "### Observed",
    "### Inferred",
    "### Self-check",
    "## Observed",
    "## Summary",
    "### Limit reached",
];

pub fn answer_text_score(text: &str) -> u32 {
    let t = text.trim();
    if t.is_empty() {
        return 0;
    }
    let mut score = 0u32;
    for marker in ANSWER_SECTION_MARKERS {
        if t.contains(marker) {
            score += 100;
        }
    }
    if t.contains("{{script}}") {
        score += 20;
    }
    if t.len() > 800 {
        score += 40;
    } else if t.len() > 200 {
        score += 15;
    }
    score
}

/// Pick the user-visible final answer when models split output across content vs reasoning.
pub fn resolve_final_agent_content(turn: &AssistantTurn) -> String {
    let c = turn.content.trim();
    let r = turn.reasoning.trim();
    let cs = answer_text_score(c);
    let rs = answer_text_score(r);

    if rs > cs {
        return r.to_string();
    }
    if cs > 0 {
        return c.to_string();
    }
    if c.is_empty() {
        return r.to_string();
    }
    if !r.is_empty() && r.len() > c.len() + 100 && (r.contains("### ") || r.contains("## ")) {
        return r.to_string();
    }
    c.to_string()
}

/// Short planning text for the tool-activity panel — never duplicate the full final answer.
pub fn planning_text_for_display(turn: &AssistantTurn, final_content: &str) -> String {
    let fc = final_content.trim();
    if fc.is_empty() {
        return combine_planning_text(&turn.reasoning, &turn.content);
    }

    let strip_answer_prefix = |text: &str| -> String {
        let t = text.trim();
        if t.is_empty() {
            return String::new();
        }
        for marker in ANSWER_SECTION_MARKERS {
            if let Some(idx) = t.find(marker) {
                let planning = t[..idx].trim();
                return if planning.is_empty() {
                    String::new()
                } else {
                    planning.to_string()
                };
            }
        }
        if t == fc || (fc.len() > 300 && t.contains(fc)) {
            return String::new();
        }
        t.to_string()
    };

    let from_reasoning = strip_answer_prefix(turn.reasoning.trim());
    let from_content = strip_answer_prefix(turn.content.trim());

    if !from_reasoning.is_empty() && (from_content.is_empty() || from_reasoning.len() >= from_content.len())
    {
        return from_reasoning;
    }
    if !from_content.is_empty() {
        return from_content;
    }

    let combined = combine_planning_text(&turn.reasoning, &turn.content);
    if combined.trim() == fc {
        String::new()
    } else if fc.len() > 200 && combined.contains(fc) {
        combined.replace(fc, "").trim().to_string()
    } else {
        combined
    }
}

const PREMATURE_STOP_PHRASES: &[&str] = &[
    "let me create",
    "let me build",
    "let me test",
    "let me run",
    "let me write",
    "let me prepare",
    "i'll create",
    "i'll build",
    "i'll write",
    "i will create",
    "i will build",
    "now let me",
    "going to create",
    "going to build",
    "going to run",
    "about to run",
    "i have enough information to build",
    "i have enough information to create",
];

const SCRIPT_REQUEST_PHRASES: &[&str] = &[
    "python script",
    "write a script",
    "make a script",
    "build a script",
    "create a script",
    "run_script",
    "cli script",
    "python cli",
    "prototype script",
    "make a python",
    "build a python",
    "create a python",
];

pub const AGENT_PREMATURE_STOP_NUDGE: &str = "[System] You ended your turn with intent to act (\"let me…\") but called no tools. \
Use the native function-calling API now — call the tool immediately. Do not reply with more planning text.";

pub const AGENT_SCRIPT_DELIVERY_NUDGE: &str = "[System] The user asked for a Python script and you have not called run_script yet. \
Do NOT reply with more planning — call run_script now with code= (full script) using API patterns from the HAR. \
If you need one more HAR lookup, call that tool in this same step; never finish without a native tool call.";

pub const AGENT_SCRIPT_RETRY_NUDGE: &str = "[System] The script prototype is not done yet (run_script has not succeeded). \
Do not stop with \"let me…\" — call run_script with append_code/replacements or fix via HAR tools, then run again.";

pub const AGENT_SCRIPT_FALSE_SUCCESS_NUDGE: &str = "[System] run_script has NOT succeeded — do not claim a working script or demo/mock output. \
Either fix the script (live API, real HAR auth) and run again until exit 0 with real results, \
or stop and tell the user honestly why a working prototype could not be built (504, missing auth, etc.). \
Never ship demo mode, search_demo(), or silent fallbacks to fake data.";

pub fn script_success_pending(
    messages: &[ChatRequestMessage],
    script_run_attempted: bool,
    script_succeeded: bool,
) -> bool {
    user_wants_script_prototype(messages) && script_run_attempted && !script_succeeded
}

pub fn looks_like_false_script_success(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if looks_like_honest_script_failure(&lower) {
        return false;
    }
    [
        "working python",
        "working script",
        "complete solution",
        "here's the complete",
        "here is the complete",
        "created a working",
        "fully working",
        "successfully created",
        "demo mode output",
        "uses har response structure for testing",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
        || (lower.contains("script") && lower.contains("saved as") && !lower.contains("failed"))
}

pub fn looks_like_honest_script_failure_text(text: &str) -> bool {
    looks_like_honest_script_failure(&text.to_ascii_lowercase())
}

fn looks_like_honest_script_failure(lower: &str) -> bool {
    [
        "failed to",
        "could not",
        "cannot create",
        "couldn't create",
        "does not work",
        "did not work",
        "not succeed",
        "unable to",
        "prototype failed",
        "script failed",
        "did not succeed",
        "not available",
        "honest",
        "blocked:",
        "rejected:",
        "stub detected",
        "mock/stub",
        "without success",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

pub fn last_real_user_message(messages: &[ChatRequestMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| {
        if m.role != "user" {
            return None;
        }
        let content = m.content.as_deref()?.trim();
        if content.is_empty() || content.starts_with("[System]") {
            return None;
        }
        Some(content.to_string())
    })
}

pub fn user_wants_script_prototype(messages: &[ChatRequestMessage]) -> bool {
    let Some(text) = last_real_user_message(messages) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    if SCRIPT_REQUEST_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        return true;
    }
    lower.contains("script") && (lower.contains("python") || lower.contains("cli"))
}

pub fn script_delivery_pending(messages: &[ChatRequestMessage], script_run_attempted: bool) -> bool {
    user_wants_script_prototype(messages) && !script_run_attempted
}

fn tail_contains_any(text: &str, phrases: &[&str], tail_len: usize) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let tail = if t.len() > tail_len {
        &t[t.len().saturating_sub(tail_len)..]
    } else {
        t
    };
    let lower = tail.to_ascii_lowercase();
    phrases.iter().any(|phrase| lower.contains(phrase))
}

pub fn looks_like_script_planning_only(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains("```python") || t.contains("{{script}}") {
        return false;
    }
    if answer_text_score(t) >= 100 && t.contains("{{script}}") {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    lower.contains("python script")
        || lower.contains("cli script")
        || lower.contains("run_script")
        || lower.contains("key observations")
        || tail_contains_any(t, PREMATURE_STOP_PHRASES, 600)
        || tail_contains_any(
            t,
            &[
                "let me create the script",
                "create the script now",
                "write and execute a python",
                "now i need to write",
            ],
            800,
        )
}

pub fn looks_like_premature_stop(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if answer_text_score(t) >= 100 {
        return false;
    }
    if t.ends_with(':') || t.ends_with('…') || t.ends_with("...") {
        return true;
    }
    if tail_contains_any(t, PREMATURE_STOP_PHRASES, 600) {
        return true;
    }
    looks_like_script_planning_only(t)
}

/// When the model returns no tool calls but clearly intended to keep working, nudge and continue the agent loop.
pub fn agent_continue_nudge(
    messages: &[ChatRequestMessage],
    turn: &AssistantTurn,
    script_run_attempted: bool,
    script_succeeded: bool,
) -> Option<&'static str> {
    if !turn.tool_calls.is_empty() {
        return None;
    }
    let final_content = resolve_final_agent_content(turn);
    if script_delivery_pending(messages, script_run_attempted) {
        return Some(AGENT_SCRIPT_DELIVERY_NUDGE);
    }
    if script_success_pending(messages, script_run_attempted, script_succeeded) {
        let lower = final_content.to_ascii_lowercase();
        if looks_like_honest_script_failure(&lower) {
            return None;
        }
        if looks_like_false_script_success(&final_content) {
            return Some(AGENT_SCRIPT_FALSE_SUCCESS_NUDGE);
        }
        if looks_like_premature_stop(&final_content) {
            return Some(AGENT_SCRIPT_RETRY_NUDGE);
        }
        if answer_text_score(&final_content) >= 120 {
            return Some(AGENT_SCRIPT_FALSE_SUCCESS_NUDGE);
        }
    }
    if user_wants_script_prototype(messages) && script_run_attempted && looks_like_premature_stop(&final_content) {
        return Some(AGENT_SCRIPT_RETRY_NUDGE);
    }
    if looks_like_premature_stop(&final_content) {
        return Some(AGENT_PREMATURE_STOP_NUDGE);
    }
    None
}

pub fn resolve_chat_model(settings: &AppSettings, thinking_mode: bool) -> String {
    if thinking_mode && !settings.thinking_model.trim().is_empty() {
        settings.thinking_model.clone()
    } else {
        settings.default_model.clone()
    }
}

pub fn format_chat_reply(content: &str, reasoning: &str, thinking_mode: bool) -> String {
    let content = sanitize_agent_visible_text(content);
    let reasoning = sanitize_agent_visible_text(reasoning);
    if thinking_mode && !reasoning.trim().is_empty() {
        if content.trim().is_empty() {
            reasoning.trim().to_string()
        } else {
            format!(
                "### Thinking\n\n{}\n\n---\n\n{}",
                reasoning.trim(),
                content.trim()
            )
        }
    } else if content.trim().is_empty() && !reasoning.trim().is_empty() {
        reasoning.trim().to_string()
    } else {
        content.to_string()
    }
}

pub async fn stream_chat<F>(
    settings: &AppSettings,
    model: &str,
    messages: Vec<ChatRequestMessage>,
    mut on_update: F,
) -> Result<(String, String), String>
where
    F: FnMut(&str, &str),
{
    stream_chat_cancellable(settings, model, messages, None, || false, on_update).await
}

pub async fn stream_chat_cancellable<F, C>(
    settings: &AppSettings,
    model: &str,
    messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
    should_cancel: C,
    mut on_update: F,
) -> Result<(String, String), String>
where
    F: FnMut(&str, &str),
    C: Fn() -> bool,
{
    if settings.openrouter_api_key.is_empty() {
        return Err("OpenRouter API key is not configured".to_string());
    }

    let mut messages = messages;
    let budget = ensure_model_context(&settings.openrouter_api_key, model).await;
    compact_messages_if_needed(settings, model, &mut messages).await?;
    clamp_chat_messages(&mut messages, budget.hard_max_chars);
    sanitize_outbound_messages(&mut messages);

    let client = http_client()?;

    let request = ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens,
        stream: Some(true),
        tools: None,
        tool_choice: None,
    };

    let resp = client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {}", settings.openrouter_api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("HTTP-Referer", "https://haralyzer.app")
        .header("X-Title", "HARalyzer")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenRouter request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read body>"));
        if let Ok(payload) = serde_json::from_str::<Value>(&body) {
            if let Some(error) = payload.get("error") {
                return Err(format!(
                    "OpenRouter error ({status}): {}",
                    format_api_error(error)
                ));
            }
        }
        return Err(format!(
            "OpenRouter error ({status}): {}",
            preview_body(&body, 800)
        ));
    }

    let mut byte_stream = resp.bytes_stream();
    let mut sse_buffer = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();

    loop {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }

        let chunk = match next_stream_chunk(&mut byte_stream, CHAT_STREAM_IDLE_TIMEOUT_SECS).await? {
            Some(chunk) => chunk,
            None => break,
        };
        sse_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = sse_buffer.find('\n') {
            let line = sse_buffer[..pos].trim().to_string();
            sse_buffer.drain(..=pos);

            if !line.starts_with("data:") {
                continue;
            }

            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }

            let payload: Value = serde_json::from_str(data).map_err(|e| {
                format!(
                    "Failed to parse stream chunk: {e}. Preview: {}",
                    preview_body(data, 200)
                )
            })?;

            if let Some(error) = payload.get("error") {
                return Err(format!("OpenRouter API error: {}", format_api_error(error)));
            }

            if let Some(delta) = payload.pointer("/choices/0/delta") {
                let (content_delta, reasoning_delta) = extract_stream_delta(delta);
                if !content_delta.is_empty() {
                    content.push_str(&content_delta);
                }
                if !reasoning_delta.is_empty() {
                    reasoning.push_str(&reasoning_delta);
                }
                if !content_delta.is_empty() || !reasoning_delta.is_empty() {
                    on_update(&content, &reasoning);
                }
            }
        }
    }

    Ok((content, reasoning))
}

pub async fn list_models(api_key: &str) -> Result<Vec<OpenRouterModel>, String> {
    if api_key.is_empty() {
        return Ok(default_models());
    }

    let fetched = fetch_models_raw(api_key).await.unwrap_or_else(|_| default_models());
    model_context::cache_model_contexts(&fetched);
    Ok(merge_with_defaults(fetched))
}

pub(crate) async fn fetch_models_raw(api_key: &str) -> Result<Vec<OpenRouterModel>, String> {
    let client = http_client()?;
    let resp = client
        .get(MODELS_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OpenRouter models HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read models response: {e}"))?;

    let parsed: model_metadata::ModelsResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse models: {e}"))?;

    Ok(parsed
        .data
        .into_iter()
        .map(model_metadata::map_model_data)
        .collect())
}

fn merge_with_defaults(mut models: Vec<OpenRouterModel>) -> Vec<OpenRouterModel> {
    let defaults = default_models();
    let mut result = defaults;
    for model in models.drain(..) {
        if !result.iter().any(|m| m.id == model.id) {
            result.push(model);
        }
    }
    result
}

fn default_models() -> Vec<OpenRouterModel> {
    let mk = |id: &str, name: &str, ctx: u32| {
        let capabilities =
            model_metadata::infer_capabilities(id, name, &None, Some(ctx));
        OpenRouterModel {
            id: id.to_string(),
            name: name.to_string(),
            context_length: Some(ctx),
            description: None,
            architecture_modality: None,
            architecture_tokenizer: None,
            architecture_instruct_type: None,
            pricing_prompt: None,
            pricing_completion: None,
            supported_parameters: vec![],
            capabilities,
        }
    };
    vec![
        mk("openai/gpt-4o-mini", "GPT-4o Mini", 128_000),
        mk("openai/gpt-4o", "GPT-4o", 128_000),
        mk("anthropic/claude-3.5-sonnet", "Claude 3.5 Sonnet", 200_000),
        mk("deepseek/deepseek-chat", "DeepSeek Chat", 64_000),
        mk("google/gemini-flash-1.5", "Gemini Flash 1.5", 1_000_000),
    ]
}

pub async fn complete(
    settings: &AppSettings,
    messages: Vec<ChatRequestMessage>,
) -> Result<String, String> {
    complete_with_limit(settings, messages, None).await
}

pub async fn complete_with_limit(
    settings: &AppSettings,
    messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    if settings.openrouter_api_key.is_empty() {
        return Err("OpenRouter API key is not configured".to_string());
    }

    let client = http_client()?;
    let api_model = settings.resolve_api_model(&settings.default_model);
    post_chat_with_retry(
        &client,
        &settings.openrouter_api_key,
        &api_model,
        messages,
        max_tokens,
    )
    .await
}

pub async fn analyze_chunk(
    settings: &AppSettings,
    system_prompt: &str,
    user_content: &str,
) -> Result<String, String> {
    analyze_chunk_with_limit(settings, system_prompt, user_content, None).await
}

async fn analyze_chunk_with_limit(
    settings: &AppSettings,
    system_prompt: &str,
    user_content: &str,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    complete_with_limit(
        settings,
        vec![
            ChatRequestMessage::text("system", system_prompt),
            ChatRequestMessage::text("user", user_content),
        ],
        max_tokens,
    )
    .await
}

pub const CHUNK_TRAFFIC_PROMPT: &str = "You are a network traffic analyst. Analyze the following HAR (HTTP Archive) request entries. Summarize: key endpoints, authentication patterns, errors (4xx/5xx), slow requests, API structure, request/response payloads, and anything unusual. Use markdown with bullet points and code blocks where helpful.";

pub const CHUNK_JS_PROMPT: &str = "You are a JavaScript and network security analyst. Analyze the following JavaScript sources extracted from a HAR file. Identify ALL fetch/XHR/axios/WebSocket calls, their URLs, payloads, auth tokens, API patterns, and business logic tied to network requests. Map JS functions to the endpoints they call. Use markdown with bullet points and ```javascript code blocks.";

pub const FINAL_SYSTEM_PROMPT: &str = "You are a network traffic analyst. You will receive a consolidated summary of HAR file analysis chunks. Write a comprehensive final report covering: overview, domains/endpoints, JS-to-API mapping, auth & security observations, errors & failures, performance issues, and recommendations. Output the report as plain markdown in your reply body: use headings (##), bullet lists, and inline `code` where helpful. Do NOT wrap the entire report in a ``` code fence. Do NOT add a preamble, introduction, or meta commentary — begin immediately with the first heading.";

pub const INTERMEDIATE_SYNTHESIS_PROMPT: &str = "You are a network traffic analyst. Merge the following HAR chunk analysis summaries into one consolidated summary. Preserve key endpoints, auth patterns, errors, JS-to-API mappings, and notable findings. Be concise — respond in at most 1200 words. Reply with plain markdown bullet points only; do not wrap the response in a code fence or add a preamble.";

fn preamble_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?is)^(?:here(?:'s| is)|below is|sure[,!]?|certainly[,!]?)[^\n#*`]{0,220}(?:report|markdown|synthesized|summary)[^\n]*:\s*\n*",
        )
        .expect("preamble regex")
    })
}

fn outer_fence_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)^```(?:markdown|md|text)?\s*\n(.*)\n```\s*$").expect("outer fence regex")
    })
}

/// Strip LLM preambles and unwrap whole-response ```markdown fences so the UI renders markdown.
pub fn normalize_markdown_report(text: &str) -> String {
    let mut s = text.trim().to_string();
    if s.is_empty() {
        return s;
    }

    s = preamble_regex().replace(&s, "").into_owned();
    s = s.trim().to_string();

    if let Some(line_end) = s.find('\n') {
        let first = s[..line_end].trim();
        if first.ends_with(':')
            && first.len() < 160
            && !first.starts_with('#')
            && first.to_ascii_lowercase().contains("report")
        {
            s = s[line_end + 1..].trim().to_string();
        }
    }

    if let Some(caps) = outer_fence_regex().captures(&s) {
        s = caps.get(1).unwrap().as_str().trim().to_string();
    }

    s
}

fn js_fence_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)^```(?:javascript|js|typescript|ts)?\s*\n(.*)\n```\s*$")
            .expect("js fence regex")
    })
}

/// Strip markdown fences and preambles from LLM deobfuscation output.
pub fn normalize_javascript_output(text: &str) -> String {
    let mut s = text.trim().to_string();
    if s.is_empty() {
        return s;
    }

    if let Some(caps) = js_fence_regex().captures(&s) {
        s = caps.get(1).unwrap().as_str().trim().to_string();
    } else if s.starts_with("```") {
        if let Some(end) = s.rfind("```") {
            let inner = s
                .trim_start_matches('`')
                .trim_start_matches(|c: char| c.is_alphabetic() || c == '\n')
                .trim();
            if end > 0 {
                s = inner.trim_end_matches('`').trim().to_string();
            }
        }
    }

    s
}

pub const DEOBFUSCATE_JS_SYSTEM: &str = "You are an expert JavaScript reverse engineer. \
Deobfuscate minified or obfuscated JavaScript captured in a HAR file.\n\n\
Output ONLY valid JavaScript source code — no markdown fences, no prose before or after.\n\n\
Requirements:\n\
- Restore readable formatting with consistent indentation\n\
- Rename variables and functions to descriptive names when meaning can be inferred\n\
- Add concise // comments explaining non-obvious logic, crypto, encoding, or network calls\n\
- Where behavior is unclear, add // UNCLEAR: ... comments rather than guessing\n\
- Preserve runtime behavior; do not invent APIs or endpoints not present in the source\n\
- If the input was truncated, note that in a top comment\n\
- Keep license banners or sourceURL comments if present";

const DEOBFUSCATE_MAX_OUTPUT_TOKENS: u32 = 16384;
const DEOBFUSCATE_CHUNK_CHARS: usize = 80_000;
const DEOBFUSCATE_CHUNK_OVERLAP: usize = 800;

fn split_js_for_deobfuscation(source: &str) -> Vec<String> {
    if source.len() <= DEOBFUSCATE_CHUNK_CHARS {
        return vec![source.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < source.len() {
        let mut end = (start + DEOBFUSCATE_CHUNK_CHARS).min(source.len());
        while end > start && !source.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(source[start..end].to_string());
        if end >= source.len() {
            break;
        }
        start = end.saturating_sub(DEOBFUSCATE_CHUNK_OVERLAP);
    }
    chunks
}

async fn deobfuscate_js_chunk<F>(
    settings: &AppSettings,
    model: &str,
    chunk: &str,
    chunk_index: usize,
    chunk_total: usize,
    mut on_update: F,
) -> Result<String, String>
where
    F: FnMut(&str),
{
    let header = if chunk_total > 1 {
        format!("// Part {} of {chunk_total}\n", chunk_index + 1)
    } else {
        String::new()
    };
    let user_message = if chunk_total > 1 {
        format!(
            "Deobfuscate this JavaScript segment (part {} of {}). \
             Output ONLY code for this segment — no markdown fences.\n\n```javascript\n{chunk}\n```",
            chunk_index + 1,
            chunk_total
        )
    } else {
        format!(
            "Deobfuscate and annotate this JavaScript:\n\n```javascript\n{chunk}\n```"
        )
    };

    let messages = vec![
        ChatRequestMessage::text("system", DEOBFUSCATE_JS_SYSTEM),
        ChatRequestMessage::text("user", user_message),
    ];

    let (content, _reasoning) = stream_chat_cancellable(
        settings,
        model,
        messages,
        Some(DEOBFUSCATE_MAX_OUTPUT_TOKENS),
        || false,
        |partial, _| on_update(partial),
    )
    .await?;

    Ok(format!("{header}{}", normalize_javascript_output(&content)))
}

pub async fn deobfuscate_javascript(
    settings: &AppSettings,
    model: &str,
    source: &str,
) -> Result<String, String> {
    use crate::har::js_analyzer::llm_body;

    let body = llm_body(source);
    let chunks = split_js_for_deobfuscation(&body);
    let mut combined = String::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let part = deobfuscate_js_chunk(settings, model, chunk, i, chunks.len(), |_| {}).await?;
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&part);
    }

    if combined.trim().is_empty() {
        return Err("Deobfuscation returned empty output".to_string());
    }
    Ok(combined)
}

pub async fn stream_deobfuscate_js<F>(
    settings: &AppSettings,
    model: &str,
    source: &str,
    mut on_update: F,
) -> Result<String, String>
where
    F: FnMut(&str),
{
    use crate::har::js_analyzer::llm_body;

    let body = llm_body(source);
    let chunks = split_js_for_deobfuscation(&body);
    let mut combined = String::new();

    for (i, chunk) in chunks.iter().enumerate() {
        if chunks.len() > 1 {
            on_update(&format!(
                "// Deobfuscating part {} of {}…\n",
                i + 1,
                chunks.len()
            ));
        }
        let part = deobfuscate_js_chunk(
            settings,
            model,
            chunk,
            i,
            chunks.len(),
            |partial| {
                let preview = if combined.is_empty() {
                    partial.to_string()
                } else {
                    format!("{combined}\n{partial}")
                };
                on_update(&preview);
            },
        )
        .await?;
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&part);
        on_update(&combined);
    }

    if combined.trim().is_empty() {
        return Err("Deobfuscation returned empty output".to_string());
    }
    Ok(combined)
}

/// Per-chunk excerpt used when building synthesis batches (full text stays in DB).
const SUMMARY_EXCERPT_CHARS: usize = 1_800;
const MERGED_EXCERPT_CHARS: usize = 3_500;
pub const SYNTH_BATCH_SIZE: usize = 10;
const INTERMEDIATE_MAX_OUTPUT_TOKENS: u32 = 2_048;
const FINAL_MAX_OUTPUT_TOKENS: u32 = 4_096;

#[derive(Debug, Clone)]
pub struct SynthesisProgressUpdate {
    pub completed_calls: usize,
    pub total_calls: usize,
    pub round: usize,
    pub batches_in_round: usize,
    pub batch_index: usize,
}

pub fn count_synthesis_calls(summary_count: usize) -> usize {
    if summary_count == 0 {
        return 0;
    }
    let mut layer = summary_count;
    let mut total = 0usize;
    while layer > 1 {
        let batches = layer.div_ceil(SYNTH_BATCH_SIZE);
        total += batches;
        layer = batches;
    }
    total + 1
}

async fn call_synthesis(
    settings: &AppSettings,
    system: &str,
    intro: &str,
    items: &[String],
    max_output_tokens: u32,
) -> Result<String, String> {
    if items.is_empty() {
        return Err("Synthesis batch is empty".to_string());
    }
    if items.len() > SYNTH_BATCH_SIZE {
        return Err(format!(
            "Internal error: synthesis batch has {} items (max {SYNTH_BATCH_SIZE})",
            items.len()
        ));
    }

    let clipped: Vec<String> = items
        .iter()
        .map(|item| truncate_for_synthesis(item, SUMMARY_EXCERPT_CHARS))
        .collect();
    let user = format!("{intro}:\n\n{}", clipped.join("\n\n"));

    let raw = analyze_chunk_with_limit(settings, system, &user, Some(max_output_tokens)).await?;
    Ok(normalize_markdown_report(&raw))
}

pub async fn synthesize_final_report<F>(
    settings: &AppSettings,
    summaries: &[(usize, String)],
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(SynthesisProgressUpdate),
{
    if summaries.is_empty() {
        return Err("No chunk summaries available for final report".to_string());
    }

    let mut ordered = summaries.to_vec();
    ordered.sort_by_key(|(i, _)| *i);

    let mut layer: Vec<String> = ordered
        .iter()
        .map(|(i, s)| {
            format!(
                "### Chunk {} Summary\n{}",
                i + 1,
                truncate_for_synthesis(s, SUMMARY_EXCERPT_CHARS)
            )
        })
        .collect();

    let total_calls = count_synthesis_calls(layer.len());
    let mut completed_calls = 0usize;

    let emit_progress = |completed_calls: usize,
                         round: usize,
                         batches_in_round: usize,
                         batch_index: usize,
                         on_progress: &mut F| {
        on_progress(SynthesisProgressUpdate {
            completed_calls,
            total_calls,
            round,
            batches_in_round,
            batch_index,
        });
    };

    emit_progress(0, 0, 0, 0, &mut on_progress);

    let mut round = 0usize;
    while layer.len() > 1 {
        round += 1;
        let batches_in_round = layer.len().div_ceil(SYNTH_BATCH_SIZE);
        let mut next_layer = Vec::new();

        for (batch_idx, batch) in layer.chunks(SYNTH_BATCH_SIZE).enumerate() {
            let intro = format!(
                "Merge these HAR analysis summaries (consolidation round {round}, part {})",
                batch_idx + 1
            );
            let merged = call_synthesis(
                settings,
                INTERMEDIATE_SYNTHESIS_PROMPT,
                &intro,
                batch,
                INTERMEDIATE_MAX_OUTPUT_TOKENS,
            )
            .await?;
            completed_calls += 1;
            emit_progress(
                completed_calls,
                round,
                batches_in_round,
                batch_idx + 1,
                &mut on_progress,
            );
            next_layer.push(truncate_for_synthesis(&merged, MERGED_EXCERPT_CHARS));
        }

        layer = next_layer;
    }

    let merged = call_synthesis(
        settings,
        FINAL_SYSTEM_PROMPT,
        "Synthesize this consolidated HAR analysis into a final report",
        &layer,
        FINAL_MAX_OUTPUT_TOKENS,
    )
    .await?;
    completed_calls += 1;
    emit_progress(
        completed_calls,
        round + 1,
        1,
        1,
        &mut on_progress,
    );

    Ok(merged)
}

fn parse_assistant_turn(payload: &Value) -> Result<AssistantTurn, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!("OpenRouter API error: {}", format_api_error(error)));
    }

    let message = payload
        .pointer("/choices/0/message")
        .ok_or_else(|| "OpenRouter response missing choices[0].message".to_string())?;

    let content = message
        .get("content")
        .and_then(extract_text_from_content)
        .unwrap_or_default();

    let reasoning = message
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let tool_calls: Vec<ToolCall> = message
        .get("tool_calls")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let finish_reason = payload
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(AssistantTurn {
        content,
        reasoning,
        tool_calls,
        finish_reason,
    })
}

async fn post_chat_completion_with_retry(
    client: &Client,
    api_key: &str,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
    tools: Option<Vec<Value>>,
) -> Result<AssistantTurn, String> {
    let mut last_error = String::new();

    for attempt in 0..MAX_RETRIES {
        match post_chat_completion(
            client,
            api_key,
            model,
            messages.clone(),
            max_tokens,
            tools.clone(),
        )
        .await
        {
            Ok(turn) => return Ok(turn),
            Err(err) => {
                last_error = err.clone();
                let retryable = err.contains("empty response")
                    || err.contains("Failed to read OpenRouter response body")
                    || err.contains("OpenRouter request failed")
                    || err.contains("error decoding")
                    || err.contains("connection")
                    || err.contains("timeout");

                if !retryable || attempt + 1 >= MAX_RETRIES {
                    return Err(err);
                }

                let delay_ms = 1000u64 * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    Err(last_error)
}

async fn post_chat_completion(
    client: &Client,
    api_key: &str,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
    tools: Option<Vec<Value>>,
) -> Result<AssistantTurn, String> {
    sanitize_outbound_messages(&mut messages);

    let request = ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens,
        stream: None,
        tools: tools.clone(),
        tool_choice: if tools.is_some() {
            Some("auto".to_string())
        } else {
            None
        },
    };

    let resp = client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("HTTP-Referer", "https://haralyzer.app")
        .header("X-Title", "HARalyzer")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenRouter request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read OpenRouter response body: {e}"))?;

    if !status.is_success() {
        if let Ok(payload) = serde_json::from_str::<Value>(&body) {
            if let Some(error) = payload.get("error") {
                return Err(format!(
                    "OpenRouter error ({status}): {}",
                    format_api_error(error)
                ));
            }
        }
        return Err(format!(
            "OpenRouter error ({status}): {}",
            preview_body(&body, 800)
        ));
    }

    let payload: Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "Failed to parse OpenRouter JSON: {e}. Preview: {}",
            preview_body(&body, 400)
        )
    })?;

    parse_assistant_turn(&payload)
}

#[derive(Default)]
struct StreamingToolAccumulator {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn merge_tool_call_delta(accumulators: &mut Vec<StreamingToolAccumulator>, delta: &Value) {
    let Some(items) = delta.get("tool_calls").and_then(|v| v.as_array()) else {
        return;
    };

    for item in items {
        let index = item
            .get("index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        while accumulators.len() <= index {
            accumulators.push(StreamingToolAccumulator::default());
        }
        let slot = &mut accumulators[index];
        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
            slot.id = Some(id.to_string());
        }
        if let Some(name) = item.pointer("/function/name").and_then(|v| v.as_str()) {
            slot.name.push_str(name);
        }
        if let Some(args) = item.pointer("/function/arguments").and_then(|v| v.as_str()) {
            slot.arguments.push_str(args);
        }
    }
}

fn streaming_tool_calls_to_vec(accumulators: Vec<StreamingToolAccumulator>) -> Vec<ToolCall> {
    accumulators
        .into_iter()
        .enumerate()
        .filter(|(_, slot)| !slot.name.is_empty() || slot.id.is_some())
        .map(|(index, slot)| ToolCall {
            id: slot
                .id
                .unwrap_or_else(|| format!("call_stream_{index}_{}", uuid::Uuid::new_v4())),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: slot.name,
                arguments: normalize_function_arguments(&slot.arguments),
            },
        })
        .collect()
}

async fn stream_agent_completion<C, U>(
    client: &Client,
    api_key: &str,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
    tools: Option<Vec<Value>>,
    should_cancel: C,
    mut on_update: U,
) -> Result<AssistantTurn, String>
where
    C: Fn() -> bool,
    U: FnMut(&str, &str),
{
    sanitize_outbound_messages(&mut messages);

    let request = ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens,
        stream: Some(true),
        tools: tools.clone(),
        tool_choice: if tools.is_some() {
            Some("auto".to_string())
        } else {
            None
        },
    };

    let resp = client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("HTTP-Referer", "https://haralyzer.app")
        .header("X-Title", "HARalyzer")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenRouter request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read body>"));
        if let Ok(payload) = serde_json::from_str::<Value>(&body) {
            if let Some(error) = payload.get("error") {
                return Err(format!(
                    "OpenRouter error ({status}): {}",
                    format_api_error(error)
                ));
            }
        }
        return Err(format!(
            "OpenRouter error ({status}): {}",
            preview_body(&body, 800)
        ));
    }

    let mut byte_stream = resp.bytes_stream();
    let mut sse_buffer = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_accumulators: Vec<StreamingToolAccumulator> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let started = std::time::Instant::now();
    let mut last_meaningful = started;

    loop {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }
        if started.elapsed() >= Duration::from_secs(AGENT_PLANNING_TIMEOUT_SECS) {
            return Err(format!(
                "OpenRouter stream exceeded {}s total planning time.",
                AGENT_PLANNING_TIMEOUT_SECS
            ));
        }
        if last_meaningful.elapsed() >= Duration::from_secs(AGENT_STREAM_IDLE_TIMEOUT_SECS) {
            return Err(format!(
                "OpenRouter stream stalled (no model output for {AGENT_STREAM_IDLE_TIMEOUT_SECS}s). \
                 Tap Stop and try again, or switch models."
            ));
        }

        let chunk = match time::timeout(Duration::from_secs(1), byte_stream.next()).await {
            Ok(Some(Ok(chunk))) => Some(chunk),
            Ok(Some(Err(e))) => return Err(format!("Stream read failed: {e}")),
            Ok(None) => None,
            Err(_) => continue,
        };
        let Some(chunk) = chunk else { break };

        sse_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = sse_buffer.find('\n') {
            let line = sse_buffer[..pos].trim().to_string();
            sse_buffer.drain(..=pos);

            if !line.starts_with("data:") {
                continue;
            }

            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() || data == "[DONE]" {
                last_meaningful = std::time::Instant::now();
                continue;
            }

            let payload: Value = serde_json::from_str(data).map_err(|e| {
                format!(
                    "Failed to parse stream chunk: {e}. Preview: {}",
                    preview_body(data, 200)
                )
            })?;

            if let Some(error) = payload.get("error") {
                return Err(format!("OpenRouter API error: {}", format_api_error(error)));
            }

            if let Some(reason) = payload.pointer("/choices/0/finish_reason").and_then(|v| v.as_str())
            {
                finish_reason = Some(reason.to_string());
                last_meaningful = std::time::Instant::now();
            }

            if let Some(delta) = payload.pointer("/choices/0/delta") {
                merge_tool_call_delta(&mut tool_accumulators, delta);
                let (content_delta, reasoning_delta) = extract_stream_delta(delta);
                if !content_delta.is_empty() {
                    content.push_str(&content_delta);
                }
                if !reasoning_delta.is_empty() {
                    reasoning.push_str(&reasoning_delta);
                }
                if !content_delta.is_empty() || !reasoning_delta.is_empty() {
                    last_meaningful = std::time::Instant::now();
                    on_update(&content, &reasoning);
                }
            }

            if matches!(finish_reason.as_deref(), Some("tool_calls") | Some("stop")) {
                break;
            }
        }

        if matches!(finish_reason.as_deref(), Some("tool_calls") | Some("stop")) {
            break;
        }
    }

    let tool_calls = streaming_tool_calls_to_vec(tool_accumulators);
    Ok(AssistantTurn {
        content,
        reasoning,
        tool_calls,
        finish_reason,
    })
}

pub async fn complete_for_agent_streaming<C, U>(
    settings: &AppSettings,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    tools: Vec<Value>,
    max_tokens: Option<u32>,
    should_cancel: C,
    mut on_update: U,
) -> Result<AssistantTurn, String>
where
    C: Fn() -> bool,
    U: FnMut(&str, &str),
{
    if settings.openrouter_api_key.is_empty() {
        return Err("OpenRouter API key is not configured".to_string());
    }

    prepare_agent_messages(settings, model, &mut messages).await?;

    let client = agent_http_client()?;
    let tools_opt = if tools.is_empty() {
        None
    } else {
        Some(tools.clone())
    };

    let messages_for_stream = messages.clone();
    match stream_agent_completion(
        &client,
        &settings.openrouter_api_key,
        model,
        messages_for_stream,
        max_tokens,
        tools_opt.clone(),
        should_cancel,
        |content, reasoning| on_update(content, reasoning),
    )
    .await
    {
        Ok(turn) => Ok(turn),
        Err(err) if stream_error_retriable(&err) => {
            on_update(
                "",
                "OpenRouter stream stalled — retrying tool planning once without streaming \
                 (this is often faster and more reliable)…",
            );
            complete_for_agent(settings, model, messages, tools, max_tokens).await
        }
        Err(err) => Err(err),
    }
}

pub async fn complete_for_agent(
    settings: &AppSettings,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    tools: Vec<Value>,
    max_tokens: Option<u32>,
) -> Result<AssistantTurn, String> {
    if settings.openrouter_api_key.is_empty() {
        return Err("OpenRouter API key is not configured".to_string());
    }

    prepare_agent_messages(settings, model, &mut messages).await?;

    let client = agent_http_client()?;
    let tools_opt = if tools.is_empty() {
        None
    } else {
        Some(tools)
    };

    post_chat_completion_with_retry(
        &client,
        &settings.openrouter_api_key,
        model,
        messages,
        max_tokens,
        tools_opt,
    )
    .await
}

pub async fn complete_with_tools(
    settings: &AppSettings,
    model: &str,
    messages: Vec<ChatRequestMessage>,
    tools: Vec<Value>,
    max_tokens: Option<u32>,
) -> Result<AssistantTurn, String> {
    complete_for_agent(settings, model, messages, tools, max_tokens).await
}

pub async fn emit_simulated_stream(
    content: &str,
    reasoning: &str,
    mut on_update: impl FnMut(&str, &str),
    should_cancel: impl Fn() -> bool,
) -> Result<(), String> {
    if content.is_empty() {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }
        on_update(content, reasoning);
        return Ok(());
    }

    let chars: Vec<char> = content.chars().collect();
    let step = 64usize;
    for end in (step..=chars.len()).step_by(step) {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }
        let partial: String = chars[..end].iter().collect();
        on_update(&partial, reasoning);
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    if chars.len() % step != 0 || chars.is_empty() {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }
        on_update(content, reasoning);
    }
    Ok(())
}

pub const CHAT_SYSTEM_PROMPT: &str = "You are HARalyzer, an expert assistant for analyzing HAR (HTTP Archive) files.\n\n\
CRITICAL: You MUST use the provided tools to look up real session data before stating facts about URLs, status codes, headers, bodies, JavaScript, or API behavior. Never invent or guess HAR data.\n\
Tool calling: use ONLY the native function-calling API. Never emit <tool_call> XML, DSML invoke markup, or \"Now let me run this\" pseudo-tool blocks in your visible reply — call run_script / other tools directly. \
Never end a turn with \"Let me create/test…\" without actually calling tools in that same step.\n\n\
**Knowledge Management** — CRITICAL FOR EFFICIENCY:\n\
- You have a persistent knowledge tree that survives across tool calls and conversation turns.\n\
- Use `get_knowledge` to retrieve accumulated insights about this HAR session.\n\
- Use `update_knowledge` to store important discoveries IMMEDIATELY when you find them.\n\
- ALWAYS record: authentication flows (JWT structure, refresh patterns), API endpoints (base URL, pagination params, rate limits), error patterns (401/403 causes, retry logic), data schemas (key fields, relationships), and any other reusable insights.\n\
- Before starting a new task, call `get_knowledge` first to avoid re-discovering facts.\n\
- Current knowledge snapshot:\n{KNOWLEDGE_TREE}\n\n\
**Task Planning** — For complex multi-step work:\n\
- Break complex tasks into a to-do list using `set_todo_list(items=[...])`.\n\
- Each item needs: title (what to do), status (pending/in_progress/done/blocked), notes (discoveries or blockers).\n\
- Use `update_todo_item(index, status, notes)` to mark progress as you work.\n\
- Work through items systematically — pick the highest-priority unfinished item next.\n\
- If you get stuck on an item, mark it blocked with notes explaining why, then move to the next item.\n\
- For simple single-step tasks, skip the to-do list and just execute.\n\n\
Workflow:\n\
1. Start with get_session_overview or list_entries to orient yourself. Note whether chunk summaries / final report exist.\n\
2. Prefer targeted tools over full dumps:\n\
   - get_entry returns short body previews by default — use get_entry_part(mode=summary) for JSON/HTML structure, mode=full for one body, or get_entry(detail=full) for both bodies. Limits scale with the model context window.\n\
   - Pass max_output_chars on run_script, execute_http_request, get_entry_part, or get_entry when you need more text (e.g. web crawlers printing page HTML — try 80000–150000 on 128K+ context models).\n\
   - get_entry_part for headers, cookies, or individual bodies\n\
   - summarize_entries for compact multi-entry overviews\n\
   - trace_cookies / trace_storage / get_auth_flow for session & auth flows\n\
   - decode_jwt for JWT header/payload decoding (NEVER guess token claims)\n\
   - list_js_scripts / get_js_call_map / get_deobfuscated_js / get_js_snippet for JavaScript relationships\n\
     - search_bodies / list_endpoints / compare_entries / compare_sessions for reverse-engineering APIs, cross-HAR session diffs\n\
    - When comparing or cross-referencing multiple HAR files: most data-accessing tools (list_entries, get_entry, get_session_overview, get_chunk_summaries, get_entry_part, search_bodies, trace_cookies, list_js_scripts, decode_jwt, etc.) accept an optional session_id parameter. Pass the session ID of another loaded HAR to query its data directly — no need to switch sessions. Combine with compare_sessions for a full diff, then drill into specific entries/chunks/bodies from either session.\n\
   - When stuck on auth (401/403) or missing parameters: search_bodies for tokens/keys, get_auth_flow, trace_cookies, get_js_snippet(search=...) — use capture evidence before guessing\n\
   - get_chunk_summaries / get_chunk_details for prior LLM analysis notes\n\
3. Use get_entry(detail=full) or get_entry_part(mode=full) when you need larger body slices — pass max_output_chars to raise the cap. Prefer get_deobfuscated_js (readable, commented) when the user deobfuscated a script in the JS tab; use get_js_snippet to quote exact line ranges in your answer.\n\
4. HTTP replay & minimal curl workflow:\n\
   - generate_curl(entry_index) for the golden HAR replay command; add header_names_to_omit / overrides to strip it manually.\n\
   - execute_http_request to send live requests (use entry_index as template, omit headers, tweak body). \
     If live replay returns 401/403, do NOT mock or hardcode a successful response in scripts or your answer. \
     Use get_auth_flow, trace_cookies, search_bodies, and execute_http_request with HAR headers/cookies until live replay works or you can prove it cannot — then report failure honestly.\n\
     **Live HTTP log:** every execute_http_request / execute_request / minimize probe is recorded for this chat. \
     After token rotation (refresh/OAuth/login), later calls MUST use the updated credentials — replaying entry_index resends stale HAR tokens. \
     Call list_live_http_requests / get_live_http_request / get_live_auth_state when auth fails or you lose track of rotated tokens.\n\
     If live replay returns 5xx (especially 504), treat it as server-side/transient — retry at most once per entry, then explain the failure instead of looping through many entries.\n\
   - minimize_http_request(entry_index, body_contains?) to automatically find the smallest live request that still works. \
     If the baseline golden request fails live (5xx), do NOT keep minimizing — report that the capture cannot be replayed right now. \
     After one successful minimize_http_request, summarize results for the user — do not minimize every working entry in the session.\n\
   - execute_request is a shortcut replay of the full HAR entry.\n\
     - run_script for multi-step prototypes — **always prefer Python** (cross-platform). PowerShell is a fallback only when Python cannot do the job; on Linux/macOS PowerShell is not built-in (user must install `pwsh`). \
       **Script workspace:** use edit_script(code=…) to create or replace the script (no execution). \
       Then call run_script to execute. Every later fix MUST use edit_script with append_code and/or replacements — run_script with no edits is rejected. \
       Use run_script(re_run=true) only to re-execute the unchanged script (e.g. after pip install). Never paste the whole script into run_script unless reset=true.\n\
       **ITERATIVE DEVELOPMENT — CRITICAL:** Never generate the entire script at once. Build incrementally:\
       - edit_script(code=…) creates a small, focused first version (one endpoint, one feature).\
       - run_script with skip_quality_checks=true to test immediately.\
       - Use edit_script(append_code=…) to add the NEXT small piece. Test again.\
       - Repeat: add one feature → test → fix → repeat. Never write 100+ lines in one edit_script call.\
       - Only when ALL features work independently, run_script WITHOUT skip_quality_checks for final validation.\
       - If you catch yourself writing a huge code block, STOP — break it into separate edit_script + run_script cycles.\n\
       **Python code quality rules — MANDATORY:** \
       - **PEP8 compliance is REQUIRED**: proper indentation (4 spaces), blank lines between functions/classes, line length ≤ 100 chars.\n\
       - **NO one-liners**: Every statement must be on its own line. No `if x: do_something()` on one line.\n\
       - **NO single-character variable names**: Use descriptive names like `response_data` not `r`, `entry_count` not `c`. Exception: loop variables `i`, `j`, `k` are acceptable.\n\
       - **Docstrings REQUIRED**: Every function must have a docstring explaining purpose, parameters, and return value.\n\
       - **Comments REQUIRED**: Explain WHY, not WHAT. Comments should clarify intent, edge cases, or non-obvious logic.\n\
       - The script must be a single, coherent prototype — no redundant `if __name__ == \"__main__\"` blocks, no duplicate `def main()`. One entry point only. \
       - Always include a proper CLI interface with argparse or sys.argv. Never hardcode test URLs/paths — accept them as arguments. \
       - Use `sys.exit(0)` on success, `sys.exit(1)` on failure. Never silently swallow exceptions. \
       - Before submitting, mentally verify: are all imports at the top? Are all variables defined before use? No duplicate function names? \
       - Use `edit_script` to review and refine the workspace code between tool steps — iterate until it is clean, well-structured, and human-written in style. \
       - When the script resets (reset=true), call `get_script_history` to see previous versions and avoid repeating mistakes or regressing on improvements. \
       **Library awareness — CHECK BEFORE WRITING:** \
       - Before writing any script, call `list_packages()` to see what's available in the Python environment.\n\
       - **Prefer specialized libraries over reinventing the wheel**: use `requests` for HTTP, `BeautifulSoup` or `lxml` for HTML parsing, `playwright` for browser automation, `pandas` for data manipulation, `jq` for JSON queries.\n\
       - If a task would be dramatically simpler with a library that's not installed, **suggest the user install it** and provide the exact `pip install` command. Never silently fall back to a hacky approach.\n\
       - Example: \"This HTML parsing task would be much cleaner with BeautifulSoup. Run: `pip install beautifulsoup4` then I'll rewrite the parser.\"\n\
      Pass args (CLI, Python sys.argv[1:]) and env (os.environ) when the script needs runtime inputs. \
     In your **final answer**, reference the workspace script with `{{script}}` — never paste the full script again in prose. \
     Before run_script with third-party imports, call check_python_environment (omit packages for pip list, or pass package names to verify). \
     **Before run_script, mentally review the code for typos, wrong imports, and missing variables** — run_script auto-runs `py_compile` and rejects syntax errors without executing. Fix syntax errors from the tool error before retrying. \
      **Never substitute mock/fake/demo/placeholder API data** when requests fail — run_script rejects scripts with demo mode, search_demo(), or silent fallbacks. \
      Skip these quality checks during intermediate test iterations by passing skip_quality_checks=true. Always run the final script iteration WITHOUT skip_quality_checks to prove real API output — exit 0 with real data is the only success metric. \
      If the final quality-checked run fails (stub/mock), the agent must report that it could not build a working prototype.\n\
     On live API failure the script must sys.exit(non-zero) and your answer must report failure — do not ship fake product JSON. \
     Exit code 0 with mocked/demo stdout does not count as success.\n\
      After the first edit_script, never pass code= again — only append_code/replacements (or reset=true). Reference the script once with `{{script}}` in your answer; do not repeat it in usage examples.\n\
     **Never re-run the same broken script unchanged** — if stderr repeats, apply a different fix via replacements/append_code (increases script revision) or switch to execute_http_request. After 3 identical failures the tool blocks until you edit the script; use force=true only after a real code change. \
     run_script is blocked when the script embeds stale auth tokens that were superseded by an earlier live HTTP response — call get_live_auth_state and update the script.\n\
     If run_script returns SCRIPT_BLOCKED (missing pip package), **stop calling tools** and ask the user to run the exact pip install command shown, then wait for confirmation before retrying. \
     Python is optional for HAR analysis — HTTP replay tools use Rust and need no Python. If Python is missing, run_script fails with install/venv guidance (Settings → Agent Python venv). \
     Disabled if user turned off \"Allow agent to run scripts\" in Settings.\n\
   NEVER pass URLs scraped from HTML/JS bodies for entry_index — use list_entries indices only.\n\
5. When you see JWTs, call decode_jwt — never invent claims.\n\
6. After gathering evidence via tools, synthesize your answer — but first re-read tool outputs and challenge your own conclusions before writing.\n\
7. Stop calling tools once you have enough to answer; do not over-fetch. If a key claim still feels shaky, one targeted verification tool call is better than guessing.\n\n\
Chunk analysis: If get_session_overview or get_chunk_summaries reports no summaries yet, say so explicitly and continue with raw entry tools — you can still analyze the HAR fully. \
When summaries exist, use get_chunk_details to see what each chunk covered before re-fetching the same data.\n\n\
If tools return no data, say so explicitly.";

pub const CHAT_ANSWER_QUALITY_GUIDE: &str = "\n\n\
Answer quality — verify, question yourself, separate facts from guesses:\n\
Before your final answer, double-check every claim: \"Did a tool result actually show this, or am I assuming?\" \
If a claim feels uncertain, call another tool to confirm instead of guessing.\n\n\
Question your own conclusions: look for contradictions between entries, missing requests in the capture, \
truncated bodies, and alternative explanations. If new tool data overturns an earlier hypothesis, say so explicitly.\n\n\
Structure final answers with these sections (omit empty sections, keep headings):\n\
### Observed in this HAR\n\
Facts directly from tool results only. Cite entry indices [#], exact URLs, status codes, header names/values, \
JS/body excerpts (use get_js_snippet line numbers when quoting code), and **full decoded JWT header/payload JSON** from decode_jwt. No speculation in this section.\n\
### Inferred / architectural interpretation\n\
Educated guesses about site architecture, auth design, backend stack, business logic, or runtime behavior \
NOT explicitly proven by the capture. Label with **Likely**, **Probably**, or **May indicate**. \
For each inference: (1) which observed facts support it, (2) what is still unknown, (3) what evidence would confirm or disprove it.\n\
### Self-check\n\
Briefly note: contradictions you looked for, gaps in the capture, claims you avoided due to insufficient evidence, \
and anything you re-verified with a second tool lookup.\n\n\
Never present inferences as observed facts. If a paragraph mixes both, mark each sentence.\n\n\
Script prototypes: if run_script never succeeded (non-zero exit, stderr errors, or mock/stub output detected), your final answer must say the prototype **failed** — use `{{script}}` (never paste a different \"fixed\" script). \
Never claim a script works when tool results show failure, stub detection, or fabricated/mock responses. \
When the user asked for live API behavior and replay returns 403, explain what you tried from the HAR and what blocked you — do not invent successful JSON.";

/// Full system prompt for the chat agent (base + answer-quality guide).
pub fn chat_system_prompt() -> String {
    chat_system_prompt_with_knowledge(None)
}

pub fn chat_system_prompt_with_knowledge(knowledge_tree: Option<&str>) -> String {
    let base_prompt = format!("{CHAT_SYSTEM_PROMPT}{CHAT_ANSWER_QUALITY_GUIDE}");
    
    if let Some(tree_content) = knowledge_tree {
        base_prompt.replace("{KNOWLEDGE_TREE}", tree_content)
    } else {
        base_prompt.replace("{KNOWLEDGE_TREE}", "(No knowledge accumulated yet)")
    }
}

pub const THINKING_CHAT_SUPPLEMENT: &str = "\n\nThinking mode: you do not have live HAR tool access in this mode. \
Answer from the conversation, session metadata, and general HAR/API knowledge. \
Clearly label anything not directly from prior messages as **Inferred**. \
Include a brief **Self-check** noting what could not be verified without tools. \
If the user needs verified data from this capture, suggest turning off thinking mode and using the default model.";

pub fn prompt_for_chunk_type(chunk_type: &str) -> &'static str {
    if chunk_type == "javascript" {
        CHUNK_JS_PROMPT
    } else {
        CHUNK_TRAFFIC_PROMPT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_preamble_and_markdown_fence() {
        let input = "Here's the synthesized final report with markdown formatting:\n\n```markdown\n# Final Report\n\n- Endpoint A\n```";
        assert_eq!(
            normalize_markdown_report(input),
            "# Final Report\n\n- Endpoint A"
        );
    }

    #[test]
    fn normalize_leaves_plain_markdown_untouched() {
        let input = "## Overview\n\nTraffic looks normal.";
        assert_eq!(normalize_markdown_report(input), input);
    }

    #[test]
    fn detects_premature_stop_colon_trailing() {
        assert!(looks_like_premature_stop(
            "I have enough information to build your Python CLI script. Let me create and test it:"
        ));
    }

    #[test]
    fn detects_script_request_in_user_message() {
        let messages = vec![ChatRequestMessage::text(
            "user",
            "Make a Python script that would search through Yandex Market for products via CLI.",
        )];
        assert!(user_wants_script_prototype(&messages));
    }

    #[test]
    fn detects_long_script_planning_without_delivery() {
        let text = "The user wants a Python CLI script to search Yandex Market products. \
Based on the HAR analysis, I've gathered the key endpoints and required headers structure. \
Key observations:\n- Search endpoint: /api/screen/search-request\n\
Let me create the script now.";
        assert!(looks_like_script_planning_only(text));
        assert!(looks_like_premature_stop(text));
    }

    #[test]
    fn nudges_when_script_requested_but_not_run() {
        let messages = vec![ChatRequestMessage::text(
            "user",
            "Make a Python script for Yandex Market search via CLI.",
        )];
        let turn = AssistantTurn {
            content: "Let me create and test it:".to_string(),
            reasoning: String::new(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        assert_eq!(
            agent_continue_nudge(&messages, &turn, false, false),
            Some(AGENT_SCRIPT_DELIVERY_NUDGE)
        );
    }

    #[test]
    fn nudges_when_script_claims_success_without_run() {
        let messages = vec![ChatRequestMessage::text(
            "user",
            "Make a Python script for Yandex Market search via CLI.",
        )];
        let turn = AssistantTurn {
            content: "I've created a working Python script for Yandex Market search. Here's the complete solution."
                .to_string(),
            reasoning: String::new(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        assert_eq!(
            agent_continue_nudge(&messages, &turn, true, false),
            Some(AGENT_SCRIPT_FALSE_SUCCESS_NUDGE)
        );
    }

    #[test]
    fn allows_honest_failure_answer_without_nudge() {
        let messages = vec![ChatRequestMessage::text(
            "user",
            "Make a Python script for Yandex Market search via CLI.",
        )];
        let turn = AssistantTurn {
            content: "I could not build a working script — live API returns 504 and auth from this HAR is insufficient."
                .to_string(),
            reasoning: String::new(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        assert!(agent_continue_nudge(&messages, &turn, true, false).is_none());
    }

    #[test]
    fn normalize_unwraps_plain_fence() {
        let input = "```\n## Summary\n\nDone.\n```";
        assert_eq!(normalize_markdown_report(input), "## Summary\n\nDone.");
    }

    #[test]
    fn extracts_complete_embedded_tool_call() {
        let text = r#"Let's check tips-full.

<tool_call>
{"name": "list_entries", "arguments": {"query": "/bff/search/tips-full", "limit": 5}}
</tool_call>"#;
        let (cleaned, calls) = extract_embedded_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list_entries");
        assert!(!cleaned.contains("tool_call"));
        assert!(cleaned.contains("Let's check tips-full"));
    }

    #[test]
    fn detects_incomplete_embedded_tool_call() {
        let text = r#"Let's call list_entries.

<tool_call> {"name": "list_entries", "arguments": {"query": "tips-full", "limit": 5"#;
        assert!(has_incomplete_tool_call(text));
        assert!(extract_embedded_tool_calls(text).1.is_empty());
        assert_eq!(
            strip_trailing_partial_tool_call(text),
            "Let's call list_entries."
        );
    }

    #[test]
    fn extracts_complete_dsml_run_script() {
        let text = r#"Here is the script.

Now let me run this:

<｜DSML｜｜tool_calls>
<｜DSML｜｜invoke name="run_script">
<｜DSML｜｜parameter name="language" string="true">python</｜DSML｜｜parameter>
<｜DSML｜｜parameter name="code" string="true">print("hello")</｜DSML｜｜parameter>
</｜DSML｜｜invoke>
</｜DSML｜｜tool_calls>"#;
        let (cleaned, calls) = extract_dsml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "run_script");
        assert!(calls[0].function.arguments.contains("hello"));
        assert!(!cleaned.contains("DSML"));
        assert!(cleaned.contains("Here is the script"));
    }

    #[test]
    fn detects_incomplete_dsml() {
        let text = r#"Running soon <｜DSML｜｜invoke name="run_script">"#;
        assert!(has_incomplete_tool_call(text));
        assert!(extract_dsml_tool_calls(text).1.is_empty());
        assert_eq!(
            strip_trailing_partial_tool_call(text),
            "Running soon"
        );
    }

    #[test]
    fn enrich_agent_turn_executes_dsml_instead_of_leaking() {
        let mut turn = AssistantTurn {
            content: r#"Script ready.
<｜DSML｜｜invoke name="list_entries">
<｜DSML｜｜parameter name="limit" string="true">5</｜DSML｜｜parameter>
</｜DSML｜｜invoke>"#
                .to_string(),
            reasoning: String::new(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        enrich_agent_turn(&mut turn);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].function.name, "list_entries");
        assert!(serde_json::from_str::<Value>(&turn.tool_calls[0].function.arguments).is_ok());
        assert!(!turn.content.contains("DSML"));
    }

    #[test]
    fn normalizes_invalid_tool_arguments_to_json_object() {
        assert_eq!(normalize_function_arguments(""), "{}");
        assert_eq!(normalize_function_arguments("not json"), "{}");
        assert_eq!(
            normalize_function_arguments(r#"{"entry_index":3}"#),
            r#"{"entry_index":3}"#
        );
    }

    #[test]
    fn normalizes_double_encoded_arguments() {
        let inner = r#"{"code":"print(1)"}"#;
        let wrapped = serde_json::to_string(&Value::String(inner.to_string())).unwrap();
        let out = normalize_function_arguments(&wrapped);
        assert_eq!(out, inner);
    }

    #[test]
    fn resolve_final_content_prefers_reasoning_with_answer_sections() {
        let turn = AssistantTurn {
            content: "{}".to_string(),
            reasoning: "Planning…\n\n### Observed in this HAR\n\nFact one.".to_string(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        let answer = resolve_final_agent_content(&turn);
        assert!(answer.contains("### Observed"));
        assert!(!answer.contains("{}"));
    }

    #[test]
    fn planning_text_strips_final_answer_from_reasoning() {
        let turn = AssistantTurn {
            content: String::new(),
            reasoning: "I need to finish now.\n\n### Observed in this HAR\n\nFact.".to_string(),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        };
        let final_content = resolve_final_agent_content(&turn);
        let planning = planning_text_for_display(&turn, &final_content);
        assert!(planning.contains("finish now"));
        assert!(!planning.contains("### Observed"));
    }
}
