use super::agent_state::EmbedOverrides;
use super::entry_format::{self, BodyViewMode, EntryDetailLevel};
use crate::db;
use crate::har::js_analyzer::llm_body;
use crate::har::types::AnalysisSession;
use crate::AppState;
use regex::Regex;
use std::sync::OnceLock;

static EMBED_RE: OnceLock<Regex> = OnceLock::new();
static SCRIPT_FENCE_RE: OnceLock<Regex> = OnceLock::new();

fn resolve_script(
    overrides: &EmbedOverrides,
    state: &AppState,
    session_id: &str,
) -> Option<super::script_workspace::SessionScript> {
    overrides
        .script
        .clone()
        .or_else(|| state.chat_agents.get_script(session_id))
}

fn resolve_script_status(
    overrides: &EmbedOverrides,
    state: &AppState,
    session_id: &str,
) -> Option<super::agent_state::ScriptRunStatus> {
    overrides
        .script_status
        .clone()
        .or_else(|| state.chat_agents.get_script_run_status(session_id))
}

fn embed_re() -> &'static Regex {
    EMBED_RE.get_or_init(|| {
        Regex::new(r"\{\{([a-z_]+)(?::([^}]*))?\}\}")
            .expect("embed placeholder regex")
    })
}

fn script_fence_re() -> &'static Regex {
    SCRIPT_FENCE_RE.get_or_init(|| {
        Regex::new(r"(?s)```(?:python|py|powershell|ps1|ps)\n.*?```")
            .expect("script fence regex")
    })
}

/// Replace inline script fences with `{{script}}` and inject the workspace script when the
/// model pasted code instead of using embeds (avoids hallucinated or stale copies in answers).
pub fn reconcile_answer_scripts(
    content: &str,
    state: &AppState,
    session: &AnalysisSession,
    overrides: &EmbedOverrides,
) -> String {
    let script = resolve_script(overrides, state, &session.id);
    let Some(script) = script else {
        return strip_script_embeds(content);
    };

    let mut out = if script_fence_re().is_match(content) {
        script_fence_re()
            .replace_all(content, "{{script}}")
            .into_owned()
    } else {
        content.to_string()
    };

    out = dedupe_script_embeds(&out);

    let status = resolve_script_status(overrides, state, &session.id);
    let failed = status.as_ref().is_some_and(|s| !s.success || s.stub_detected);

    if !out.contains("{{script") {
        if failed || script.revision > 0 {
            out.push_str("\n\n### Script prototype\n\n{{script}}\n");
        }
    }

    out
}

/// Close any unclosed code fences before `{{script}}` embeds to prevent markdown leaking.
fn close_unclosed_fences_before_embeds(content: &str) -> String {
    let fence_re = Regex::new(r"(?m)^```").unwrap();
    let embed_re_local = Regex::new(r"\{\{(?:script|entry|js|javascript|js_snippet|cookies|headers)(?::[^}]*)?\}\}").unwrap();

    let mut out = String::with_capacity(content.len() + 64);
    let mut last_end = 0usize;

    for m in embed_re_local.find_iter(content) {
        let embed_start = m.start();
        let before = &content[last_end..embed_start];

        let fence_count = fence_re.find_iter(before).count();
        if fence_count % 2 != 0 {
            out.push_str(before);
            out.push_str("\n```\n\n");
        } else {
            out.push_str(before);
        }
        last_end = embed_start;
    }
    out.push_str(&content[last_end..]);
    out
}

/// Strip orphan closing fences that leak after embed expansion.
/// After `{{script}}` expands to a fenced block, any trailing ``` that was
/// part of the LLM's original code fence becomes orphaned markdown.
fn strip_orphan_trailing_fences(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut end = lines.len();
    while end > 0 {
        let t = lines[end - 1].trim();
        // Strip trailing empty lines, orphan fence closers, and common markdown artifacts
        if t.is_empty()
            || t == "```"
            || t.starts_with("```")
            || t.starts_with("](#)")
            || t.starts_with("*(")
            || (t.contains("**") && t.contains("120KB"))
            || t.ends_with("```.")
        {
            end -= 1;
        } else {
            break;
        }
    }
    lines[..end].join("\n")
}

/// Keep a single `{{script}}` embed — duplicate placeholders blow up answers when expanded.
pub fn dedupe_script_embeds(content: &str) -> String {
    if !content.contains("{{script") {
        return content.to_string();
    }
    let mut seen_script = false;
    embed_re()
        .replace_all(content, |caps: &regex::Captures| {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if kind != "script" {
                return caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string();
            }
            if !seen_script {
                seen_script = true;
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            } else {
                "*(same workspace script — see above)*".to_string()
            }
        })
        .into_owned()
}

fn strip_script_embeds(content: &str) -> String {
    embed_re()
        .replace_all(content, |caps: &regex::Captures| {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if kind == "script" {
                String::new()
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned()
        .replace("\n\n\n", "\n\n")
}

pub fn expand_embeds(
    content: &str,
    state: &AppState,
    session: &AnalysisSession,
    overrides: &EmbedOverrides,
) -> Result<String, String> {
    if !content.contains("{{") {
        return Ok(content.to_string());
    }

    let content = close_unclosed_fences_before_embeds(content);
    let content = dedupe_script_embeds(&content);

    let mut last_err: Option<String> = None;
    let expanded = embed_re()
        .replace_all(&content, |caps: &regex::Captures| {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let arg = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            match expand_one(kind, arg, state, session, overrides) {
                Ok(text) => text,
                Err(err) => {
                    last_err = Some(err.clone());
                    if kind == "script" {
                        "*No workspace script was saved in this chat (run_script did not store a prototype). \
                         Check earlier tool steps for script output.*"
                            .to_string()
                    } else {
                        format!("*[Could not expand `{kind}` embed: {err}]*")
                    }
                }
            }
        })
        .into_owned();

    if let Some(err) = last_err {
        eprintln!("HARalyzer: embed expansion warning: {err}");
    }

    Ok(strip_orphan_trailing_fences(&expanded))
}

fn expand_one(
    kind: &str,
    arg: &str,
    state: &AppState,
    session: &AnalysisSession,
    overrides: &EmbedOverrides,
) -> Result<String, String> {
    match kind {
        "script" => expand_script(arg, state, session, overrides),
        "entry" => expand_entry(arg, state, session),
        "js" | "javascript" => expand_js(arg, state, session),
        "js_snippet" => expand_js_snippet(arg, state, session),
        "cookies" => expand_cookies(state, session),
        "headers" => expand_headers(arg, state, session),
        other => Err(format!("Unknown embed type `{other}`")),
    }
}

fn escape_ticks(s: &str) -> String {
    s.replace("```", "'''")
}

fn expand_script(
    arg: &str,
    state: &AppState,
    session: &AnalysisSession,
    overrides: &EmbedOverrides,
) -> Result<String, String> {
    match arg.trim() {
        "" | "current" | "workspace" => {
            let script = resolve_script(overrides, state, &session.id)
                .ok_or_else(|| {
                    "No script in workspace yet — call run_script with code= first".to_string()
                })?;
            let lang_key = script.language.to_ascii_lowercase();
            let lang = match lang_key.as_str() {
                "python" | "py" => "python",
                "powershell" | "ps1" | "ps" => "powershell",
                other => other,
            };
            let status = resolve_script_status(overrides, state, &session.id);
            let status_note = match status {
                Some(s) if s.stub_detected => {
                    format!(
                        "\n\n> **Warning: script mocks/simulates or demo-fakes results** (rev {}). \
                         Last run did **not** satisfy the user's request — do not claim this works.",
                        s.revision
                    )
                }
                Some(s) if s.success => {
                    format!("\n\n*Last run: succeeded (rev {}).*", s.revision)
                }
                Some(s) if !s.stderr_excerpt.trim().is_empty() => {
                    format!(
                        "\n\n> **Warning: last run failed** (rev {}). The script below did **not** work when executed.\n\
                         > ```\n> {}\n> ```",
                        s.revision,
                        s.stderr_excerpt.trim().replace('\n', "\n> ")
                    )
                }
                Some(s) => format!(
                    "\n\n> **Warning: last run failed** (rev {}). Do not assume this script works.",
                    s.revision
                ),
                None => String::new(),
            };
            Ok(format!(
                "**Workspace script** (rev {}, {} lines){status_note}\n\n```{lang}\n{}\n```",
                script.revision,
                script.code.lines().count(),
                escape_ticks(script.code.trim_end())
            ))
        }
        "diff" => {
            let diff = state
                .chat_agents
                .get_last_script_diff(&session.id)
                .ok_or_else(|| "No script edits in this reply yet".to_string())?;
            let escaped = diff.replace("```", "'''");
            Ok(format!("**Last script edit**\n\n```diff\n{escaped}\n```"))
        }
        other => Err(format!("Unknown script embed `{other}` — use {{script}} or {{script:diff}}")),
    }
}

fn expand_entry(arg: &str, state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let (index, part) = parse_index_and_part(arg)?;
    let entry = {
        let db = db::lock_db(&state.db)?;
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry [{index}] not found"))
    }?;

    let s = &entry.summary;
    match part.as_str() {
        "" | "overview" => Ok(entry_format::format_entry_detail(
            &entry,
            EntryDetailLevel::Overview,
            None,
        )),
        "full" => Ok(entry_format::format_entry_detail(
            &entry,
            EntryDetailLevel::Full,
            None,
        )),
        "headers" | "all_headers" => {
            let mut out = format!("Entry [{index}] {} {}\n\n", s.method, s.url);
            out.push_str("Request headers:\n");
            for h in &entry.request_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
            out.push_str("\nResponse headers:\n");
            for h in &entry.response_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
            Ok(out.trim_end().to_string())
        }
        "request_headers" => format_headers_section(&entry, index, "Request", &entry.request_headers),
        "response_headers" => {
            format_headers_section(&entry, index, "Response", &entry.response_headers)
        }
        "request_body" => format_body_embed(&entry, index, "Request", &entry.request_body),
        "response_body" => format_body_embed(&entry, index, "Response", &entry.response_body),
        "cookies" => {
            let mut out = format!("Entry [{index}] {} {} — cookies\n\n", s.method, s.url);
            for h in entry
                .request_headers
                .iter()
                .chain(entry.response_headers.iter())
            {
                let lower = h.name.to_ascii_lowercase();
                if lower == "cookie" || lower == "set-cookie" {
                    out.push_str(&format!("  {}: {}\n", h.name, h.value));
                }
            }
            if out.lines().count() <= 2 {
                out.push_str("  (none in headers)\n");
            }
            Ok(out.trim_end().to_string())
        }
        other => Err(format!("Unknown entry embed part `{other}`")),
    }
}

fn expand_headers(arg: &str, state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let index: usize = arg
        .trim()
        .parse()
        .map_err(|_| format!("headers embed needs entry index, got `{arg}`"))?;
    expand_entry(&format!("{index}:headers"), state, session)
}

fn format_headers_section(
    entry: &crate::har::types::HarEntryDetail,
    index: usize,
    label: &str,
    headers: &[crate::har::types::HeaderPair],
) -> Result<String, String> {
    let s = &entry.summary;
    let mut out = format!("Entry [{index}] {} {} — {label} headers\n\n", s.method, s.url);
    for h in headers {
        out.push_str(&format!("  {}: {}\n", h.name, h.value));
    }
    Ok(out.trim_end().to_string())
}

fn format_body_embed(
    entry: &crate::har::types::HarEntryDetail,
    index: usize,
    label: &str,
    body: &str,
) -> Result<String, String> {
    let s = &entry.summary;
    Ok(format!(
        "Entry [{index}] {} {} — {label} body ({} bytes):\n\n```\n{}\n```",
        s.method,
        s.url,
        body.len(),
        escape_ticks(&entry_format::format_body(body, BodyViewMode::Preview))
    ))
}

fn expand_js(arg: &str, state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let index: usize = arg
        .trim()
        .parse()
        .map_err(|_| format!("js embed needs entry index, got `{arg}`"))?;
    let entry = {
        let db = db::lock_db(&state.db)?;
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry [{index}] not found"))
    }?;

    if !entry.summary.is_javascript {
        return Err(format!("Entry [{index}] is not JavaScript"));
    }

    if let Some(ref code) = entry.deobfuscated_js {
        if !code.trim().is_empty() {
            return Ok(format!(
                "Deobfuscated JavaScript — entry [{index}] {} {}\n\n```javascript\n{}\n```",
                entry.summary.method,
                entry.summary.url,
                escape_ticks(&llm_body(code))
            ));
        }
    }

    Ok(format!(
        "JavaScript — entry [{index}] {} {} (raw)\n\n```javascript\n{}\n```",
        entry.summary.method,
        entry.summary.url,
        escape_ticks(&llm_body(&entry.response_body))
    ))
}

fn expand_js_snippet(arg: &str, state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let mut parts = arg.splitn(2, ':');
    let index: usize = parts
        .next()
        .ok_or_else(|| "js_snippet embed needs `{{js_snippet:INDEX:start-end}}`".to_string())?
        .trim()
        .parse()
        .map_err(|_| format!("Invalid entry index in `{arg}`"))?;
    let range = parts
        .next()
        .ok_or_else(|| "js_snippet embed needs line range, e.g. {{js_snippet:5:10-25}}".to_string())?;

    let (start, end) = parse_line_range(range)?;
    let entry = {
        let db = db::lock_db(&state.db)?;
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry [{index}] not found"))
    }?;

    let source = entry
        .deobfuscated_js
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(entry.response_body.as_str());
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Err(format!("Entry [{index}] has no JavaScript source"));
    }

    let from = start.saturating_sub(1).min(lines.len());
    let to = end.max(start).min(lines.len());
    let mut out = format!(
        "JavaScript snippet — entry [{index}], lines {start}–{to} of {}:\n\n```javascript\n",
        lines.len()
    );
    for (i, line) in lines[from..to].iter().enumerate() {
        out.push_str(&format!("{:>5} | {}\n", from + i + 1, escape_ticks(line)));
    }
    out.push_str("```");
    Ok(out)
}

fn expand_cookies(state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let details = {
        let db = db::lock_db(&state.db)?;
        db.get_session_entry_details(&session.id)?
    };

    let mut events = Vec::new();
    for entry in &details {
        for h in entry
            .request_headers
            .iter()
            .chain(entry.response_headers.iter())
        {
            let lower = h.name.to_ascii_lowercase();
            if lower == "cookie" || lower == "set-cookie" {
                events.push(format!(
                    "[{}] {} {} — {}: {}",
                    entry.summary.index,
                    entry.summary.method,
                    entry.summary.url,
                    h.name,
                    h.value
                ));
            }
        }
    }

    if events.is_empty() {
        return Ok("*No Cookie/Set-Cookie headers in this session.*".to_string());
    }
    events.truncate(80);
    Ok(format!(
        "**Cookie flow** ({} events):\n\n{}",
        events.len(),
        events.join("\n")
    ))
}

fn parse_index_and_part(arg: &str) -> Result<(usize, String), String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err("entry embed needs index, e.g. {{entry:5}}".to_string());
    }
    let mut parts = arg.splitn(2, ':');
    let index: usize = parts
        .next()
        .unwrap()
        .parse()
        .map_err(|_| format!("Invalid entry index in `{arg}`"))?;
    let part = parts.next().unwrap_or("").to_string();
    Ok((index, part))
}

fn parse_line_range(range: &str) -> Result<(usize, usize), String> {
    let range = range.trim();
    if let Some((a, b)) = range.split_once('-') {
        let start: usize = a.trim().parse().map_err(|_| format!("Invalid range `{range}`"))?;
        let end: usize = b.trim().parse().map_err(|_| format!("Invalid range `{range}`"))?;
        return Ok((start, end));
    }
    let line: usize = range
        .parse()
        .map_err(|_| format!("Invalid line range `{range}` — use start-end"))?;
    Ok((line, line))
}

pub const EMBED_USAGE_GUIDE: &str = "\n\n\
Rich embeds — do NOT paste large scripts, headers, bodies, or JS inline. The app expands placeholders when rendering your answer:\n\
- `{{script}}` — current workspace script from run_script (use **once** in your answer — never repeat in usage examples or shell commands)\n\
- `{{script:diff}}` — last script edit (line diff)\n\
- `{{entry:N}}` — HAR entry [#N] overview\n\
- `{{entry:N:headers}}` / `{{entry:N:request_body}}` / `{{entry:N:response_body}}` / `{{entry:N:cookies}}`\n\
- `{{headers:N}}` — shorthand for entry headers\n\
- `{{js:N}}` — deobfuscated JS (or raw excerpt) for JS entry [#N]\n\
- `{{js_snippet:N:10-25}}` — line range from JS entry\n\
- `{{cookies}}` — session cookie flow\n\
When you built a prototype with run_script, reference it with `{{script}}` instead of repeating the code.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_script_embeds_keeps_first_only() {
        let input = "Usage:\n{{script}}\n\nAlso:\n{{script}}\n";
        let out = dedupe_script_embeds(input);
        assert_eq!(out.matches("{{script}}").count(), 1);
        assert!(out.contains("see above"));
    }

    #[test]
    fn parses_line_range() {
        assert_eq!(parse_line_range("10-25").unwrap(), (10, 25));
        assert_eq!(parse_line_range("42").unwrap(), (42, 42));
    }
}
