use crate::har::types::{AppSettings, HarEntryDetail, HeaderPair};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time;

use super::python_runtime;
use super::script_quality;

pub const HTTP_RESPONSE_DEFAULT: usize = 32_000;
pub const SCRIPT_OUTPUT_DEFAULT: usize = 64_000;
pub const SCRIPT_CODE_MAX: usize = 48_000;
const DEFAULT_SCRIPT_TIMEOUT_SECS: u64 = 45;
const DEFAULT_MINIMIZE_ATTEMPTS: usize = 35;

#[derive(Debug, Clone)]
pub struct HttpRequestSpec {
    pub method: String,
    pub url: String,
    pub headers: Vec<HeaderPair>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct SuccessCriteria {
    pub status_min: u16,
    pub status_max: u16,
    pub body_contains: Option<String>,
}

impl Default for SuccessCriteria {
    fn default() -> Self {
        Self {
            status_min: 200,
            status_max: 299,
            body_contains: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpExecuteResult {
    pub status: u16,
    pub status_text: String,
    pub elapsed_ms: f64,
    pub response_headers: Vec<String>,
    pub body_preview: String,
    pub body_bytes: usize,
}

/// Headers stripped automatically when relaying (handled by reqwest).
const SKIP_WHEN_SENDING: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
];

/// Likely-nonessential headers to try removing during minimization (lowercase names).
const MINIMIZE_CANDIDATES: &[&str] = &[
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "sec-fetch-dest",
    "sec-fetch-mode",
    "sec-fetch-site",
    "sec-fetch-user",
    "sec-purpose",
    "priority",
    "dnt",
    "pragma",
    "cache-control",
    "accept-language",
    "accept-encoding",
    "upgrade-insecure-requests",
    "x-client-data",
    "x-devtools-emulation-network-conditions-client-hints",
    "x-requested-with",
    "origin",
    "referer",
    "user-agent",
];

pub fn shell_escape(value: &str) -> String {
    value.replace('\'', "'\\''")
}

pub fn spec_from_entry(entry: &HarEntryDetail) -> HttpRequestSpec {
    HttpRequestSpec {
        method: entry.summary.method.clone(),
        url: entry.summary.url.clone(),
        headers: entry.request_headers.clone(),
        body: entry.request_body.clone(),
    }
}

pub fn apply_request_overrides(spec: &mut HttpRequestSpec, args: &Value) {
    if let Some(m) = args.get("method").and_then(|v| v.as_str()) {
        if !m.is_empty() {
            spec.method = m.to_ascii_uppercase();
        }
    }
    if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            spec.url = url.to_string();
        }
    }
    if let Some(body) = args.get("body").and_then(|v| v.as_str()) {
        spec.body = body.to_string();
    }
    if args.get("omit_body").and_then(|v| v.as_bool()) == Some(true) {
        spec.body.clear();
    }

    if let Some(headers) = args.get("headers").and_then(|v| v.as_array()) {
        spec.headers = parse_header_list(headers);
    }

    if let Some(omit) = args.get("header_names_to_omit").and_then(|v| v.as_array()) {
        let omit_set: HashSet<String> = omit
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .collect();
        spec.headers
            .retain(|h| !omit_set.contains(&h.name.to_ascii_lowercase()));
    }

    if let Some(only) = args.get("include_headers_only").and_then(|v| v.as_array()) {
        let keep: HashSet<String> = only
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .collect();
        spec.headers
            .retain(|h| keep.contains(&h.name.to_ascii_lowercase()));
    }
}

pub fn parse_header_list(headers: &[Value]) -> Vec<HeaderPair> {
    headers
        .iter()
        .filter_map(|h| {
            let name = h.get("name").and_then(|v| v.as_str())?;
            let value = h.get("value").and_then(|v| v.as_str()).unwrap_or("");
            Some(HeaderPair {
                name: name.to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

pub fn build_curl_from_spec(spec: &HttpRequestSpec) -> String {
    let mut parts = vec![format!("curl -X {}", spec.method)];

    for h in &spec.headers {
        if h.name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        parts.push(format!(
            "-H '{}: {}'",
            shell_escape(&h.name),
            shell_escape(&h.value)
        ));
    }

    if !spec.body.is_empty() && spec.method != "GET" && spec.method != "HEAD" {
        parts.push(format!("-d '{}'", shell_escape(&spec.body)));
    }

    parts.push(format!("'{}'", shell_escape(&spec.url)));
    parts.join(" \\\n  ")
}

pub fn meets_criteria(result: &HttpExecuteResult, criteria: &SuccessCriteria) -> bool {
    if result.status < criteria.status_min || result.status > criteria.status_max {
        return false;
    }
    if let Some(needle) = &criteria.body_contains {
        if !result.body_preview.contains(needle.as_str()) {
            return false;
        }
    }
    true
}

pub fn format_http_result(
    label: &str,
    spec: &HttpRequestSpec,
    result: &HttpExecuteResult,
) -> String {
    let curl = build_curl_from_spec(spec);
    format!(
        "{label}\n\
         HTTP {} {} ({:.0} ms)\n\n\
         Request curl:\n```bash\n{curl}\n```\n\n\
         Response headers:\n{}\n\n\
         Response body preview:\n```\n{}\n```{}",
        result.status,
        result.status_text,
        result.elapsed_ms,
        result.response_headers.join("\n"),
        result.body_preview,
        if result.status == 401 || result.status == 403 {
            script_quality::AUTH_FAILURE_REMEDIATION
        } else {
            ""
        }
    )
}

pub async fn execute_http_spec(spec: &HttpRequestSpec) -> Result<HttpExecuteResult, String> {
    execute_http_spec_with_limit(spec, HTTP_RESPONSE_DEFAULT).await
}

pub async fn execute_http_spec_with_limit(
    spec: &HttpRequestSpec,
    max_body_chars: usize,
) -> Result<HttpExecuteResult, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let method = reqwest::Method::from_bytes(spec.method.as_bytes())
        .map_err(|_| format!("Unsupported HTTP method: {}", spec.method))?;

    let mut request = client.request(method, &spec.url);

    for h in &spec.headers {
        let lower = h.name.to_ascii_lowercase();
        if SKIP_WHEN_SENDING.contains(&lower.as_str()) {
            continue;
        }
        request = request.header(&h.name, &h.value);
    }

    if !spec.body.is_empty() && spec.method != "GET" && spec.method != "HEAD" {
        request = request.body(spec.body.clone());
    }

    let started = Instant::now();
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
    let body_preview = truncate_body(&body_text, max_body_chars);

    Ok(HttpExecuteResult {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        elapsed_ms,
        response_headers: resp_headers,
        body_preview,
        body_bytes: body_bytes.len(),
    })
}

fn truncate_body(body: &str, max: usize) -> String {
    if body.len() <= max {
        body.to_string()
    } else {
        format!(
            "{}\n\n[... response truncated at {max} chars, total {} bytes ...]",
            super::preview_chars(body, max.saturating_sub(80)),
            body.len()
        )
    }
}

pub fn success_criteria_from_args(args: &Value) -> SuccessCriteria {
    let mut c = SuccessCriteria::default();
    if let Some(status) = args.get("expect_status").and_then(|v| v.as_u64()) {
        let s = status as u16;
        c.status_min = s;
        c.status_max = s;
    }
    if let (Some(min), Some(max)) = (
        args.get("expect_status_min").and_then(|v| v.as_u64()),
        args.get("expect_status_max").and_then(|v| v.as_u64()),
    ) {
        c.status_min = min as u16;
        c.status_max = max as u16;
    }
    if let Some(s) = args.get("body_contains").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            c.body_contains = Some(s.to_string());
        }
    }
    c
}

#[derive(Debug, Clone)]
pub struct HttpProbeRecord {
    pub label: String,
    pub spec: HttpRequestSpec,
    pub result: HttpExecuteResult,
}

pub async fn minimize_http_request(
    entry: &HarEntryDetail,
    criteria: &SuccessCriteria,
    max_attempts: usize,
) -> Result<(String, Vec<HttpProbeRecord>), String> {
    let index = entry.summary.index;
    let mut spec = spec_from_entry(entry);
    let mut attempts = 0usize;
    let mut probes: Vec<HttpProbeRecord> = Vec::new();

    let baseline = execute_http_spec(&spec).await?;
    probes.push(HttpProbeRecord {
        label: "minimize_http_request:baseline".into(),
        spec: spec.clone(),
        result: baseline.clone(),
    });
    if !meets_criteria(&baseline, criteria) {
        return Ok((format!(
            "Entry [{index}] baseline request did NOT meet success criteria (HTTP {}).\n\
             Cannot minimize — fix the golden request or relax expect_status / body_contains first.\n\n{}",
            baseline.status,
            format_http_result("Baseline attempt", &spec, &baseline)
        ), probes));
    }

    let mut removed: Vec<String> = Vec::new();
    let header_names: Vec<String> = spec.headers.iter().map(|h| h.name.clone()).collect();

    for name in header_names {
        if attempts >= max_attempts {
            break;
        }
        let lower = name.to_ascii_lowercase();
        if lower == "authorization"
            || lower == "cookie"
            || lower.starts_with("x-api-key")
            || lower == "content-type" && !spec.body.is_empty()
        {
            continue;
        }
        let is_candidate = MINIMIZE_CANDIDATES
            .iter()
            .any(|c| lower == *c || lower.starts_with(&format!("{c}-")));
        if !is_candidate {
            continue;
        }

        attempts += 1;
        let mut trial = spec.clone();
        trial
            .headers
            .retain(|h| !h.name.eq_ignore_ascii_case(&name));
        let result = execute_http_spec(&trial).await?;
        probes.push(HttpProbeRecord {
            label: "minimize_http_request:probe".into(),
            spec: trial.clone(),
            result: result.clone(),
        });
        if meets_criteria(&result, criteria) {
            spec = trial;
            removed.push(name);
        }
    }

    if spec.method != "GET"
        && spec.method != "HEAD"
        && !spec.body.is_empty()
        && attempts < max_attempts
    {
        attempts += 1;
        let mut trial = spec.clone();
        trial.body.clear();
        if let Ok(result) = execute_http_spec(&trial).await {
            probes.push(HttpProbeRecord {
                label: "minimize_http_request:probe".into(),
                spec: trial.clone(),
                result: result.clone(),
            });
            if meets_criteria(&result, criteria) {
                spec = trial;
                removed.push("(request body)".to_string());
            }
        }
    }

    let final_result = execute_http_spec(&spec).await?;
    probes.push(HttpProbeRecord {
        label: "minimize_http_request:final".into(),
        spec: spec.clone(),
        result: final_result.clone(),
    });
    let curl = build_curl_from_spec(&spec);

    Ok((format!(
        "Minimal request found for entry [{index}] after {attempts} live probe(s).\n\
         Removed {} nonessential part(s): {}\n\
         Remaining headers: {}\n\n\
         Minimal curl:\n```bash\n{curl}\n```\n\n\
         Verification (live):\nHTTP {} ({:.0} ms)\n\
         Body preview (first {} bytes):\n```\n{}\n```\n\n\
         Tip: call execute_http_request with header_names_to_omit / custom headers to refine further.",
        removed.len(),
        if removed.is_empty() {
            "(none — baseline was already minimal among tested headers)".to_string()
        } else {
            removed.join(", ")
        },
        if spec.headers.is_empty() {
            "(none)".to_string()
        } else {
            spec.headers
                .iter()
                .map(|h| format!("{}: {}", h.name, super::preview_chars(&h.value, 60)))
                .collect::<Vec<_>>()
                .join("; ")
        },
        final_result.status,
        final_result.elapsed_ms,
        final_result.body_bytes.min(HTTP_RESPONSE_DEFAULT),
        final_result.body_preview
    ), probes))
}

pub fn default_max_minimize_attempts(args: &Value) -> usize {
    args.get("max_attempts")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MINIMIZE_ATTEMPTS as u64)
        .clamp(5, 50) as usize
}

pub fn parse_script_run_options(args: &serde_json::Value) -> (Vec<String>, HashMap<String, String>) {
    let cli_args: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let env: HashMap<String, String> = args
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    (cli_args, env)
}

pub async fn run_agent_script(
    settings: &AppSettings,
    code: &str,
    language: &str,
    timeout_secs: u64,
    cli_args: &[String],
    env: &HashMap<String, String>,
    output_limit: usize,
    script_code_max: usize,
    script_timeout_max_secs: u64,
) -> Result<String, String> {
    if code.len() > script_code_max {
        return Err(format!(
            "Script too large ({} chars; max {script_code_max})",
            code.len()
        ));
    }

    let timeout_secs = timeout_secs.clamp(5, script_timeout_max_secs);
    let dir = std::env::temp_dir().join("haralyzer-agent-scripts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create script dir: {e}"))?;

    let id = uuid::Uuid::new_v4();
    let lang = language.to_ascii_lowercase();

    let python_runtime = if lang == "python" || lang == "py" {
        Some(python_runtime::resolve_python(settings)?)
    } else {
        None
    };

    match lang.as_str() {
        "python" | "py" => {
            let runtime = python_runtime
                .as_ref()
                .expect("python runtime resolved above");
            python_runtime::validate_python_script(runtime, code).await?;
        }
        "powershell" | "ps1" | "ps" => {
            python_runtime::validate_powershell_script(code).await?;
        }
        _ => {}
    }

    let (path, mut cmd) =
        script_command(settings, &dir, id, code, &lang, python_runtime.as_ref(), cli_args, env).await?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| script_spawn_error(&lang, &e))?;

    let output = match time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Script process error: {e}"));
        }
        Err(_) => {
            // Dropping the timed-out wait_with_output future kills the child (kill_on_drop).
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "Script timed out after {timeout_secs}s (process terminated)."
            ));
        }
    };

    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit = output.status.code().unwrap_or(-1);

    if (lang == "python" || lang == "py") && exit != 0 {
        if let (Some(runtime), Some(pkg)) = (
            python_runtime.as_ref(),
            python_runtime::parse_missing_python_module(&stderr),
        ) {
            return Ok(python_runtime::format_missing_package_stop(runtime, &pkg));
        }
    }

    let output_limit = output_limit.max(4_000);
    let stdout_max = (output_limit * 4) / 5;
    let stderr_max = output_limit.saturating_sub(stdout_max).max(2_000);

    let mut out = format!(
        "Script finished (exit code {exit}, language: {lang})\n\n\
         --- stdout ---\n{}\n\n--- stderr ---\n{}",
        truncate_body(&stdout, stdout_max),
        truncate_body(&stderr, stderr_max)
    );

    if exit != 0 {
        out.push_str("\n\n(Script exited non-zero — fix and run_script again, or ask the user for help.)");
    }

    Ok(out)
}

fn script_spawn_error(lang: &str, err: &std::io::Error) -> String {
    if lang == "python" || lang == "py" {
        format!(
            "Failed to start Python: {err}. Install Python 3 and ensure it is on PATH, \
             or set Settings → Agent Python venv to a virtualenv folder."
        )
    } else {
        format!(
            "Failed to start PowerShell: {err}. On Linux/macOS install PowerShell (pwsh) or use Python prototypes instead."
        )
    }
}

async fn script_command(
    settings: &AppSettings,
    dir: &PathBuf,
    id: uuid::Uuid,
    code: &str,
    lang: &str,
    python_runtime: Option<&python_runtime::PythonRuntime>,
    cli_args: &[String],
    env: &HashMap<String, String>,
) -> Result<(PathBuf, Command), String> {
    match lang {
        "python" | "py" => {
            let path = dir.join(format!("{id}.py"));
            std::fs::write(&path, code).map_err(|e| format!("Failed to write script: {e}"))?;
            let runtime = python_runtime
                .cloned()
                .unwrap_or_else(|| python_runtime::resolve_python(settings).expect("resolved above"));
            let mut cmd = runtime.command();
            cmd.arg(&path);
            for (k, v) in env {
                cmd.env(k, v);
            }
            for arg in cli_args {
                cmd.arg(arg);
            }
            Ok((path, cmd))
        }
        "powershell" | "ps1" | "ps" => {
            let path = dir.join(format!("{id}.ps1"));
            std::fs::write(&path, code).map_err(|e| format!("Failed to write script: {e}"))?;
            let mut cmd = python_runtime::build_powershell_command(&path).await?;
            for (k, v) in env {
                cmd.env(k, v);
            }
            for arg in cli_args {
                cmd.arg(arg);
            }
            Ok((path, cmd))
        }
        other => Err(format!(
            "Unsupported script language '{other}'. Prefer 'python' for cross-platform prototypes."
        )),
    }
}

pub fn suggested_script_language() -> &'static str {
    python_runtime::suggested_script_language()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_curl_from_spec() {
        let spec = HttpRequestSpec {
            method: "POST".into(),
            url: "https://example.com/api".into(),
            headers: vec![HeaderPair {
                name: "Content-Type".into(),
                value: "application/json".into(),
            }],
            body: r#"{"a":1}"#.into(),
        };
        let curl = build_curl_from_spec(&spec);
        assert!(curl.contains("curl -X POST"));
        assert!(curl.contains("example.com"));
    }

    #[test]
    fn omits_headers_from_spec() {
        let entry = HarEntryDetail {
            summary: crate::har::types::HarEntrySummary {
                index: 0,
                method: "GET".into(),
                url: "https://example.com".into(),
                status: 200,
                mime_type: String::new(),
                size: 0,
                time_ms: 0.0,
                started_at: None,
                is_javascript: false,
                resource_type: None,
            },
            request_headers: vec![
                HeaderPair {
                    name: "Authorization".into(),
                    value: "Bearer x".into(),
                },
                HeaderPair {
                    name: "User-Agent".into(),
                    value: "test".into(),
                },
            ],
            response_headers: vec![],
            request_body: String::new(),
            response_body: String::new(),
            js_insights: vec![],
            deobfuscated_js: None,
        };
        let mut spec = spec_from_entry(&entry);
        apply_request_overrides(
            &mut spec,
            &serde_json::json!({ "header_names_to_omit": ["User-Agent"] }),
        );
        assert_eq!(spec.headers.len(), 1);
        assert_eq!(spec.headers[0].name, "Authorization");
    }
}
