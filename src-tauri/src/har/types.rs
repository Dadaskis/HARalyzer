use crate::har::js_analyzer::{analyze_javascript, decode_content_text, llm_body, store_body};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarEntrySummary {
    pub index: usize,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub mime_type: String,
    pub size: u64,
    pub time_ms: f64,
    pub started_at: Option<String>,
    pub is_javascript: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarEntryDetail {
    pub summary: HarEntrySummary,
    pub request_headers: Vec<HeaderPair>,
    pub response_headers: Vec<HeaderPair>,
    pub request_body: String,
    pub response_body: String,
    pub js_insights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarParseProgress {
    pub bytes_read: u64,
    pub total_bytes: u64,
    pub entries_parsed: usize,
    pub phase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarParseComplete {
    pub session_id: String,
    pub file_path: String,
    pub file_name: String,
    pub total_entries: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarChunk {
    pub id: String,
    pub session_id: String,
    pub chunk_index: usize,
    pub entry_count: usize,
    pub estimated_tokens: usize,
    pub payload: String,
    pub summary: Option<String>,
    pub status: String,
    pub chunk_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisSession {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub total_entries: usize,
    pub total_bytes: u64,
    pub created_at: String,
    pub status: String,
    pub final_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub openrouter_api_key: String,
    pub default_model: String,
    pub thinking_model: String,
    pub chunk_max_tokens: usize,
    pub filter_static_assets: bool,
    pub max_concurrent_requests: usize,
    pub analyze_javascript: bool,
    #[serde(default = "default_chat_agent_max_steps")]
    pub chat_agent_max_steps: usize,
}

fn default_chat_agent_max_steps() -> usize {
    10
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            openrouter_api_key: String::new(),
            default_model: "openai/gpt-4o-mini".to_string(),
            thinking_model: String::new(),
            chunk_max_tokens: 3000,
            filter_static_assets: true,
            max_concurrent_requests: 4,
            analyze_javascript: true,
            chat_agent_max_steps: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamEvent {
    pub session_id: String,
    pub content: String,
    pub reasoning: String,
    pub done: bool,
    pub message_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolEvent {
    pub session_id: String,
    pub id: String,
    pub step: usize,
    pub tool: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAgentLimitEvent {
    pub session_id: String,
    pub steps_used: usize,
    pub step_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSendResult {
    pub message: Option<ChatMessage>,
    pub needs_continue: bool,
    pub steps_used: usize,
    pub step_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisProgress {
    pub session_id: String,
    pub phase: String,
    pub chunks_done: usize,
    pub chunks_total: usize,
    pub current_chunk: Option<usize>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis_done: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis_total: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis_round: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStreamChunk {
    pub session_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub context_type: Option<String>,
    pub context_ref: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContext {
    pub context_type: String,
    pub entry_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct RawHarEntry {
    pub request: RawRequest,
    pub response: RawResponse,
    #[serde(rename = "startedDateTime")]
    pub started_date_time: Option<String>,
    pub time: Option<f64>,
    #[serde(rename = "_resourceType")]
    pub resource_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawRequest {
    pub method: String,
    pub url: String,
    pub headers: Option<Vec<RawHeader>>,
    #[serde(rename = "postData")]
    pub post_data: Option<RawPostData>,
}

#[derive(Debug, Deserialize)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Option<Vec<RawHeader>>,
    pub content: Option<RawContent>,
}

#[derive(Debug, Deserialize)]
pub struct RawHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct RawPostParam {
    pub name: String,
    pub value: Option<String>,
    #[serde(rename = "fileName")]
    pub file_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawPostData {
    pub text: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub encoding: Option<String>,
    pub params: Option<Vec<RawPostParam>>,
}

fn extract_request_body(post_data: Option<&RawPostData>) -> String {
    let Some(pd) = post_data else {
        return String::new();
    };

    if let Some(text) = &pd.text {
        if !text.is_empty() {
            let decoded = crate::har::js_analyzer::decode_content_text(
                Some(text.clone()),
                pd.encoding.clone(),
            );
            return crate::har::js_analyzer::store_body(&decoded);
        }
    }

    if let Some(params) = &pd.params {
        if !params.is_empty() {
            let body = format_post_params(params, pd.mime_type.as_deref());
            return crate::har::js_analyzer::store_body(&body);
        }
    }

    String::new()
}

fn format_post_params(params: &[RawPostParam], mime_type: Option<&str>) -> String {
    if params.len() == 1 {
        if let Some(v) = &params[0].value {
            if params[0].name.is_empty() || mime_type.unwrap_or("").contains("json") {
                return v.clone();
            }
        }
    }

    if mime_type.unwrap_or("").contains("json") {
        if let Ok(json) = serde_json::to_string_pretty(params) {
            return json;
        }
    }

    params
        .iter()
        .map(|p| {
            let value = p.value.as_deref().unwrap_or("");
            if p.file_name.is_some() {
                format!("{}=<file: {}>", p.name, p.file_name.as_deref().unwrap_or(""))
            } else {
                format!("{}={}", p.name, value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Deserialize)]
pub struct RawContent {
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub size: Option<u64>,
    pub text: Option<String>,
    pub encoding: Option<String>,
}

pub fn entry_from_raw(index: usize, raw: RawHarEntry) -> HarEntryDetail {
    let mime_type = raw
        .response
        .content
        .as_ref()
        .and_then(|c| c.mime_type.clone())
        .unwrap_or_default();

    let request_headers: Vec<HeaderPair> = raw
        .request
        .headers
        .as_ref()
        .map(|hs| {
            hs.iter()
                .map(|h| HeaderPair {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let response_headers: Vec<HeaderPair> = raw
        .response
        .headers
        .as_ref()
        .map(|hs| {
            hs.iter()
                .map(|h| HeaderPair {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let request_body = extract_request_body(raw.request.post_data.as_ref());

    let response_text = raw
        .response
        .content
        .as_ref()
        .map(|c| decode_content_text(c.text.clone(), c.encoding.clone()))
        .unwrap_or_default();

    let response_body = store_body(&response_text);
    let is_javascript = is_javascript_entry(&mime_type, &raw.request.url);

    let js_insights = if is_javascript && !response_text.is_empty() {
        analyze_javascript(&response_text)
    } else {
        Vec::new()
    };

    HarEntryDetail {
        summary: HarEntrySummary {
            index,
            method: raw.request.method,
            url: raw.request.url,
            status: raw.response.status,
            mime_type,
            size: raw
                .response
                .content
                .as_ref()
                .and_then(|c| c.size)
                .unwrap_or(0),
            time_ms: raw.time.unwrap_or(0.0),
            started_at: raw.started_date_time,
            is_javascript,
            resource_type: raw.resource_type,
        },
        request_headers,
        response_headers,
        request_body,
        response_body,
        js_insights,
    }
}

pub fn is_javascript_entry(mime_type: &str, url: &str) -> bool {
    let mime = mime_type.to_lowercase();
    let lower = url.to_lowercase();
    mime.contains("javascript")
        || mime.contains("ecmascript")
        || lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".jsx")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
}

pub fn is_static_asset(mime_type: &str, url: &str) -> bool {
    if is_javascript_entry(mime_type, url) {
        return false;
    }
    let mime = mime_type.to_lowercase();
    if mime.starts_with("image/")
        || mime.starts_with("font/")
        || mime.contains("woff")
    {
        return true;
    }
    let lower = url.to_lowercase();
    lower.ends_with(".css")
        || lower.ends_with(".map")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".svg")
        || lower.ends_with(".ico")
        || lower.ends_with(".woff")
        || lower.ends_with(".woff2")
}

pub fn should_keep_entry(filter_static: bool, entry: &HarEntryDetail, analyze_js: bool) -> bool {
    if !filter_static {
        return true;
    }
    if analyze_js && entry.summary.is_javascript {
        return true;
    }
    !is_static_asset(&entry.summary.mime_type, &entry.summary.url)
}

pub fn normalize_entry_for_llm(entry: &HarEntryDetail) -> String {
    let s = &entry.summary;
    let mut line = format!(
        "[{}] {} {} -> {} ({}, {} bytes, {:.0}ms)",
        s.index, s.method, s.url, s.status, s.mime_type, s.size, s.time_ms
    );

    if !entry.request_body.is_empty() {
        line.push_str(&format!(
            "\n  Request body: {}",
            llm_body(&entry.request_body)
        ));
    }

    if s.method != "GET" && entry.response_body.len() < 2000 {
        if entry.summary.mime_type.contains("json") || entry.summary.mime_type.contains("text") {
            line.push_str(&format!(
                "\n  Response body: {}",
                llm_body(&entry.response_body)
            ));
        }
    }

    if entry.summary.is_javascript {
        if !entry.js_insights.is_empty() {
            line.push_str("\n  JS patterns detected:");
            for insight in &entry.js_insights {
                line.push_str(&format!("\n    - {insight}"));
            }
        }
        if !entry.response_body.is_empty() {
            line.push_str(&format!(
                "\n  JS source excerpt:\n```javascript\n{}\n```",
                llm_body(&entry.response_body)
            ));
        }
    }

    line
}

pub fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

pub fn build_chunks_from_entries(
    session_id: &str,
    entries: &[HarEntryDetail],
    max_tokens: usize,
) -> Vec<HarChunk> {
    let mut chunks = Vec::new();
    let mut traffic_lines: Vec<String> = Vec::new();
    let mut traffic_tokens = 0usize;
    let mut js_lines: Vec<String> = Vec::new();
    let mut js_tokens = 0usize;
    let mut chunk_index = 0usize;

    for entry in entries {
        let line = normalize_entry_for_llm(entry);
        let line_tokens = estimate_tokens(&line);

        if entry.summary.is_javascript {
            if !js_lines.is_empty() && js_tokens + line_tokens > max_tokens {
                push_chunk(
                    &mut chunks,
                    session_id,
                    &mut chunk_index,
                    &js_lines.join("\n"),
                    js_lines.len(),
                    js_tokens,
                    "javascript",
                );
                js_lines.clear();
                js_tokens = 0;
            }
            js_lines.push(line);
            js_tokens += line_tokens;
        } else {
            if !traffic_lines.is_empty() && traffic_tokens + line_tokens > max_tokens {
                push_chunk(
                    &mut chunks,
                    session_id,
                    &mut chunk_index,
                    &traffic_lines.join("\n"),
                    traffic_lines.len(),
                    traffic_tokens,
                    "traffic",
                );
                traffic_lines.clear();
                traffic_tokens = 0;
            }
            traffic_lines.push(line);
            traffic_tokens += line_tokens;
        }
    }

    if !traffic_lines.is_empty() {
        push_chunk(
            &mut chunks,
            session_id,
            &mut chunk_index,
            &traffic_lines.join("\n"),
            traffic_lines.len(),
            traffic_tokens,
            "traffic",
        );
    }

    if !js_lines.is_empty() {
        push_chunk(
            &mut chunks,
            session_id,
            &mut chunk_index,
            &js_lines.join("\n"),
            js_lines.len(),
            js_tokens,
            "javascript",
        );
    }

    chunks
}

fn push_chunk(
    chunks: &mut Vec<HarChunk>,
    session_id: &str,
    chunk_index: &mut usize,
    payload: &str,
    entry_count: usize,
    estimated_tokens: usize,
    chunk_type: &str,
) {
    chunks.push(HarChunk {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        chunk_index: *chunk_index,
        entry_count,
        estimated_tokens,
        payload: payload.to_string(),
        summary: None,
        status: "pending".to_string(),
        chunk_type: chunk_type.to_string(),
    });
    *chunk_index += 1;
}

pub fn entry_detail_to_context(entry: &HarEntryDetail) -> String {
    let mut ctx = normalize_entry_for_llm(entry);
    if !entry.request_headers.is_empty() {
        ctx.push_str("\n\nRequest headers:");
        for h in &entry.request_headers {
            ctx.push_str(&format!("\n  {}: {}", h.name, h.value));
        }
    }
    if !entry.response_headers.is_empty() {
        ctx.push_str("\n\nResponse headers:");
        for h in entry.response_headers.iter().take(30) {
            ctx.push_str(&format!("\n  {}: {}", h.name, h.value));
        }
    }
    ctx
}
