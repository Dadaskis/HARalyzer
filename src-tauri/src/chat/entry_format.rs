use crate::har::js_analyzer::truncate_body;
use crate::har::types::HarEntryDetail;
use serde_json::Value;

/// Short excerpt shown by default in `get_entry` and `get_entry_part` body fetches.
pub const ENTRY_BODY_PREVIEW_CHARS: usize = 600;

/// Truncate long header values in overview mode.
const HEADER_VALUE_PREVIEW_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyViewMode {
    Preview,
    Summary,
    Full,
}

impl BodyViewMode {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("summary") => Self::Summary,
            Some("full") => Self::Full,
            _ => Self::Preview,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryDetailLevel {
    Overview,
    Full,
}

impl EntryDetailLevel {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("full") => Self::Full,
            _ => Self::Overview,
        }
    }
}

pub fn format_body(text: &str, mode: BodyViewMode) -> String {
    format_body_limited(text, mode, None)
}

pub fn format_body_limited(text: &str, mode: BodyViewMode, max_chars: Option<usize>) -> String {
    if text.is_empty() {
        return "(empty)".to_string();
    }
    match mode {
        BodyViewMode::Preview => truncate_body(
            text,
            max_chars.unwrap_or(ENTRY_BODY_PREVIEW_CHARS),
        ),
        BodyViewMode::Summary => summarize_body(text),
        BodyViewMode::Full => truncate_body(
            text,
            max_chars.unwrap_or(super::output_limits::ENTRY_BODY_FULL_DEFAULT),
        ),
    }
}

pub fn format_entry_detail(
    entry: &HarEntryDetail,
    level: EntryDetailLevel,
    max_body_chars: Option<usize>,
) -> String {
    let s = &entry.summary;
    let truncate_headers = level == EntryDetailLevel::Overview;
    let body_mode = match level {
        EntryDetailLevel::Overview => BodyViewMode::Preview,
        EntryDetailLevel::Full => BodyViewMode::Full,
    };

    let mut out = format!(
        "Entry [{}] {} {}\nStatus: {} · MIME: {} · Size: {} bytes · Time: {:.0}ms\n",
        s.index, s.method, s.url, s.status, s.mime_type, s.size, s.time_ms
    );

    if !entry.request_headers.is_empty() {
        out.push_str("\nRequest headers:\n");
        for h in &entry.request_headers {
            out.push_str(&format!(
                "  {}: {}\n",
                h.name,
                format_header_value(&h.value, truncate_headers)
            ));
        }
    }

    if !entry.request_body.is_empty() {
        out.push_str("\nRequest body");
        push_body_section(&mut out, &entry.request_body, body_mode, level, max_body_chars);
    }

    if !entry.response_headers.is_empty() {
        out.push_str("\nResponse headers:\n");
        for h in entry.response_headers.iter().take(40) {
            out.push_str(&format!(
                "  {}: {}\n",
                h.name,
                format_header_value(&h.value, truncate_headers)
            ));
        }
        if entry.response_headers.len() > 40 {
            out.push_str(&format!(
                "  … (+{} more headers — use get_entry_part part=response_headers)\n",
                entry.response_headers.len() - 40
            ));
        }
    }

    if !entry.response_body.is_empty() {
        out.push_str("\nResponse body (from HAR capture)");
        push_body_section(&mut out, &entry.response_body, body_mode, level, max_body_chars);
    }

    if s.is_javascript && !entry.js_insights.is_empty() {
        out.push_str("\nJS patterns (use get_js_analysis for source excerpt):\n");
        for insight in &entry.js_insights {
            out.push_str(&format!("  - {insight}\n"));
        }
    }

    if level == EntryDetailLevel::Overview {
        out.push_str(
            "\n---\nBodies above are short previews only. \
Use get_entry_part(part=request_body|response_body, mode=summary) for structure/stats, \
mode=full for a larger body slice (pass max_output_chars to raise the cap), or get_entry(detail=full) for both bodies.\n",
        );
    }

    out
}

fn push_body_section(
    out: &mut String,
    body: &str,
    mode: BodyViewMode,
    level: EntryDetailLevel,
    max_body_chars: Option<usize>,
) {
    match mode {
        BodyViewMode::Summary => {
            out.push_str(" (summary):\n");
            out.push_str(&format_body(body, BodyViewMode::Summary));
            out.push('\n');
        }
        BodyViewMode::Preview | BodyViewMode::Full => {
            let label = if level == EntryDetailLevel::Overview && mode == BodyViewMode::Preview {
                format!(" (preview, {} bytes total)", body.len())
            } else {
                format!(" ({} bytes total)", body.len())
            };
            out.push_str(&label);
            out.push_str(":\n```\n");
            out.push_str(&format_body_limited(body, mode, max_body_chars));
            out.push_str("\n```\n");
        }
    }
}

fn format_header_value(value: &str, truncate: bool) -> String {
    if !truncate || value.len() <= HEADER_VALUE_PREVIEW_CHARS {
        return value.to_string();
    }
    truncate_body(value, HEADER_VALUE_PREVIEW_CHARS)
}

pub fn summarize_body(text: &str) -> String {
    if text.is_empty() {
        return "(empty)".to_string();
    }

    let total = text.len();
    let trimmed = text.trim_start();

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return summarize_json(&value, total);
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("<html") || lower.contains("<!doctype") {
        return summarize_html(text, total);
    }

    let lines = text.lines().count();
    let preview = truncate_body(text, 400);
    format!("Text/plain · {total} bytes · {lines} lines\nPreview:\n{preview}")
}

fn summarize_json(value: &Value, total_bytes: usize) -> String {
    match value {
        Value::Object(map) => {
            let key_count = map.len();
            let keys: Vec<&str> = map.keys().map(String::as_str).take(25).collect();
            let mut out = format!(
                "JSON object · {total_bytes} bytes · {key_count} top-level keys\nKeys: {}",
                keys.join(", ")
            );
            if key_count > keys.len() {
                out.push_str(&format!(" … (+{} more)", key_count - keys.len()));
            }
            for (key, val) in map.iter().take(10) {
                out.push_str(&format!("\n  · {key}: {}", json_value_hint(val)));
            }
            out
        }
        Value::Array(arr) => {
            let mut out = format!(
                "JSON array · {total_bytes} bytes · {} items",
                arr.len()
            );
            if let Some(first) = arr.first() {
                out.push_str(&format!("\nFirst item: {}", json_value_hint(first)));
                if arr.len() > 1 {
                    if let Some(last) = arr.last() {
                        if json_shape_signature(first) == json_shape_signature(last) {
                            out.push_str("\nItems appear homogeneous (first ≈ last shape).");
                        } else {
                            out.push_str(&format!(
                                "\nLast item: {}",
                                json_value_hint(last)
                            ));
                        }
                    }
                }
            }
            out
        }
        other => format!(
            "JSON scalar · {total_bytes} bytes\nValue: {}",
            json_value_hint(other)
        ),
    }
}

fn json_shape_signature(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            format!("object:{}", keys.join(","))
        }
        Value::Array(arr) => format!("array:{}", arr.len()),
        other => json_value_hint(other),
    }
}

fn json_value_hint(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("bool ({b})"),
        Value::Number(_) => "number".into(),
        Value::String(s) => {
            if s.len() <= 80 {
                format!("string \"{s}\"")
            } else {
                format!("string ({} chars)", s.len())
            }
        }
        Value::Array(arr) => {
            if let Some(first) = arr.first() {
                format!("array[{}] of {}", arr.len(), json_value_hint(first))
            } else {
                "array[0]".into()
            }
        }
        Value::Object(map) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).take(12).collect();
            let mut hint = format!("object {{{}}}", keys.join(", "));
            if map.len() > keys.len() {
                hint.push_str(&format!(" … +{} keys", map.len() - keys.len()));
            }
            hint.push('}');
            hint
        }
    }
}

fn summarize_html(text: &str, total_bytes: usize) -> String {
    let lower = text.to_ascii_lowercase();
    let script_count = lower.matches("<script").count();
    let link_count = lower.matches("<link").count();
    let title = text
        .lines()
        .find_map(|line| {
            let l = line.to_ascii_lowercase();
            l.find("<title")
                .map(|start| {
                    let rest = &line[start..];
                    rest.split('>')
                        .nth(1)
                        .and_then(|t| t.split('<').next())
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
        })
        .filter(|t| !t.is_empty());

    let mut out = format!(
        "HTML · {total_bytes} bytes · ~{script_count} script tags · ~{link_count} link tags"
    );
    if let Some(title) = title {
        out.push_str(&format!("\n<title>: {title}"));
    }
    out.push_str("\nPreview:\n");
    out.push_str(&truncate_body(text, 400));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_shorter_than_full() {
        let big = "x".repeat(5000);
        let preview = format_body(&big, BodyViewMode::Preview);
        let full = format_body(&big, BodyViewMode::Full);
        assert!(preview.len() < full.len());
        assert!(preview.contains("truncated"));
    }

    #[test]
    fn summarizes_json_object_keys() {
        let body = r#"{"products":[{"id":1}],"total":42,"page":1}"#;
        let summary = summarize_body(body);
        assert!(summary.contains("JSON object"));
        assert!(summary.contains("products"));
        assert!(summary.contains("total"));
    }

    #[test]
    fn summarizes_json_array() {
        let body = r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#;
        let summary = summarize_body(body);
        assert!(summary.contains("JSON array"));
        assert!(summary.contains("2 items"));
        assert!(summary.contains("homogeneous"));
    }

    #[test]
    fn overview_entry_includes_escalation_hint() {
        let entry = HarEntryDetail {
            summary: crate::har::types::HarEntrySummary {
                index: 0,
                method: "GET".into(),
                url: "https://example.com".into(),
                status: 200,
                mime_type: "application/json".into(),
                size: 100,
                time_ms: 12.0,
                started_at: None,
                is_javascript: false,
                resource_type: None,
            },
            request_headers: vec![],
            response_headers: vec![],
            request_body: String::new(),
            response_body: "{\"data\":\"ok\"}".into(),
            js_insights: vec![],
            deobfuscated_js: None,
        };
        let out = format_entry_detail(&entry, EntryDetailLevel::Overview, None);
        assert!(out.contains("preview"));
        assert!(out.contains("get_entry_part"));
    }
}
