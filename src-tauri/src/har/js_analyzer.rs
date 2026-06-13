const MAX_BODY_STORE: usize = 120_000;
const MAX_BODY_FETCH: usize = 5_000_000;
const MAX_BODY_LLM: usize = 120_000;

pub fn truncate_body(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }

    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}… [truncated, {} bytes total]", &text[..end], text.len())
}

pub fn decode_content_text(text: Option<String>, encoding: Option<String>) -> String {
    let Some(raw) = text else {
        return String::new();
    };
    let decoded = if encoding.as_deref() == Some("base64") {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(raw.as_bytes())
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or(raw)
    } else {
        raw
    };
    fix_utf8_mojibake(&decoded)
}

/// Fix UTF-8 bytes misinterpreted as Latin-1 (common with Cyrillic in HAR JSON).
fn fix_utf8_mojibake(text: &str) -> String {
    let cyrillic_before = text.chars().filter(|c| ('\u{0400}'..='\u{04FF}').contains(c)).count();
    if cyrillic_before > 2 {
        return text.to_string();
    }
    if !text.chars().any(|c| c as u32 >= 0x80) {
        return text.to_string();
    }
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let fixed = String::from_utf8_lossy(&bytes).into_owned();
    let cyrillic_after = fixed.chars().filter(|c| ('\u{0400}'..='\u{04FF}').contains(c)).count();
    if cyrillic_after > cyrillic_before {
        fixed
    } else {
        text.to_string()
    }
}

pub fn store_body_fetch(text: &str) -> String {
    truncate_body(text, MAX_BODY_FETCH)
}

pub fn analyze_javascript(source: &str) -> Vec<String> {
    let mut findings = Vec::new();
    // Use r##"..."## for patterns that contain double-quote characters.
    let patterns: &[(&str, &str)] = &[
        (r##"fetch\s*\(\s*['"`]([^'"`]+)"##, "fetch"),
        (r"fetch\s*\(\s*([a-zA-Z_$][\w$]*)", "fetch (variable URL)"),
        (r##"XMLHttpRequest|\.open\s*\(\s*['"](GET|POST|PUT|DELETE|PATCH)"##, "XHR"),
        (r"axios\.(get|post|put|delete|patch|request)\s*\(", "axios"),
        (r"\$\.(?:ajax|get|post|getJSON)\s*\(", "jQuery AJAX"),
        (r"navigator\.sendBeacon\s*\(", "sendBeacon"),
        (r##"new\s+WebSocket\s*\(\s*['"]"##, "WebSocket"),
        (r##"EventSource\s*\(\s*['"]"##, "SSE/EventSource"),
        (r##"graphql|/gql['"`]"##, "GraphQL endpoint hint"),
        (r##"Authorization['"`]?\s*[:=]"##, "Authorization header in JS"),
        (r"Bearer\s+[A-Za-z0-9\-_.]+", "Bearer token literal"),
        (r##"localStorage\.(get|set)Item\s*\(\s*['"]"##, "localStorage access"),
        (r##"sessionStorage\.(get|set)Item\s*\(\s*['"]"##, "sessionStorage access"),
        (r"JSON\.stringify\s*\(", "JSON.stringify (payload serialization)"),
    ];

    for (pattern, label) in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            for cap in re.captures_iter(source).take(5) {
                let snippet = cap
                    .get(0)
                    .map(|m| truncate_body(m.as_str(), 120))
                    .unwrap_or_default();
                findings.push(format!("{label}: `{snippet}`"));
            }
        }
    }

    if findings.is_empty() && (source.contains("fetch") || source.contains("axios")) {
        findings.push("Contains fetch/axios references (pattern match inconclusive)".to_string());
    }

    findings.sort();
    findings.dedup();
    findings.truncate(20);
    findings
}

pub fn store_body(text: &str) -> String {
    truncate_body(text, MAX_BODY_STORE)
}

pub fn llm_body(text: &str) -> String {
    truncate_body(text, MAX_BODY_LLM)
}
