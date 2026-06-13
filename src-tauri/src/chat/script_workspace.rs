use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SessionScript {
    pub language: String,
    pub code: String,
    pub revision: u32,
}

#[derive(Debug, Clone)]
pub struct ScriptHistory {
    pub versions: Vec<SessionScript>,
}

impl ScriptHistory {
    pub fn push(&mut self, script: &SessionScript) {
        self.versions.push(script.clone());
        if self.versions.len() > 50 {
            self.versions.remove(0);
        }
    }

    pub fn get_revision(&self, rev: u32) -> Option<&SessionScript> {
        self.versions.iter().find(|s| s.revision == rev)
    }

    pub fn prev_revision(&self, rev: u32) -> Option<&SessionScript> {
        self.versions
            .iter()
            .rev()
            .find(|s| s.revision < rev)
    }

    pub fn format_history(&self, rev: u32, max_prev: usize) -> String {
        let prevs: Vec<&SessionScript> = self
            .versions
            .iter()
            .filter(|s| s.revision < rev)
            .rev()
            .take(max_prev)
            .collect();
        if prevs.is_empty() {
            return String::new();
        }
        let mut out = String::from("Previous script versions (most recent first):\n");
        for s in prevs {
            out.push_str(&format!(
                "--- rev {} ({}, {} lines) ---\n```{}\n{}\n```\n\n",
                s.revision,
                s.language,
                s.code.lines().count(),
                s.language,
                preview_script_code(&s.code, 80)
            ));
        }
        out
    }
}

impl Default for ScriptHistory {
    fn default() -> Self {
        Self {
            versions: Vec::new(),
        }
    }
}

fn preview_script_code(code: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = code.lines().collect();
    if lines.len() <= max_lines {
        code.to_string()
    } else {
        let head: String = lines[..max_lines / 2].join("\n");
        let tail: String = lines[lines.len() - max_lines / 2..].join("\n");
        format!(
            "{}\n\n... ({} lines omitted — see diffs in tool panel or use get_script_history) ...\n\n{}",
            head,
            lines.len() - max_lines,
            tail
        )
    }
}

pub fn resolve_script_edit(
    current: Option<&SessionScript>,
    args: &Value,
    default_language: &str,
) -> Result<(SessionScript, Option<String>), String> {
    let mut script = if args.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
        SessionScript {
            language: default_language.to_string(),
            code: String::new(),
            revision: 0,
        }
    } else {
        current
            .cloned()
            .unwrap_or(SessionScript {
                language: default_language.to_string(),
                code: String::new(),
                revision: 0,
            })
    };

    if let Some(lang) = args.get("language").and_then(|v| v.as_str()) {
        if !lang.trim().is_empty() {
            script.language = lang.to_string();
        }
    }

    let has_full = args
        .get("code")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_append = args
        .get("append_code")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let has_replacements = args
        .get("replacements")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if args.get("append_code").is_some() && !has_append {
        return Err(
            "append_code was provided but empty. Pass the new Python/PowerShell lines as a non-empty string."
                .to_string(),
        );
    }

    if !has_full && !has_append && !has_replacements && script.code.is_empty() {
        return Err(
            "No script in workspace. First run_script must include code= (full script). \
             Later runs: use append_code and/or replacements — do not resend the entire script."
                .to_string(),
        );
    }

    let code_before = script.code.clone();

    if has_full {
        if current.is_some_and(|s| !s.code.is_empty())
            && !args.get("reset").and_then(|v| v.as_bool()).unwrap_or(false)
        {
            return Err(format!(
                "Do not resend the entire script with code= (workspace already has rev {}). \
                 Use append_code and/or replacements for edits. Pass reset=true only to discard and start over.",
                script.revision
            ));
        }
        script.code = args
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        script.revision = script.revision.saturating_add(1);
    } else if script.code.is_empty() {
        return Err(
            "No stored script for this session. Pass code= to create one, or reset=true to start over."
                .to_string(),
        );
    }

    if let Some(replacements) = args.get("replacements").and_then(|v| v.as_array()) {
        for (i, rep) in replacements.iter().enumerate() {
            let find = rep
                .get("find")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("replacements[{i}] missing non-empty find"))?;
            let replace = rep.get("replace").and_then(|v| v.as_str()).unwrap_or("");
            if !script.code.contains(find) {
                return Err(format!(
                    "replacements[{i}].find not found in workspace script (rev {}, {} lines). \
                     Use a shorter unique snippet or append_code instead.",
                    script.revision,
                    script.code.lines().count()
                ));
            }
            script.code = script.code.replace(find, replace);
            script.revision = script.revision.saturating_add(1);
        }
    }

    if let Some(append) = args.get("append_code").and_then(|v| v.as_str()) {
        if !append.is_empty() {
            if !script.code.is_empty() && !script.code.ends_with('\n') {
                script.code.push('\n');
            }
            script.code.push_str(append);
            script.revision = script.revision.saturating_add(1);
        }
    }

    if script.code.trim().is_empty() {
        return Err("Script is empty after edits.".to_string());
    }

    let diff = if code_before != script.code {
        Some(format_script_diff(&code_before, &script.code))
    } else {
        None
    };

    Ok((script, diff))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffTag {
    Same,
    Add,
    Remove,
}

fn diff_line_ops<'a>(old: &'a [&'a str], new: &'a [&'a str]) -> Vec<(DiffTag, usize, &'a str)> {
    let n = old.len();
    let m = new.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j + 1]
                    .max(dp[i][j + 1])
                    .max(dp[i + 1][j])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push((DiffTag::Same, j + 1, new[j]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push((DiffTag::Remove, i + 1, old[i]));
            i += 1;
        } else {
            ops.push((DiffTag::Add, j + 1, new[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push((DiffTag::Remove, i + 1, old[i]));
        i += 1;
    }
    while j < m {
        ops.push((DiffTag::Add, j + 1, new[j]));
        j += 1;
    }
    ops
}

pub fn format_script_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    if old.trim().is_empty() {
        let mut out = String::from("All lines added (new script):\n");
        for (i, line) in new_lines.iter().enumerate() {
            out.push_str(&format!("+ {:>4} | {}\n", i + 1, line));
        }
        return out.trim_end().to_string();
    }

    let ops = diff_line_ops(&old_lines, &new_lines);
    let mut out = String::new();
    for (tag, line_no, text) in ops {
        match tag {
            DiffTag::Same => {}
            DiffTag::Add => out.push_str(&format!("+ {:>4} | {}\n", line_no, text)),
            DiffTag::Remove => out.push_str(&format!("- {:>4} | {}\n", line_no, text)),
        }
    }

    if out.is_empty() {
        "(no line changes)".to_string()
    } else {
        out.trim_end().to_string()
    }
}

fn strip_trailing_markdown_leak(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut end = lines.len();
    while end > 0 {
        let t = lines[end - 1].trim();
        if t.is_empty()
            || t == "```"
            || t.starts_with("```")
            || t.starts_with("](#)")
            || t.starts_with("*(")
            || (t.contains("**") && t.contains("120KB"))
            || t.ends_with("```.")
            || t.ends_with("```—")
            || t.ends_with("```**")
        {
            end -= 1;
        } else {
            break;
        }
    }
    lines[..end].join("\n")
}

pub fn format_script_tool_panel(
    code_diff: Option<&str>,
    script: &SessionScript,
    re_run_only: bool,
    success: bool,
    stderr: &str,
) -> String {
    let mut out = String::new();
    if let Some(d) = code_diff {
        if !d.is_empty() {
            out.push_str(&strip_trailing_markdown_leak(d));
            out.push('\n');
        }
    } else if re_run_only {
        out.push_str(&format!(
            "Re-run without code changes (rev {}, {} lines)\n",
            script.revision,
            script.code.lines().count()
        ));
    }

    if success {
        out.push_str(&format!("✓ Script exited 0 (rev {})", script.revision));
    } else {
        out.push_str(&format!("✗ Script failed (rev {})", script.revision));
        if !stderr.trim().is_empty() {
            out.push_str("\n--- stderr ---\n");
            out.push_str(&strip_trailing_markdown_leak(stderr.trim()));
        }
    }
    strip_trailing_markdown_leak(&out)
}

pub fn script_workspace_header(script: &SessionScript) -> String {
    format!(
        "Workspace script rev {} · {} · {} lines\n",
        script.revision,
        script.language,
        script.code.lines().count()
    )
}

pub fn allows_rerun_without_edit(args: &Value, force: bool) -> bool {
    if force {
        return true;
    }
    if args.get("re_run").and_then(|v| v.as_bool()).unwrap_or(false) {
        return true;
    }
    args.get("args").is_some() || args.get("env").is_some()
}

pub fn script_preview(code: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = code.lines().collect();
    if lines.len() <= max_lines {
        return code.to_string();
    }
    let mut out = lines[..max_lines].join("\n");
    out.push_str(&format!(
        "\n... ({} more lines — full source is in the workspace / {{script}} embed)",
        lines.len() - max_lines
    ));
    out
}

pub fn format_no_edit_error(script: &SessionScript) -> String {
    let preview = script_preview(&script.code, 24);
    format!(
        "No script edits in this run_script call — workspace unchanged (rev {}, {} lines).\n\
         To extend or fix the script you MUST pass one of:\n\
         • append_code=\"...\"  (non-empty string — preferred for new blocks)\n\
         • replacements=[{{\"find\":\"...\", \"replace\":\"...\"}}]\n\
         • code=\"...\" with reset=true (start over)\n\
         To re-run the same script without edits (e.g. after pip install): pass re_run=true, force=true, or args/env.\n\n\
         Current workspace script (rev {}, {} lines):\n{}",
        script.revision,
        script.code.lines().count(),
        script.revision,
        script.code.lines().count(),
        preview
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_run_requires_code() {
        let err = resolve_script_edit(None, &json!({}), "python").unwrap_err();
        assert!(err.contains("First run_script"));
    }

    #[test]
    fn append_without_resending_full_code() {
        let (first, _) = resolve_script_edit(
            None,
            &json!({"code": "print(1)\n", "language": "python"}),
            "python",
        )
        .unwrap();
        let (second, diff) = resolve_script_edit(
            Some(&first),
            &json!({"append_code": "print(2)\n"}),
            "python",
        )
        .unwrap();
        assert!(second.code.contains("print(1)"));
        assert!(second.code.contains("print(2)"));
        assert_eq!(second.revision, 2);
        assert!(diff.unwrap().contains("+"));
    }

    #[test]
    fn replacement_edit() {
        let (first, diff1) = resolve_script_edit(
            None,
            &json!({"code": "URL = \"old\"\n"}),
            "python",
        )
        .unwrap();
        assert!(diff1.unwrap().contains("All lines added"));
        let (second, diff2) = resolve_script_edit(
            Some(&first),
            &json!({"replacements": [{"find": "old", "replace": "new"}]}),
            "python",
        )
        .unwrap();
        assert!(second.code.contains("new"));
        let d = diff2.unwrap();
        assert!(d.contains("-"));
        assert!(d.contains("+"));
    }

    #[test]
    fn empty_append_code_is_rejected() {
        let (first, _) = resolve_script_edit(
            None,
            &json!({"code": "print(1)\n", "language": "python"}),
            "python",
        )
        .unwrap();
        let err = resolve_script_edit(
            Some(&first),
            &json!({"append_code": ""}),
            "python",
        )
        .unwrap_err();
        assert!(err.contains("append_code was provided but empty"));
    }

    #[test]
    fn no_edit_error_includes_preview() {
        let script = SessionScript {
            language: "python".to_string(),
            code: "print('hello')\n".to_string(),
            revision: 4,
        };
        let err = format_no_edit_error(&script);
        assert!(err.contains("append_code"));
        assert!(err.contains("print('hello')"));
    }

    #[test]
    fn blocks_resending_full_code_after_first_save() {
        let (first, _) = resolve_script_edit(
            None,
            &json!({"code": "print(1)\n", "language": "python"}),
            "python",
        )
        .unwrap();
        let err = resolve_script_edit(
            Some(&first),
            &json!({"code": "print(1)\nprint(2)\n"}),
            "python",
        )
        .unwrap_err();
        assert!(err.contains("Do not resend the entire script"));
    }

    #[test]
    fn new_script_diff_shows_all_added_lines() {
        let (_, diff) = resolve_script_edit(
            None,
            &json!({"code": "a\nb\n"}),
            "python",
        )
        .unwrap();
        let d = diff.unwrap();
        assert!(d.contains("All lines added"));
        assert!(d.contains("+    1"));
        assert!(d.contains("+    2"));
    }
}
