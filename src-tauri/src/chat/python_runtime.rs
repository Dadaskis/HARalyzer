use crate::har::types::AppSettings;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const PIP_LIST_MAX_LINES: usize = 80;
const ENV_CMD_TIMEOUT_SECS: u64 = 20;
const SYNTAX_CHECK_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone)]
pub struct PythonRuntime {
    pub python: PathBuf,
    pub label: String,
}

impl PythonRuntime {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.python);
        if is_py_launcher(&self.python) {
            cmd.arg("-3");
        }
        cmd.env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");
        cmd
    }
}

fn is_py_launcher(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("py"))
        .unwrap_or(false)
}

pub fn suggested_script_language() -> &'static str {
    "python"
}

pub fn resolve_python(settings: &AppSettings) -> Result<PythonRuntime, String> {
    let venv = settings.agent_python_venv_path.trim();
    if !venv.is_empty() {
        return python_from_venv(venv);
    }

    let candidates: Vec<(&str, &str)> = if cfg!(windows) {
        vec![("py -3 launcher", "py"), ("python", "python"), ("python3", "python3")]
    } else {
        vec![("python3", "python3"), ("python", "python")]
    };

    for (label, exe) in candidates {
        if let Some(path) = find_on_path(exe) {
            return Ok(PythonRuntime {
                python: path,
                label: label.to_string(),
            });
        }
    }

    Err(missing_python_message(settings))
}

fn missing_python_message(settings: &AppSettings) -> String {
    if settings.agent_python_venv_path.trim().is_empty() {
        "Python 3 was not found on PATH (tried python3 / python / py -3). \
         Install Python 3, or set a virtualenv path in Settings → Agent Python venv."
            .to_string()
    } else {
        format!(
            "Configured venv path does not contain a Python executable: {}",
            settings.agent_python_venv_path.trim()
        )
    }
}

fn python_from_venv(venv_input: &str) -> Result<PythonRuntime, String> {
    let root = normalize_venv_root(venv_input);
    let python = venv_python_path(&root).ok_or_else(|| {
        format!(
            "No Python executable under venv path `{}`. \
             Point Settings → Agent Python venv at the venv folder (the one containing bin/ or Scripts/).",
            root.display()
        )
    })?;
    Ok(PythonRuntime {
        label: format!("venv ({})", root.display()),
        python,
    })
}

fn normalize_venv_root(input: &str) -> PathBuf {
    let p = PathBuf::from(input.trim());
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name == "python"
        || name == "python3"
        || name == "python.exe"
        || p.ends_with("bin")
        || p.ends_with("Scripts")
    {
        p.parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or(p)
    } else {
        p
    }
}

fn venv_python_path(root: &Path) -> Option<PathBuf> {
    let candidates = if cfg!(windows) {
        vec![
            root.join("Scripts").join("python.exe"),
            root.join("Scripts").join("python3.exe"),
        ]
    } else {
        vec![
            root.join("bin").join("python3"),
            root.join("bin").join("python"),
        ]
    };
    candidates.into_iter().find(|p| p.is_file())
}

fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(if cfg!(windows) {
            format!("{exe}.exe")
        } else {
            exe.to_string()
        });
        if candidate.is_file() {
            Some(candidate)
        } else {
            let plain = dir.join(exe);
            if plain.is_file() { Some(plain) } else { None }
        }
    })
}

pub async fn validate_python_script(runtime: &PythonRuntime, code: &str) -> Result<(), String> {
    if code.trim().is_empty() {
        return Err("Script is empty — provide Python source before calling run_script.".to_string());
    }

    let dir = std::env::temp_dir().join("haralyzer-agent-scripts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create script dir: {e}"))?;
    let id = uuid::Uuid::new_v4();
    let path = dir.join(format!("{id}-check.py"));
    std::fs::write(&path, code).map_err(|e| format!("Failed to write script for syntax check: {e}"))?;

    let mut cmd = runtime.command();
    cmd.args(["-m", "py_compile", path.to_str().unwrap_or_default()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let run = cmd.output();
    let output = timeout(Duration::from_secs(SYNTAX_CHECK_TIMEOUT_SECS), run)
        .await
        .map_err(|_| "Python syntax check timed out".to_string())?
        .map_err(|e| format!("Failed to run syntax check: {e}"))?;

    let _ = std::fs::remove_file(&path);

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "Script syntax check failed — fix these errors and re-read the code before calling run_script again:\n\
         {stderr}{stdout}\n\n\
         Do not run the script until syntax errors are fixed."
    ))
}

fn resolve_powershell_exe() -> Result<&'static str, String> {
    let candidates: &[&str] = if cfg!(windows) {
        &["powershell", "pwsh"]
    } else {
        &["pwsh", "powershell"]
    };

    for exe in candidates {
        if find_on_path(exe).is_some() {
            return Ok(exe);
        }
    }

    Err(
        "PowerShell was not found (tried pwsh / powershell). \
         Use Python prototypes instead — install Python 3 or set a venv in Settings."
            .to_string(),
    )
}

pub async fn validate_powershell_script(code: &str) -> Result<(), String> {
    if code.trim().is_empty() {
        return Err("Script is empty — provide PowerShell source before calling run_script.".to_string());
    }

    let dir = std::env::temp_dir().join("haralyzer-agent-scripts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create script dir: {e}"))?;
    let id = uuid::Uuid::new_v4();
    let path = dir.join(format!("{id}-check.ps1"));
    std::fs::write(&path, code).map_err(|e| format!("Failed to write script for syntax check: {e}"))?;

    let exe = resolve_powershell_exe()?;
    let mut check = Command::new(exe);
    check
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "$null = [System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$null, [ref]$errs); \
                 if ($errs) {{ $errs | ForEach-Object {{ $_.ToString() }} | Write-Output; exit 1 }}",
                path.to_string_lossy().replace('\'', "''")
            ),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let run = check.output();
    let output = timeout(Duration::from_secs(SYNTAX_CHECK_TIMEOUT_SECS), run)
        .await
        .map_err(|_| "PowerShell syntax check timed out".to_string())?
        .map_err(|e| format!("Failed to run syntax check: {e}"))?;

    let _ = std::fs::remove_file(&path);

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "Script syntax check failed — fix these errors before calling run_script again:\n\
         {stdout}{stderr}"
    ))
}

pub fn pip_install_command(runtime: &PythonRuntime, package: &str) -> String {
    let py = runtime.python.display();
    if is_py_launcher(&runtime.python) {
        format!("\"{py}\" -3 -m pip install {}", package.trim())
    } else {
        format!("\"{py}\" -m pip install {}", package.trim())
    }
}

pub fn parse_missing_python_module(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("modulenotfounderror") && !lower.contains("importerror") {
            continue;
        }
        if let Some(idx) = line.find("No module named") {
            let rest = line[idx + "No module named".len()..].trim();
            let pkg = rest
                .trim_start_matches([':', ' '])
                .trim_matches(['\'', '"', ' ']);
            let top_level = pkg.split('.').next().unwrap_or(pkg);
            if !top_level.is_empty() {
                return Some(top_level.to_string());
            }
        }
    }
    None
}

pub fn format_missing_package_stop(runtime: &PythonRuntime, package: &str) -> String {
    let install = pip_install_command(runtime, package);
    format!(
        "SCRIPT_BLOCKED — missing Python package `{package}`\n\n\
         STOP: Do not call run_script or other tools for this prototype until the user installs the package.\n\
         Tell the user to run this in their terminal, then ask them to confirm when done:\n\n\
         ```\n{install}\n```\n\n\
         Python runtime: {}\n\
         After the user confirms installation, call check_python_environment with packages: [\"{package}\"] before retrying.",
        runtime.label
    )
}

pub async fn check_python_environment(
    settings: &AppSettings,
    package_names: &[String],
) -> Result<String, String> {
    let runtime = resolve_python(settings)?;

    let version = run_capture(&runtime, &["--version"]).await?;
    let mut out = format!(
        "Python runtime: {}\nExecutable: {}\nVersion: {}\n",
        runtime.label,
        runtime.python.display(),
        version.trim()
    );

    if !settings.agent_python_venv_path.trim().is_empty() {
        out.push_str(&format!(
            "Configured venv: {}\n",
            settings.agent_python_venv_path.trim()
        ));
    }

    if package_names.is_empty() {
        let list = run_capture(
            &runtime,
            &["-m", "pip", "list", "--format=columns"],
        )
        .await
        .unwrap_or_else(|e| format!("(pip list failed: {e})"));
        let line_count = list.lines().count();
        let lines: Vec<&str> = list.lines().take(PIP_LIST_MAX_LINES).collect();
        out.push_str("\nInstalled packages (pip list, truncated):\n```\n");
        out.push_str(&lines.join("\n"));
        if line_count > PIP_LIST_MAX_LINES {
            out.push_str("\n… (truncated)");
        }
        out.push_str("\n```");
    } else {
        out.push('\n');
        for pkg in package_names {
            let show = run_capture(&runtime, &["-m", "pip", "show", pkg]).await;
            match show {
                Ok(text) if !text.trim().is_empty() && !text.contains("WARNING: Package(s) not found") => {
                    out.push_str(&format!("✓ `{pkg}` is installed\n{text}\n"));
                }
                _ => {
                    out.push_str(&format!(
                        "✗ `{pkg}` is NOT installed — user must run:\n  {}\n\n",
                        pip_install_command(&runtime, pkg)
                    ));
                }
            }
        }
    }

    Ok(out)
}

async fn run_capture(runtime: &PythonRuntime, args: &[&str]) -> Result<String, String> {
    let mut cmd = runtime.command();
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let run = cmd.output();
    let output = timeout(Duration::from_secs(ENV_CMD_TIMEOUT_SECS), run)
        .await
        .map_err(|_| format!("Command timed out after {ENV_CMD_TIMEOUT_SECS}s"))?
        .map_err(|e| format!("Failed to run {}: {e}", runtime.python.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "Command failed ({} {}): {}{}",
            runtime.python.display(),
            args.join(" "),
            stderr,
            stdout
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn build_powershell_command(script_path: &Path) -> Result<Command, String> {
    let exe = resolve_powershell_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        script_path.to_str().unwrap(),
    ]);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_module_not_found() {
        let stderr = "Traceback...\nModuleNotFoundError: No module named 'requests'\n";
        assert_eq!(parse_missing_python_module(stderr), Some("requests".into()));
    }

    #[test]
    fn parses_nested_module_not_found() {
        let stderr = "ModuleNotFoundError: No module named 'foo.bar.baz'";
        assert_eq!(parse_missing_python_module(stderr), Some("foo".into()));
    }
}
