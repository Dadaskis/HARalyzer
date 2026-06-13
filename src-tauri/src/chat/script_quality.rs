use regex::Regex;
use std::sync::LazyLock;

static CODE_STUB_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"(?i)\b(mock|fake|dummy|placeholder)[_\s]?(response|data|result|json|output)\b")
                .unwrap(),
            "mock/fake/placeholder response data",
        ),
        (
            Regex::new(r"(?i)#+\s*(mock|fake|stub|simulated)\b").unwrap(),
            "mock/stub comment in code",
        ),
        (
            Regex::new(r"(?i)\bsimulate(d)?\s+(the\s+)?(api|response|request|success|failure)\b").unwrap(),
            "simulated API/response",
        ),
        (
            Regex::new(r"(?i)\bdef\s+search_demo\b|\bsearch_demo\s*\(").unwrap(),
            "demo/stub search function",
        ),
        (
            Regex::new(r"(?i)\bdemo\s+mode\b").unwrap(),
            "demo mode (fake data path)",
        ),
        (
            Regex::new(r"(?i)using demo mode instead|fallback.*demo|fall back.*demo").unwrap(),
            "live failure falls back to demo data",
        ),
        (
            Regex::new(r"(?i)HAR_RESPONSE_EXAMPLE|EXAMPLE.*products.*5640007233").unwrap(),
            "hardcoded example response used as fake API data",
        ),
        (
            Regex::new(r"(?i)real HAR response structure for testing|without live API access").unwrap(),
            "fake data path labeled as testing",
        ),
        (
            Regex::new(r"(?i)\bfor\s+(demo|demonstration)\s+purposes\b").unwrap(),
            "demo-only output",
        ),
        (
            Regex::new(r"(?i)\bwould\s+have\s+returned\b").unwrap(),
            "hypothetical result text",
        ),
        (
            Regex::new(r"(?i)\bpretend\s+(the\s+)?(request|response|api|call)\b").unwrap(),
            "pretend/fake request handling",
        ),
        (
            Regex::new(r"(?i)\bhardcoded\s+(response|result|data|json)\b").unwrap(),
            "hardcoded response",
        ),
        (
            Regex::new(r"(?i)\bfrom\s+unittest\.mock\s+import|\bMagicMock\b|@patch\b").unwrap(),
            "unittest.mock test doubles",
        ),
        (
            Regex::new(r"(?i)if\s+.*\b403\b.*:\s*\n\s*(print|return)\s*\(").unwrap(),
            "returns/prints on 403 instead of fixing auth",
        ),
        (
            Regex::new(r"(?i)except\s+.*:\s*\n\s*(print|return)\s*\(").unwrap(),
            "returns/prints after exception (often fake fallback data)",
        ),
    ]
});

/// Patterns that detect scripts potentially malicious to the host OS.
static SECURITY_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r#"(?i)\b(os\.system|subprocess\.call|subprocess\.run|subprocess\.Popen)\s*\(\s*['"].*\b(rm\s+-rf|del\s+/[fqsr]|rmdir\s+/[sq]|format\s+[a-z]:|shutdown|reboot|init\s+0)\b"#).unwrap(),
            "destructive OS command detected",
        ),
        (
            Regex::new(r#"(?i)\bshutil\.rmtree\s*\(\s*['"]/(?:|usr|etc|var|opt|bin|sbin|boot|dev|proc|sys|tmp|root|home)['"]"#).unwrap(),
            "attempt to delete critical system directory",
        ),
        (
            Regex::new(r#"(?i)\bos\.(remove|unlink|rmdir)\s*\(\s*['"]/(?:etc|usr|var|boot|root|bin|sbin)['"]"#).unwrap(),
            "attempt to delete system files",
        ),
        (
            Regex::new(r#"(?i)\bos\.system\s*\(\s*['"](sudo|su\s|chmod\s+777|chown\s)"#).unwrap(),
            "privilege escalation command",
        ),
        (
            Regex::new(r#"(?i)\bexec\s*\(\s*(?:requests\.get|urllib\.request\.urlopen|http\.client)\s*\(\s*['"]https?://(?!api\.openrouter\.ai)"#).unwrap(),
            "downloads and executes remote code",
        ),
        (
            Regex::new(r"(?i)\b(eval|exec)\s*\(\s*(?:requests|urllib|http)\b").unwrap(),
            "executes fetched content as code",
        ),
        (
            Regex::new(r#"(?i)\b__import__\s*\(\s*['"]subprocess['"]\s*\)\.call\s*\(\s*['"].*\b(pip|conda)\s+install\b"#).unwrap(),
            "dynamic package installation via import",
        ),
        (
            Regex::new(r#"(?i)\bsubprocess\.(?:call|run|Popen)\s*\(\s*\[?\s*['"]pip['"]\s*,\s*['"]install['"]"#).unwrap(),
            "pip install in script (user should install manually)",
        ),
        (
            Regex::new(r#"(?i)\binput\s*\(\s*['"]password|getpass\.getpass"#).unwrap(),
            "prompts for password (potential credential theft)",
        ),
        (
            Regex::new(r#"(?i)\b(open|read)\s*\(\s*['"]/(?:etc/shadow|etc/passwd|Windows/System32/config/SAM)['"]"#).unwrap(),
            "reads sensitive system credential files",
        ),
        (
            Regex::new(r"(?i)\bos\.environ\b.*\b(requests\.post|urllib\.request\.urlopen|http\.client)\b").unwrap(),
            "exfiltrates environment variables to external server",
        ),
        (
            Regex::new(r"(?i)\b(base64\.b64decode|codecs\.decode)\s*\(.*\b(exec|eval|__import__)\s*\(").unwrap(),
            "decodes and executes obfuscated payload",
        ),
        (
            Regex::new(r"(?i)\bctypes\.(?:cdll|windll|WinDLL)\b").unwrap(),
            "direct native library access (unsafe)",
        ),
        (
            Regex::new(r"(?i)\bsocket\.(?:socket|create_connection)\b.*\b(connect|send)\b").unwrap(),
            "raw socket connection (potential reverse shell)",
        ),
        (
            Regex::new(r"(?i)\bregister\s+protocol_handler|start\s+process.*-verb\s+runas|ShellExecute.*runas").unwrap(),
            "Windows privilege escalation attempt",
        ),
    ]
});

static OUTPUT_STUB_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"(?i)(mock|simulated|fake|dummy|placeholder|demo)\s+(response|result|data|output|mode)").unwrap(),
            "stdout claims mock/simulated/demo results",
        ),
        (
            Regex::new(r"(?i)using demo mode").unwrap(),
            "stdout reports demo mode execution",
        ),
        (
            Regex::new(r"(?i)\bwould have returned\b").unwrap(),
            "stdout describes hypothetical results",
        ),
        (
            Regex::new(r"(?i)\bfor demonstration\b").unwrap(),
            "stdout marked as demonstration",
        ),
    ]
});

pub fn detect_stub_code(code: &str) -> Option<String> {
    for (re, label) in CODE_STUB_PATTERNS.iter() {
        if re.is_match(code) {
            return Some((*label).to_string());
        }
    }
    None
}

pub fn detect_stub_output(run_output: &str) -> Option<String> {
    let stdout = run_output
        .split("--- stdout ---")
        .nth(1)
        .and_then(|s| s.split("--- stderr ---").next())
        .unwrap_or("");
    let stderr = run_output.split("--- stderr ---").nth(1).unwrap_or("");
    let combined = format!("{stdout}\n{stderr}");
    for (re, label) in OUTPUT_STUB_PATTERNS.iter() {
        if re.is_match(&combined) {
            return Some((*label).to_string());
        }
    }
    None
}

pub const AUTH_FAILURE_REMEDIATION: &str = "\n\n[Auth failure — do NOT mock a successful response]\n\
Live request was rejected. Fix using evidence from this HAR capture:\n\
1. get_auth_flow + trace_cookies — find Authorization headers, cookies, CSRF tokens\n\
2. list_live_http_requests / get_live_auth_state — if an earlier live call rotated tokens, use the latest values (not stale HAR capture)\n\
3. execute_http_request(entry_index) — replay the golden entry with captured auth; strip headers only after proving they are unnecessary\n\
4. search_bodies(query=...) — locate tokens, API keys, session IDs in request/response bodies\n\
5. get_js_snippet(entry_index, search=...) — find client-side signing/auth logic\n\
6. compare_entries — diff working vs failing calls in the capture\n\
If live replay still returns 403 after using HAR auth, say so honestly in your answer — never fabricate JSON or claim success.";

pub fn format_stub_rejection(reason: &str) -> String {
    format!(
        "Script rejected: appears to mock/simulate results ({reason}).\n\
         The user expects a real working solution — do NOT substitute fake data when HTTP calls fail.\n\n\
         Required next steps (use HAR tools):\n\
         • get_auth_flow + trace_cookies\n\
         • execute_http_request with captured headers/cookies from the HAR entry\n\
         • search_bodies / get_js_snippet(search=...) to find auth tokens or client logic\n\
         • compare_entries between working and failing calls\n\n\
         Remove mock/fake/demo/placeholder responses and fix the real failure, or report honestly that live replay cannot succeed. \
         Scripts must exit non-zero when live API calls fail — never silently substitute demo data."
    )
}

pub fn format_stub_output_warning(reason: &str) -> String {
    format!(
        "\n\n[SCRIPT_QUALITY] Script exited 0 but output looks mocked/simulated ({reason}). \
         This does NOT count as success. Fix the real issue using get_auth_flow, trace_cookies, \
         execute_http_request, and search_bodies — or report failure honestly. \
         Do not present this script as working in your final answer."
    )
}

static PLACEHOLDER_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?m)^\s*#+\s*(rest\s+of\s+the\s+code|remaining\s+code)").unwrap(),
        Regex::new(r"(?m)^\s*\.{3,}\s*$").unwrap(),
        Regex::new(r"(?m)^\s*//\s*\.{3,}").unwrap(),
    ]
});

static ONE_LINER_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(if|elif|for|while|try)\b[^:]*:[^#\n]*\S").unwrap()
});

pub fn detect_placeholder_code(code: &str) -> Option<String> {
    for re in PLACEHOLDER_PATTERNS.iter() {
        if re.is_match(code) {
            return Some("Contains placeholder markers (... or # rest of code). Provide the FULL implementation — no shorthand, no placeholders.".to_string());
        }
    }
    None
}

pub fn detect_one_liners(code: &str) -> Option<String> {
    let lines: Vec<&str> = code.lines().collect();
    let mut count = 0;
    for line in &lines {
        if ONE_LINER_PATTERNS.is_match(line) {
            count += 1;
        }
    }
    if count > 3 {
        return Some(format!(
            "{count} one-liner if/for/while statements found. \
             Each statement must be on its own line with proper indentation — no `if x: do()` on one line."
        ));
    }
    None
}

static SINGLE_CHAR_VAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*([a-hl-wzA-HL-WZ])\s*=").unwrap()
});

pub fn detect_single_char_vars(code: &str) -> Option<String> {
    let mut bad_vars = Vec::new();
    for cap in SINGLE_CHAR_VAR.captures_iter(code) {
        let var = cap.get(1).unwrap().as_str();
        let lower = var.to_ascii_lowercase();
        if !["i", "j", "k", "x", "y", "z", "r", "s", "n", "f"].contains(&lower.as_str()) {
            bad_vars.push(var.to_string());
        }
    }
    if bad_vars.len() > 5 {
        return Some(format!(
            "Found {} single-character variable assignments ({}). \
             Use descriptive names — no single-char variable names except loop indices (i,j,k).",
            bad_vars.len(),
            bad_vars.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    None
}

pub fn detect_quality_issues(code: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if let Some(msg) = detect_placeholder_code(code) { issues.push(msg); }
    if let Some(msg) = detect_one_liners(code) { issues.push(msg); }
    if let Some(msg) = detect_single_char_vars(code) { issues.push(msg); }
    issues
}

pub fn detect_malicious_code(code: &str) -> Option<String> {
    for (re, label) in SECURITY_PATTERNS.iter() {
        if re.is_match(code) {
            return Some((*label).to_string());
        }
    }
    None
}

pub fn format_security_rejection(reason: &str) -> String {
    format!(
        "Script BLOCKED for security: {reason}.\n\n\
         HARalyzer executes scripts locally on the user's machine. Scripts from HAR files \
         must NEVER:\n\
         • Delete or modify system files\n\
         • Install packages (ask the user to pip install manually)\n\
         • Download and execute remote code\n\
         • Access system credentials or environment secrets\n\
         • Use privilege escalation\n\
         • Make raw socket connections\n\n\
         Rewrite the script to only read/analyze data. Network calls should be limited to \
         the target API endpoints visible in the HAR data. All file I/O must be confined \
         to the current working directory."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mock_response_in_code() {
        let code = "mock_response = {'status': 200, 'data': []}\nprint(mock_response)\n";
        assert!(detect_stub_code(code).is_some());
    }

    #[test]
    fn detects_simulated_api() {
        let code = "# simulate the api when blocked\nprint('ok')\n";
        assert!(detect_stub_code(code).is_some());
    }

    #[test]
    fn allows_real_requests_code() {
        let code = "import requests\nr = requests.get(url, headers=headers)\nprint(r.status_code, r.text)\n";
        assert!(detect_stub_code(code).is_none());
    }

    #[test]
    fn detects_yandex_demo_cli_script() {
        let code = r#"
def search_demo(query, page=1):
    return {"payload": {"state": {"search": {"screens": {}}}}}

if __name__ == "__main__":
    result = search_demo("phone")
"#;
        assert!(detect_stub_code(code).is_some());
    }

    #[test]
    fn detects_mock_stdout() {
        let out = "Script finished (exit code 0)\n\n--- stdout ---\nMock response for demo\n\n--- stderr ---\n";
        assert!(detect_stub_output(out).is_some());
    }
}
