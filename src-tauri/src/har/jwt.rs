use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtKind {
    Jws,
    Jwe,
}

#[derive(Debug, Clone)]
pub enum JwtSegment {
    Json(Value),
    Opaque {
        reason: String,
        byte_length: usize,
        hex_preview: String,
    },
}

#[derive(Debug, Clone)]
pub struct DecodedJwt {
    pub kind: JwtKind,
    pub header: JwtSegment,
    pub payload: JwtSegment,
    /// JWS signature or JWE segments after header (encrypted key, iv, ciphertext, tag).
    pub trailing_segments: Vec<JwtSegment>,
    pub claim_notes: Vec<String>,
}

fn jwt_body_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:Bearer\s+)?([A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+){2,4})")
            .expect("jwt regex")
    })
}

pub fn normalize_jwt_token(raw: &str) -> String {
    let s = raw.trim().trim_matches('"').trim_matches('\'');
    if let Some(rest) = s.strip_prefix("Bearer ") {
        rest.trim().to_string()
    } else if let Some(rest) = s.strip_prefix("bearer ") {
        rest.trim().to_string()
    } else {
        s.to_string()
    }
}

fn is_b64url_segment(part: &str) -> bool {
    !part.is_empty()
        && part.len() >= 4
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn looks_like_jwt(token: &str) -> bool {
    let token = normalize_jwt_token(token);
    let parts: Vec<&str> = token.split('.').collect();
    matches!(parts.len(), 3 | 5) && parts.iter().all(|p| is_b64url_segment(p))
}

fn decode_segment_bytes(segment: &str) -> Result<Vec<u8>, String> {
    let mut padded = segment.to_string();
    let pad = padded.len() % 4;
    if pad != 0 {
        padded.push_str(&"=".repeat(4 - pad));
    }

    URL_SAFE_NO_PAD
        .decode(segment)
        .or_else(|_| URL_SAFE_NO_PAD.decode(&padded))
        .or_else(|_| URL_SAFE.decode(&padded))
        .map_err(|e| format!("base64 decode failed: {e}"))
}

fn hex_preview(bytes: &[u8], max: usize) -> String {
    bytes
        .iter()
        .take(max)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn opaque_reason(segment_name: &str, err: &str, _bytes: &[u8], header_json: Option<&Value>) -> String {
    if let Some(header) = header_json {
        if header.get("enc").is_some() {
            return format!(
                "{segment_name} is not readable JSON — this looks like an encrypted JWT (JWE); \
                 the {segment_name} is ciphertext or key material, not claims."
            );
        }
        if header.get("zip").and_then(|v| v.as_str()) == Some("DEF") {
            return format!(
                "{segment_name} is not UTF-8 JSON — header declares DEF compression; \
                 payload may be compressed rather than plain JSON."
            );
        }
    }

    if err.contains("UTF-8") {
        format!(
            "{segment_name} decoded from base64 but is not UTF-8 text ({err}). \
             Common causes: encrypted JWE segment, compressed payload, truncated token in the HAR, \
             or a false positive (random data that matched the JWT shape)."
        )
    } else {
        format!("{segment_name}: {err}")
    }
}

fn decode_segment_flexible(
    segment: &str,
    segment_name: &str,
    header_json: Option<&Value>,
) -> JwtSegment {
    match decode_segment_bytes(segment) {
        Ok(bytes) => {
            match String::from_utf8(bytes.clone()) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(value) => JwtSegment::Json(value),
                    Err(e) => JwtSegment::Opaque {
                        reason: format!(
                            "{segment_name} is UTF-8 text but not JSON: {e}. \
                             Raw text preview: {}",
                            preview_text(&text, 120)
                        ),
                        byte_length: bytes.len(),
                        hex_preview: hex_preview(&bytes, 32),
                    },
                },
                Err(e) => JwtSegment::Opaque {
                    reason: opaque_reason(segment_name, &e.to_string(), &bytes, header_json),
                    byte_length: bytes.len(),
                    hex_preview: hex_preview(&bytes, 32),
                },
            }
        }
        Err(e) => JwtSegment::Opaque {
            reason: format!("{segment_name}: {e}"),
            byte_length: 0,
            hex_preview: String::new(),
        },
    }
}

fn preview_text(text: &str, max: usize) -> String {
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

fn header_json_value(header: &JwtSegment) -> Option<&Value> {
    match header {
        JwtSegment::Json(v) => Some(v),
        JwtSegment::Opaque { .. } => None,
    }
}

fn claim_notes(payload: &JwtSegment) -> Vec<String> {
    let obj = match payload {
        JwtSegment::Json(Value::Object(obj)) => obj,
        _ => return Vec::new(),
    };

    let mut notes = Vec::new();
    for key in ["exp", "iat", "nbf", "auth_time"] {
        if let Some(secs) = obj.get(key).and_then(|v| v.as_i64()) {
            notes.push(format_unix_claim(key, secs));
        }
    }
    if let Some(sub) = obj.get("sub").and_then(|v| v.as_str()) {
        notes.push(format!("sub={sub}"));
    }
    if let Some(aud) = obj.get("aud") {
        notes.push(format!("aud={aud}"));
    }
    if let Some(iss) = obj.get("iss").and_then(|v| v.as_str()) {
        notes.push(format!("iss={iss}"));
    }
    notes
}

fn format_unix_claim(name: &str, secs: i64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_opt(secs, 0).single() {
        Some(dt) => format!("{name}={secs} ({dt} UTC)"),
        None => format!("{name}={secs}"),
    }
}

pub fn decode_jwt_token(raw: &str) -> Result<DecodedJwt, String> {
    let token = normalize_jwt_token(raw);
    if !looks_like_jwt(&token) {
        return Err(
            "String does not look like a JWT (expected 3-part JWS or 5-part JWE)".to_string(),
        );
    }

    let parts: Vec<&str> = token.split('.').collect();

    if parts.len() == 5 {
        let header = decode_segment_flexible(parts[0], "JWE header", None);
        let header_ref = header_json_value(&header);
        let mut trailing = Vec::new();
        for (idx, name) in [
            "encrypted key",
            "initialization vector",
            "ciphertext",
            "authentication tag",
        ]
        .iter()
        .enumerate()
        {
            trailing.push(decode_segment_flexible(
                parts[idx + 1],
                name,
                header_ref,
            ));
        }
        let notes = Vec::new();
        return Ok(DecodedJwt {
            kind: JwtKind::Jwe,
            header,
            payload: trailing
                .get(2)
                .cloned()
                .unwrap_or(JwtSegment::Opaque {
                    reason: "missing ciphertext segment".into(),
                    byte_length: 0,
                    hex_preview: String::new(),
                }),
            trailing_segments: trailing,
            claim_notes: notes,
        });
    }

    let header = decode_segment_flexible(parts[0], "header", None);
    let header_ref = header_json_value(&header);
    let payload = decode_segment_flexible(parts[1], "payload", header_ref);
    let signature = decode_segment_flexible(parts[2], "signature", header_ref);
    let notes = claim_notes(&payload);

    Ok(DecodedJwt {
        kind: JwtKind::Jws,
        header,
        payload,
        trailing_segments: vec![signature],
        claim_notes: notes,
    })
}

pub fn find_jwts_in_text(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for caps in jwt_body_regex().captures_iter(text) {
        let token = normalize_jwt_token(caps.get(1).unwrap().as_str());
        if looks_like_jwt(&token) && seen.insert(token.clone()) {
            found.push(token);
        }
    }
    found
}

pub fn scan_entry_for_jwts(
    request_headers: &[crate::har::types::HeaderPair],
    response_headers: &[crate::har::types::HeaderPair],
    request_body: &str,
    response_body: &str,
) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut scan = |location: &str, text: &str| {
        for token in find_jwts_in_text(text) {
            if seen.insert(token.clone()) {
                hits.push((location.to_string(), token));
            }
        }
    };

    for h in request_headers {
        let name = h.name.to_ascii_lowercase();
        if name.contains("authorization")
            || name.contains("token")
            || name.contains("jwt")
            || name.contains("cookie")
        {
            scan(&format!("request header {}", h.name), &h.value);
        }
    }
    for h in response_headers {
        let name = h.name.to_ascii_lowercase();
        if name.contains("authorization")
            || name.contains("token")
            || name.contains("jwt")
            || name.contains("set-cookie")
        {
            scan(&format!("response header {}", h.name), &h.value);
        }
    }
    scan("request body", request_body);
    scan("response body", response_body);

    hits.sort_by(|a, b| a.0.cmp(&b.0));
    hits
}

fn format_segment(label: &str, segment: &JwtSegment) -> String {
    match segment {
        JwtSegment::Json(value) => {
            let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
            format!("{label}:\n```json\n{json}\n```")
        }
        JwtSegment::Opaque {
            reason,
            byte_length,
            hex_preview,
        } => {
            let hex_note = if hex_preview.is_empty() {
                String::new()
            } else {
                format!("\nHex preview (first bytes): `{hex_preview}`")
            };
            format!(
                "{label}: _(not JSON — {byte_length} bytes decoded from base64)_\n\
                 {reason}{hex_note}"
            )
        }
    }
}

pub fn format_decoded_jwt(decoded: &DecodedJwt, location: &str, entry_index: Option<usize>) -> String {
    let entry_note = entry_index
        .map(|i| format!("Entry [{i}] — "))
        .unwrap_or_default();

    let kind = match decoded.kind {
        JwtKind::Jws => "JWS (signed JWT)",
        JwtKind::Jwe => "JWE (encrypted JWT — claims are inside ciphertext and cannot be read without the key)",
    };

    let claims = if decoded.claim_notes.is_empty() {
        String::new()
    } else {
        format!("\nClaim summary: {}", decoded.claim_notes.join("; "))
    };

    let mut out = format!("{entry_note}{location}\nType: {kind}\n");

    if decoded.kind == JwtKind::Jwe {
        out.push_str(&format_segment("Header", &decoded.header));
        out.push_str("\n\n");
        let labels = [
            "Encrypted key",
            "IV",
            "Ciphertext (claims are here, encrypted)",
            "Auth tag",
        ];
        for (label, segment) in labels.iter().zip(decoded.trailing_segments.iter()) {
            out.push_str(&format_segment(label, segment));
            out.push_str("\n\n");
        }
        out.push_str(
            "Note: JWE payloads cannot be unpacked without the decryption key — this is expected, not a bug.",
        );
        return out;
    }

    out.push_str(&format_segment("Header", &decoded.header));
    out.push_str("\n\n");
    out.push_str(&format_segment("Payload", &decoded.payload));
    if let Some(sig) = decoded.trailing_segments.first() {
        out.push_str("\n\n");
        match sig {
            JwtSegment::Json(_) => out.push_str(&format_segment("Signature", sig)),
            JwtSegment::Opaque { byte_length, .. } => {
                out.push_str(&format!(
                    "Signature: present ({byte_length} bytes, not shown — validity NOT verified)"
                ));
            }
        }
    }
    out.push_str(&claims);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sample_jwt() {
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2HT4u8";
        let decoded = decode_jwt_token(token).expect("decode");
        assert_eq!(decoded.kind, JwtKind::Jws);
        assert!(matches!(decoded.header, JwtSegment::Json(_)));
        assert!(matches!(decoded.payload, JwtSegment::Json(_)));
    }

    #[test]
    fn strips_bearer_prefix() {
        let token = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2HT4u8";
        let decoded = decode_jwt_token(token).expect("decode");
        if let JwtSegment::Json(payload) = decoded.payload {
            assert_eq!(payload["sub"], "1234567890");
        } else {
            panic!("expected json payload");
        }
    }

    #[test]
    fn finds_jwt_in_bearer_header() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2HT4u8";
        let found = find_jwts_in_text(text);
        assert_eq!(found.len(), 1);
        assert!(looks_like_jwt(&found[0]));
    }

    #[test]
    fn opaque_payload_does_not_fail_entire_decode() {
        // Valid JSON header + binary payload + dummy sig
        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let payload = "gIGB"; // base64url for non-utf8 bytes
        let sig = "abcd";
        let token = format!("{header}.{payload}.{sig}");
        let decoded = decode_jwt_token(&token).expect("should decode partially");
        assert!(matches!(decoded.header, JwtSegment::Json(_)));
        assert!(matches!(decoded.payload, JwtSegment::Opaque { .. }));
    }
}
