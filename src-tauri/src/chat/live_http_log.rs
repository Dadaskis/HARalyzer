use crate::har::types::HeaderPair;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::http_tools::{HttpExecuteResult, HttpRequestSpec};

const MAX_RECORDS: usize = 50;
const MAX_VALUE_LEN: usize = 16_384;
const PREVIEW_LEN: usize = 2_000;

#[derive(Debug, Clone)]
pub struct AuthValue {
    pub key: String,
    pub value: String,
    pub source_record_id: u32,
}

#[derive(Debug, Clone, Default)]
pub struct LiveAuthState {
    /// Latest known auth values keyed by normalized name (e.g. refresh_token, cookie:wbx_refresh).
    values: HashMap<String, AuthValue>,
}

#[derive(Debug, Clone)]
pub struct LiveHttpRecord {
    pub id: u32,
    pub source_tool: String,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<HeaderPair>,
    pub request_body_preview: String,
    pub status: u16,
    pub response_headers: Vec<String>,
    pub response_body_preview: String,
    pub elapsed_ms: f64,
    pub auth_changes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveHttpSessionLog {
    records: Vec<LiveHttpRecord>,
    auth_state: LiveAuthState,
    next_id: u32,
}

pub const STALE_TOKEN_WARNING: &str = "\n\n[Live auth state — STALE TOKEN DETECTED]\n\
This request reuses an auth value that was superseded by an earlier live response in this chat session. \
Rotating endpoints (refresh token, login, OAuth) issue new credentials — later calls must use the latest values, not the original HAR capture.\n\
Call list_live_http_requests / get_live_http_request to review prior live exchanges and copy updated tokens into headers, cookies, or script variables.\n";

pub const AUTH_ROTATION_HINT: &str = "\n\n[Live auth state — credentials updated]\n\
This response issued new auth material. Any follow-up live request or run_script MUST use these updated values — \
replaying the same HAR entry_index will send stale tokens from the capture.\n\
Use get_live_auth_state or get_live_http_request to retrieve the latest tokens before the next call.\n";

fn json_token_fields() -> &'static [(&'static str, &'static str)] {
    &[
        ("refresh_token", "refresh_token"),
        ("refreshToken", "refresh_token"),
        ("access_token", "access_token"),
        ("accessToken", "access_token"),
        ("id_token", "id_token"),
        ("idToken", "id_token"),
        ("token", "token"),
        ("auth_token", "auth_token"),
        ("authToken", "auth_token"),
    ]
}

fn cookie_auth_names() -> &'static [&'static str] {
    &[
        "refresh",
        "token",
        "session",
        "auth",
        "jwt",
        "access",
        "wb",
        "wbx",
        "x-auth",
    ]
}

fn jwt_in_text_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:Bearer\s+)?([A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)")
            .expect("jwt in text regex")
    })
}

fn json_field_re(field: &str) -> Regex {
    // Keep repetition bound modest — the regex crate rejects very large upper limits.
    Regex::new(&format!(r#"(?i)"{field}"\s*:\s*"([^"]{{8,4096}})""#)).unwrap_or_else(|e| {
        panic!("json field regex for {field}: {e}");
    })
}

fn normalize_auth_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    if lower.starts_with("cookie:") {
        let name = lower.strip_prefix("cookie:").unwrap_or(&lower);
        if name.contains("refresh") {
            return "refresh_token".to_string();
        }
        if name.contains("access") {
            return "access_token".to_string();
        }
        if name.contains("token") || name.contains("auth") || name.contains("session") {
            return format!("cookie:{name}");
        }
    }
    lower
}

fn normalize_auth_map(map: HashMap<String, String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (key, value) in map {
        out.insert(normalize_auth_key(&key), value);
    }
    out
}

fn normalize_value(value: &str) -> String {
    value.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn truncate_preview(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        let mut end = max.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &text[..end])
    }
}

fn preview_token(value: &str) -> String {
    let v = normalize_value(value);
    if v.len() <= 24 {
        v
    } else {
        format!("{}…{} ({} chars)", &v[..12], &v[v.len() - 8..], v.len())
    }
}

fn is_auth_header_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("authorization")
        || lower.contains("cookie")
        || lower.contains("token")
        || lower.contains("x-api-key")
        || lower.contains("x-auth")
        || lower.starts_with("x-wb")
}

fn parse_cookie_pairs(header_value: &str) -> Vec<(String, String)> {
    header_value
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            let (name, value) = part.split_once('=')?;
            if name.trim().is_empty() {
                return None;
            }
            Some((name.trim().to_string(), normalize_value(value)))
        })
        .collect()
}

fn cookie_key(name: &str) -> String {
    format!("cookie:{}", name.to_ascii_lowercase())
}

fn looks_like_auth_cookie(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    cookie_auth_names()
        .iter()
        .any(|needle| lower.contains(needle))
}

pub fn extract_auth_from_request(spec: &HttpRequestSpec) -> HashMap<String, String> {
    let mut out = HashMap::new();

    for h in &spec.headers {
        let lower = h.name.to_ascii_lowercase();
        if lower == "authorization" {
            out.insert("authorization".to_string(), normalize_value(&h.value));
        } else if lower == "cookie" {
            for (name, value) in parse_cookie_pairs(&h.value) {
                if looks_like_auth_cookie(&name) || value.len() >= 16 {
                    out.insert(cookie_key(&name), value);
                }
            }
        } else if lower.contains("token") || lower.contains("auth") {
            out.insert(lower.clone(), normalize_value(&h.value));
        }
    }

    if !spec.body.is_empty() {
        out.extend(extract_auth_from_json_body(&spec.body));
    }

    normalize_auth_map(out)
}

pub fn extract_auth_from_response(body: &str, response_headers: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();

    for line in response_headers {
        let (name, value) = match line.split_once(':') {
            Some(v) => v,
            None => continue,
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("set-cookie") {
            if let Some((cookie_name, cookie_value)) = value.split_once('=') {
                let cookie_name = cookie_name.trim();
                let cookie_value = cookie_value
                    .split(';')
                    .next()
                    .unwrap_or(cookie_value)
                    .trim();
                if looks_like_auth_cookie(cookie_name) || cookie_value.len() >= 16 {
                    out.insert(cookie_key(cookie_name), normalize_value(cookie_value));
                }
            }
        } else if is_auth_header_name(name) {
            out.insert(name.to_ascii_lowercase(), normalize_value(value));
        }
    }

    out.extend(extract_auth_from_json_body(body));
    normalize_auth_map(out)
}

fn extract_auth_from_json_body(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (field, key) in json_token_fields() {
        if let Some(re) = json_field_cache().get(*field) {
            if let Some(caps) = re.captures(body) {
                if let Some(m) = caps.get(1) {
                    out.insert((*key).to_string(), normalize_value(m.as_str()));
                }
            }
        }
    }
    out
}

fn json_field_cache() -> &'static HashMap<&'static str, Regex> {
    static CACHE: OnceLock<HashMap<&'static str, Regex>> = OnceLock::new();
    CACHE.get_or_init(|| {
        json_token_fields()
            .iter()
            .map(|(field, _)| (*field, json_field_re(field)))
            .collect()
    })
}

impl LiveAuthState {
    pub fn apply_updates(
        &mut self,
        updates: HashMap<String, String>,
        record_id: u32,
    ) -> Vec<String> {
        let mut changes = Vec::new();
        for (key, value) in updates {
            if value.len() > MAX_VALUE_LEN {
                continue;
            }
            let preview = preview_token(&value);
            match self.values.get(&key) {
                Some(prev) if prev.value == value => {}
                Some(prev) => {
                    changes.push(format!(
                        "{key} rotated (was {} → now {}) [live #{record_id}]",
                        preview_token(&prev.value),
                        preview
                    ));
                    self.values.insert(
                        key.clone(),
                        AuthValue {
                            key,
                            value,
                            source_record_id: record_id,
                        },
                    );
                }
                None => {
                    changes.push(format!("{key} captured ({preview}) [live #{record_id}]"));
                    self.values.insert(
                        key.clone(),
                        AuthValue {
                            key,
                            value,
                            source_record_id: record_id,
                        },
                    );
                }
            }
        }
        changes
    }

    pub fn check_request_staleness(&self, request_auth: &HashMap<String, String>) -> Vec<String> {
        let mut warnings = Vec::new();
        for (key, sent) in request_auth {
            let Some(latest) = self.values.get(key) else {
                continue;
            };
            if sent == &latest.value {
                continue;
            }
            // Only warn when we have evidence the server issued a newer value.
            if sent.len() >= 8 && latest.value.len() >= 8 {
                warnings.push(format!(
                    "Request sends stale {key}: using {} but live #{} already returned {}",
                    preview_token(sent),
                    latest.source_record_id,
                    preview_token(&latest.value)
                ));
            }
        }
        warnings
    }

    pub fn format_summary(&self) -> String {
        if self.values.is_empty() {
            return "No live auth values captured yet.".to_string();
        }
        let mut lines: Vec<String> = self
            .values
            .values()
            .map(|v| {
                format!(
                    "• {} = {} (from live #{})",
                    v.key,
                    preview_token(&v.value),
                    v.source_record_id
                )
            })
            .collect();
        lines.sort();
        format!(
            "Latest live auth values ({}):\n\n{}\n\n\
             Use these in execute_http_request headers/cookies or run_script — not stale HAR capture tokens.",
            lines.len(),
            lines.join("\n")
        )
    }
}

impl LiveHttpSessionLog {
    pub fn record_exchange(
        &mut self,
        source_tool: &str,
        spec: &HttpRequestSpec,
        result: &HttpExecuteResult,
    ) -> LiveHttpRecord {
        let id = self.next_id;
        self.next_id += 1;

        let request_auth = extract_auth_from_request(spec);
        let mut warnings = self.auth_state.check_request_staleness(&request_auth);

        let response_auth =
            extract_auth_from_response(&result.body_preview, &result.response_headers);
        let auth_changes = self.auth_state.apply_updates(response_auth, id);

        if !auth_changes.is_empty() && source_tool.contains("entry_index") {
            warnings.push(
                "Response updated auth tokens — if you replay entry_index again you will send stale HAR values."
                    .to_string(),
            );
        }

        let record = LiveHttpRecord {
            id,
            source_tool: source_tool.to_string(),
            method: spec.method.clone(),
            url: spec.url.clone(),
            request_headers: spec
                .headers
                .iter()
                .filter(|h| is_auth_header_name(&h.name))
                .cloned()
                .collect(),
            request_body_preview: truncate_preview(&spec.body, PREVIEW_LEN),
            status: result.status,
            response_headers: result.response_headers.clone(),
            response_body_preview: truncate_preview(&result.body_preview, PREVIEW_LEN),
            elapsed_ms: result.elapsed_ms,
            auth_changes,
            warnings: warnings.clone(),
        };

        self.records.push(record.clone());
        if self.records.len() > MAX_RECORDS {
            let drop = self.records.len() - MAX_RECORDS;
            self.records.drain(0..drop);
        }

        record
    }

    pub fn auth_state(&self) -> &LiveAuthState {
        &self.auth_state
    }

    pub fn format_warnings_for_result(record: &LiveHttpRecord) -> String {
        let mut out = String::new();
        if !record.warnings.is_empty() {
            out.push_str(STALE_TOKEN_WARNING);
            for w in &record.warnings {
                out.push_str(&format!("• {w}\n"));
            }
        }
        if !record.auth_changes.is_empty() {
            out.push_str(AUTH_ROTATION_HINT);
            for c in &record.auth_changes {
                out.push_str(&format!("• {c}\n"));
            }
        }
        out
    }

    pub fn list_records(&self, limit: usize, offset: usize) -> String {
        let total = self.records.len();
        if total == 0 {
            return "No live HTTP requests recorded in this chat session yet.".to_string();
        }
        let end = (offset + limit).min(total);
        if offset >= total {
            return format!(
                "No records at offset {offset} (total live requests: {total})."
            );
        }
        let slice = &self.records[offset..end];
        let mut out = format!(
            "Live HTTP log: showing {}–{} of {total} (newest last). \
             Call get_live_http_request(id) for full request/response.\n\n",
            offset + 1,
            offset + slice.len()
        );
        for r in slice {
            let auth_note = if r.auth_changes.is_empty() {
                String::new()
            } else {
                format!(" · auth: {}", r.auth_changes.join("; "))
            };
            let warn_note = if r.warnings.is_empty() {
                String::new()
            } else {
                " · ⚠ stale auth".to_string()
            };
            out.push_str(&format!(
                "#{id} {tool} {method} {url} → HTTP {status}{auth}{warn}\n",
                id = r.id,
                tool = r.source_tool,
                method = r.method,
                url = truncate_preview(&r.url, 120),
                status = r.status,
                auth = auth_note,
                warn = warn_note,
            ));
        }
        if !self.auth_state.values.is_empty() {
            out.push_str("\n");
            out.push_str(&self.auth_state.format_summary());
        }
        out
    }

    pub fn get_record(&self, id: u32) -> Option<String> {
        let r = self.records.iter().find(|rec| rec.id == id)?;
        let mut out = format!(
            "Live HTTP #{id} ({tool})\n{method} {url}\nHTTP {status} ({elapsed:.0} ms)\n\n",
            id = r.id,
            tool = r.source_tool,
            method = r.method,
            url = r.url,
            status = r.status,
            elapsed = r.elapsed_ms,
        );
        if !r.request_headers.is_empty() {
            out.push_str("Request auth headers:\n");
            for h in &r.request_headers {
                out.push_str(&format!("  {}: {}\n", h.name, preview_token(&h.value)));
            }
            out.push('\n');
        }
        if !r.request_body_preview.is_empty() {
            out.push_str(&format!(
                "Request body preview:\n```\n{}\n```\n\n",
                r.request_body_preview
            ));
        }
        out.push_str("Response headers:\n");
        for h in &r.response_headers {
            out.push_str(&format!("  {h}\n"));
        }
        out.push('\n');
        out.push_str(&format!(
            "Response body preview:\n```\n{}\n```\n",
            r.response_body_preview
        ));
        if !r.warnings.is_empty() {
            out.push_str(STALE_TOKEN_WARNING);
            for w in &r.warnings {
                out.push_str(&format!("• {w}\n"));
            }
        }
        if !r.auth_changes.is_empty() {
            out.push_str(AUTH_ROTATION_HINT);
            for c in &r.auth_changes {
                out.push_str(&format!("• {c}\n"));
            }
        }
        Some(out)
    }
}

/// Scan script source for hardcoded auth values that differ from the latest live session state.
pub fn check_script_auth_staleness(code: &str, auth_state: &LiveAuthState) -> Option<String> {
    if auth_state.values.is_empty() {
        return None;
    }

    let script_auth = extract_auth_from_json_body(code);
    let mut stale: Vec<String> = Vec::new();

    for (key, sent) in &script_auth {
        let Some(latest) = auth_state.values.get(key) else {
            continue;
        };
        if sent != &latest.value && sent.len() >= 12 && latest.value.len() >= 12 {
            stale.push(format!(
                "script hardcodes stale {key}: {} but live #{} returned {}",
                preview_token(sent),
                latest.source_record_id,
                preview_token(&latest.value)
            ));
        }
    }

    for av in auth_state.values.values() {
        if av.value.len() < 16 {
            continue;
        }
        if code.contains(&av.value) {
            continue;
        }
        for secret in find_embedded_secrets(code) {
            if secret.len() >= 16
                && secret != av.value
                && !stale.iter().any(|s| s.contains(&preview_token(&secret)))
            {
                stale.push(format!(
                    "script embeds a token/credential that does not match latest live {} from #{} ({})",
                    av.key,
                    av.source_record_id,
                    preview_token(&av.value)
                ));
                break;
            }
        }
    }

    if stale.is_empty() {
        None
    } else {
        Some(format!(
            "Script auth staleness detected:\n{}\n\n\
             Update the script via replacements/append_code to use the latest live auth values \
             (get_live_auth_state / get_live_http_request), or re-run execute_http_request with updated headers.",
            stale
                .iter()
                .map(|s| format!("• {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

fn find_embedded_secrets(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(caps) = jwt_in_text_re().captures(code) {
        if let Some(m) = caps.get(1) {
            found.push(m.as_str().to_string());
        }
    }
    found.extend(extract_auth_from_json_body(code).into_values());
    found
}

pub fn format_script_staleness_warning(msg: &str) -> String {
    format!(
        "{STALE_TOKEN_WARNING}{msg}\n\n\
         Fix the script before re-running, or call get_live_auth_state to see current credentials."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_token_rotation() {
        let mut log = LiveHttpSessionLog::default();
        let spec1 = HttpRequestSpec {
            method: "POST".into(),
            url: "https://example.com/refresh".into(),
            headers: vec![HeaderPair {
                name: "Cookie".into(),
                value: "refresh=OLD_TOKEN_12345".into(),
            }],
            body: String::new(),
        };
        let result1 = HttpExecuteResult {
            status: 200,
            status_text: "OK".into(),
            elapsed_ms: 10.0,
            response_headers: vec![],
            body_preview: r#"{"refresh_token":"NEW_TOKEN_12345","access_token":"ACCESS_12"}"#.into(),
            body_bytes: 40,
        };
        let rec1 = log.record_exchange("execute_http_request", &spec1, &result1);
        assert!(!rec1.auth_changes.is_empty());

        let spec2 = HttpRequestSpec {
            method: "GET".into(),
            url: "https://example.com/api".into(),
            headers: vec![HeaderPair {
                name: "Cookie".into(),
                value: "refresh=OLD_TOKEN_12345".into(),
            }],
            body: String::new(),
        };
        let result2 = HttpExecuteResult {
            status: 401,
            status_text: "Unauthorized".into(),
            elapsed_ms: 5.0,
            response_headers: vec![],
            body_preview: "unauthorized".into(),
            body_bytes: 12,
        };
        let rec2 = log.record_exchange("execute_http_request", &spec2, &result2);
        assert!(!rec2.warnings.is_empty());
    }

    #[test]
    fn extracts_json_tokens() {
        let body = r#"{"refreshToken":"abc123xyz","data":1}"#;
        let auth = extract_auth_from_json_body(body);
        assert_eq!(auth.get("refresh_token").map(String::as_str), Some("abc123xyz"));
    }
}
