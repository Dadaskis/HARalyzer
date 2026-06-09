use crate::har::types::AppSettings;
use futures::StreamExt;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const REQUEST_TIMEOUT_SECS: u64 = 180;
/// Timeout for chat agent tool-planning calls (non-streaming).
pub const AGENT_PLANNING_TIMEOUT_SECS: u64 = 180;
const AGENT_TOOL_RESULT_MAX_CHARS: usize = 3_500;
const AGENT_MAX_TOOL_MESSAGES: usize = 8;
pub const AGENT_MAX_TOOLS_PER_STEP: usize = 4;
pub const AGENT_MAX_TOOLS_PER_RUN: usize = 12;
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
    "get_js_analysis",
    "get_session_overview",
    "get_chunk_summaries",
    "generate_curl",
    "execute_request",
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
        Some(other) => other.to_string(),
        None => "{}".to_string(),
    };

    Some(ToolCall {
        id: format!("call_{}", uuid::Uuid::new_v4()),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: name.to_string(),
            arguments,
        },
    })
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
    if let Some(idx) = text.find("<tool_call>") {
        text[..idx].trim_end().to_string()
    } else {
        text.trim().to_string()
    }
}

pub fn has_incomplete_tool_call(text: &str) -> bool {
    text.contains("<tool_call>") && extract_embedded_tool_calls(text).1.is_empty()
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
    } else {
        turn.content = extract_embedded_tool_calls(&turn.content).0;
        turn.reasoning = extract_embedded_tool_calls(&turn.reasoning).0;
    }

    turn.content = strip_trailing_partial_tool_call(&turn.content);
    turn.reasoning = strip_trailing_partial_tool_call(&turn.reasoning);
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelData>,
}

#[derive(Debug, Deserialize)]
struct ModelData {
    id: String,
    name: Option<String>,
}

fn build_http_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

fn http_client() -> Result<Client, String> {
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

fn preview_body(body: &str, max: usize) -> String {
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

fn message_content_len(m: &ChatRequestMessage) -> usize {
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
fn clamp_chat_messages(messages: &mut [ChatRequestMessage]) {
    const MIN_LAST_USER_CHARS: usize = 2_000;

    loop {
        let total: usize = messages.iter().map(message_content_len).sum();
        if total <= HARD_MAX_REQUEST_CHARS {
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
            let budget = HARD_MAX_REQUEST_CHARS
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
pub fn prune_agent_messages(messages: &mut [ChatRequestMessage]) {
    let mut tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();

    while tool_indices.len() > AGENT_MAX_TOOL_MESSAGES {
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
                *content = truncate_chat_content(content, AGENT_TOOL_RESULT_MAX_CHARS);
            }
        }
    }

    clamp_chat_messages(messages);
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

async fn post_chat_with_retry(
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
    if let Some(text) = delta.get("reasoning").and_then(|v| v.as_str()) {
        reasoning.push_str(text);
    }

    (content, reasoning)
}

pub fn resolve_chat_model(settings: &AppSettings, thinking_mode: bool) -> String {
    if thinking_mode && !settings.thinking_model.trim().is_empty() {
        settings.thinking_model.clone()
    } else {
        settings.default_model.clone()
    }
}

pub fn format_chat_reply(content: &str, reasoning: &str, thinking_mode: bool) -> String {
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
    stream_chat_cancellable(settings, model, messages, || false, on_update).await
}

pub async fn stream_chat_cancellable<F, C>(
    settings: &AppSettings,
    model: &str,
    messages: Vec<ChatRequestMessage>,
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

    let client = http_client()?;
    let mut messages = messages;
    clamp_chat_messages(&mut messages);

    let request = ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens: None,
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

    while let Some(item) = byte_stream.next().await {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }

        let chunk = item.map_err(|e| format!("Stream read failed: {e}"))?;
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

    let client = http_client()?;
    let resp = client
        .get(MODELS_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {e}"))?;

    if !resp.status().is_success() {
        return Ok(default_models());
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read models response: {e}"))?;

    let parsed: ModelsResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse models: {e}"))?;

    let fetched: Vec<OpenRouterModel> = parsed
        .data
        .into_iter()
        .map(|m| OpenRouterModel {
            name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
        })
        .collect();

    Ok(merge_with_defaults(fetched))
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
    vec![
        OpenRouterModel {
            id: "openai/gpt-4o-mini".to_string(),
            name: "GPT-4o Mini".to_string(),
        },
        OpenRouterModel {
            id: "openai/gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
        },
        OpenRouterModel {
            id: "anthropic/claude-3.5-sonnet".to_string(),
            name: "Claude 3.5 Sonnet".to_string(),
        },
        OpenRouterModel {
            id: "google/gemini-flash-1.5".to_string(),
            name: "Gemini Flash 1.5".to_string(),
        },
        OpenRouterModel {
            id: "deepseek/deepseek-chat".to_string(),
            name: "DeepSeek Chat".to_string(),
        },
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
    post_chat_with_retry(
        &client,
        &settings.openrouter_api_key,
        &settings.default_model,
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
    messages: Vec<ChatRequestMessage>,
    max_tokens: Option<u32>,
    tools: Option<Vec<Value>>,
) -> Result<AssistantTurn, String> {
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

    prune_agent_messages(&mut messages);

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
    let step = 24usize;
    for end in (step..=chars.len()).step_by(step) {
        if should_cancel() {
            return Err(crate::chat::agent_state::CHAT_CANCELLED_ERROR.to_string());
        }
        let partial: String = chars[..end].iter().collect();
        on_update(&partial, reasoning);
        tokio::time::sleep(Duration::from_millis(6)).await;
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
CRITICAL: You MUST use the provided tools to look up real session data before stating facts about URLs, status codes, headers, bodies, JavaScript, or API behavior. Never invent or guess HAR data.\n\n\
Workflow:\n\
1. Start with list_entries or get_session_overview to orient yourself.\n\
2. Use get_entry for request/response details; get_js_analysis for JavaScript sources.\n\
3. Use get_chunk_summaries for prior LLM analysis notes.\n\
4. Use generate_curl to show a replay command; use execute_request only when the user wants live testing.\n\
5. After verifying facts via tools, answer in clear markdown citing entry indices and exact values.\n\
6. Stop calling tools once you have enough to answer — do not exhaustively scan the entire HAR. \
   If the user asks for code (e.g. a Python script), deliver the complete script after identifying the relevant endpoints.\n\n\
If tools return no data, say so explicitly.";

pub const THINKING_CHAT_SUPPLEMENT: &str = "\n\nThinking mode: you do not have live HAR tool access in this mode. \
Answer from the conversation, session metadata, and general HAR/API knowledge. \
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
}
