pub mod chat;
pub mod commands;
pub mod db;
pub mod har;
pub mod llm;

use chat::agent_state::ChatAgentState;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri::webview::PageLoadEvent;

pub struct AppState {
    pub db: Mutex<db::Database>,
    pub chat_agents: ChatAgentState,
}

pub struct PendingHarOpens(pub Mutex<Vec<String>>);

fn is_har_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("har") || ext.eq_ignore_ascii_case("json")
    )
}

fn normalize_launch_arg(arg: &str) -> String {
    arg.trim().trim_matches('"').trim().to_string()
}

fn path_from_launch_arg(arg: &str) -> Option<PathBuf> {
    let arg = normalize_launch_arg(arg);
    if arg.is_empty() || arg.starts_with('-') {
        return None;
    }

    if let Ok(url) = url::Url::parse(&arg) {
        if url.scheme() == "file" {
            return url.to_file_path().ok();
        }
        return None;
    }

    Some(PathBuf::from(arg))
}

fn resolve_launch_path(path: PathBuf, cwd: Option<&Path>) -> PathBuf {
    let path = if path.is_relative() {
        cwd.map(|dir| dir.join(&path)).unwrap_or(path)
    } else {
        path
    };

    #[cfg(windows)]
    {
        path.canonicalize().unwrap_or(path)
    }
    #[cfg(not(windows))]
    {
        path.canonicalize().unwrap_or(path)
    }
}

fn paths_from_args(args: impl IntoIterator<Item = String>, cwd: Option<&Path>) -> Vec<PathBuf> {
    args.into_iter()
        .filter_map(|arg| path_from_launch_arg(&arg))
        .map(|path| resolve_launch_path(path, cwd))
        .filter(|path| is_har_path(path))
        .collect()
}

fn collect_launch_files() -> Vec<PathBuf> {
    paths_from_args(std::env::args().skip(1), std::env::current_dir().ok().as_deref())
}

fn queue_har_paths(app: &AppHandle, paths: Vec<PathBuf>) -> Vec<String> {
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|path| is_har_path(path))
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    if paths.is_empty() {
        return vec![];
    }

    let pending_state = app.state::<PendingHarOpens>();
    let mut pending = pending_state
        .0
        .lock()
        .expect("pending har opens lock");

    let mut queued = Vec::new();
    for path in paths {
        if !pending.iter().any(|existing| paths_equal(existing, &path)) {
            pending.push(path.clone());
            queued.push(path);
        }
    }

    queued
}

fn emit_pending_har_files(app: &AppHandle) {
    let paths = app
        .state::<PendingHarOpens>()
        .0
        .lock()
        .expect("pending har opens lock")
        .clone();

    if !paths.is_empty() {
        let _ = app.emit("open-har-files", paths);
    }
}

fn dispatch_har_opens(app: &AppHandle, paths: Vec<PathBuf>) {
    let queued = queue_har_paths(app, paths);
    if !queued.is_empty() {
        emit_pending_har_files(app);
    }
}

fn paths_equal(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn handle_launch_args(app: &AppHandle, args: Vec<String>, cwd: Option<PathBuf>) {
    let cwd = cwd.or_else(|| std::env::current_dir().ok());
    let files = paths_from_args(args, cwd.as_deref());
    if !files.is_empty() {
        focus_main_window(app);
        dispatch_har_opens(app, files);
    }
}

#[tauri::command]
fn take_pending_har_files(state: tauri::State<'_, PendingHarOpens>) -> Vec<String> {
    state
        .0
        .lock()
        .expect("pending har opens lock")
        .clone()
}

#[tauri::command]
fn ack_pending_har_files(state: tauri::State<'_, PendingHarOpens>, paths: Vec<String>) -> Result<(), String> {
    state
        .0
        .lock()
        .expect("pending har opens lock")
        .retain(|existing| !paths.iter().any(|path| paths_equal(existing, path)));
    Ok(())
}

#[tauri::command]
fn notify_frontend_ready(app: AppHandle, state: tauri::State<'_, PendingHarOpens>) -> Result<(), String> {
    let launch_files = collect_launch_files();
    if !launch_files.is_empty() {
        queue_har_paths(&app, launch_files);
    }

    let paths = state
        .0
        .lock()
        .expect("pending har opens lock")
        .clone();

    if !paths.is_empty() {
        let _ = app.emit("open-har-files", paths);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            handle_launch_args(app, args, Some(PathBuf::from(cwd)));
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingHarOpens(Mutex::new(vec![])))
        .on_page_load(|webview, payload| {
            if payload.event() != PageLoadEvent::Finished || webview.label() != "main" {
                return;
            }
            emit_pending_har_files(webview.app_handle());
        })
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&app_data_dir).ok();
            let db_path = app_data_dir.join("haralyzer.db");
            let database = db::Database::new(&db_path).expect("failed to open database");
            app.manage(AppState {
                db: Mutex::new(database),
                chat_agents: ChatAgentState::new(),
            });

            let launch_files = collect_launch_files();
            if !launch_files.is_empty() {
                queue_har_paths(&app.handle(), launch_files);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::list_sessions,
            commands::get_session,
            commands::get_session_entries,
            commands::get_entry_detail,
            commands::get_entry_body_full,
            commands::get_session_chunks,
            commands::get_chat_messages,
            commands::get_tool_steps,
            commands::clear_chat_messages,
            commands::send_chat_message,
            commands::continue_chat_agent,
            commands::finalize_chat_agent,
            commands::cancel_chat_agent,
            commands::load_agent_script,
            commands::open_har_file,
            commands::parse_har_file,
            commands::build_chunks,
            commands::start_analysis,
            commands::finalize_analysis,
            commands::reset_session_analysis,
            commands::export_report,
            commands::save_report,
            commands::delete_session,
            commands::delete_session_entries,
            commands::restore_session_entries,
            commands::get_session_entries_snapshot,
            commands::save_har_file,
            commands::deobfuscate_js_entry,
            commands::list_openrouter_models,
            commands::get_agent_limit_docs,
            commands::append_har_file,
            commands::list_session_ids,
            take_pending_har_files,
            ack_pending_har_files,
            notify_frontend_ready,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
            if let RunEvent::Opened { urls } = event {
                let files = urls
                    .into_iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .collect::<Vec<_>>();
                dispatch_har_opens(app, files);
            }

            #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
            {
                let _ = (app, event);
            }
        });
}
