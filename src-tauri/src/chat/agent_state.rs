use crate::llm::{ChatRequestMessage, ContextBudget};
use super::live_http_log::LiveHttpSessionLog;
use super::script_workspace::{ScriptHistory, SessionScript};
use super::knowledge::KnowledgeTree;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub const CHAT_CANCELLED_ERROR: &str = "Chat cancelled by user";
pub const CHAT_CANCELLED_KEEP: &str = "Chat stopped — progress kept";
pub const CHAT_CANCELLED_FINALIZE: &str = "Chat stopped — finalize requested";

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub calls: u64,
}

impl TokenUsage {
    pub fn add_input(&mut self, chars: usize) {
        self.estimated_input_tokens += (chars.max(1) / 4) as u64;
    }

    pub fn add_output(&mut self, chars: usize) {
        self.estimated_output_tokens += (chars.max(1) / 4) as u64;
        self.calls += 1;
    }

    pub fn estimated_cost_usd(&self, prompt_price: f64, completion_price: f64) -> f64 {
        let input_m = self.estimated_input_tokens as f64 / 1_000_000.0;
        let output_m = self.estimated_output_tokens as f64 / 1_000_000.0;
        input_m * prompt_price + output_m * completion_price
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatCancelMode {
    #[default]
    Abort,
    KeepProgress,
    FinalizePartial,
}

const MAX_SAME_FAILURE_REPEATS: u32 = 3;

#[derive(Clone)]
pub struct PendingChatAgent {
    pub messages: Vec<ChatRequestMessage>,
    pub reasoning_accum: String,
    pub model: String,
    pub thinking_mode: bool,
    pub steps_used: usize,
    pub tools_executed: usize,
    pub tool_run_limit: usize,
    pub pending_tool_boost: bool,
    pub paused: bool,
    pub script_snapshot: Option<SessionScript>,
    pub script_status_snapshot: Option<ScriptRunStatus>,
}

#[derive(Default, Clone)]
struct ScriptRunTracker {
    runs: u32,
    consecutive_failures: u32,
    last_failure_sig: Option<String>,
    had_success: bool,
    last_revision: u32,
}

#[derive(Clone, Default)]
pub struct ScriptRunStatus {
    pub revision: u32,
    pub success: bool,
    pub stderr_excerpt: String,
    pub stub_detected: bool,
}

#[derive(Clone, Default)]
pub struct EmbedOverrides {
    pub script: Option<SessionScript>,
    pub script_status: Option<ScriptRunStatus>,
}

impl EmbedOverrides {
    pub fn capture(state: &crate::AppState, session_id: &str) -> Self {
        Self {
            script: state.chat_agents.get_script(session_id),
            script_status: state.chat_agents.get_script_run_status(session_id),
        }
    }

    pub fn restore(&self, state: &crate::AppState, session_id: &str) {
        if let Some(script) = &self.script {
            state.chat_agents.set_script(session_id, script.clone());
        }
        if let Some(status) = &self.script_status {
            state.chat_agents.set_script_run_status(session_id, status.clone());
        }
    }
}

pub struct ChatAgentState {    pub pending: Mutex<HashMap<String, PendingChatAgent>>,
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    cancel_modes: Mutex<HashMap<String, ChatCancelMode>>,
    scripts: Mutex<HashMap<String, SessionScript>>,
    script_history: Mutex<HashMap<String, ScriptHistory>>,
    script_runs: Mutex<HashMap<String, ScriptRunTracker>>,
    last_script_diff: Mutex<HashMap<String, String>>,
    last_script_status: Mutex<HashMap<String, ScriptRunStatus>>,
    context_budgets: Mutex<HashMap<String, ContextBudget>>,
    agent_limits: Mutex<HashMap<String, crate::har::types::AgentLimitsSettings>>,
    live_http_logs: Mutex<HashMap<String, LiveHttpSessionLog>>,
    token_usage: Mutex<HashMap<String, TokenUsage>>,
    knowledge_trees: Mutex<HashMap<String, KnowledgeTree>>,
    todo_lists: Mutex<HashMap<String, TodoList>>,
}
fn lock_agent<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!(
                "HARalyzer: recovering poisoned chat agent lock (a background task panicked)"
            );
            poisoned.into_inner()
        }
    }
}

impl ChatAgentState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            cancel_modes: Mutex::new(HashMap::new()),
            scripts: Mutex::new(HashMap::new()),
            script_history: Mutex::new(HashMap::new()),
            script_runs: Mutex::new(HashMap::new()),
            last_script_diff: Mutex::new(HashMap::new()),
            last_script_status: Mutex::new(HashMap::new()),
            context_budgets: Mutex::new(HashMap::new()),
            agent_limits: Mutex::new(HashMap::new()),
            live_http_logs: Mutex::new(HashMap::new()),
            token_usage: Mutex::new(HashMap::new()),
            knowledge_trees: Mutex::new(HashMap::new()),
            todo_lists: Mutex::new(HashMap::new()),
        }
    }

    pub fn begin_run(&self, session_id: &str) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        let mut active = lock_agent(&self.active);
        active.insert(session_id.to_string(), Arc::clone(&token));
        token
    }

    pub fn end_run(&self, session_id: &str) {
        lock_agent(&self.active).remove(session_id);
        lock_agent(&self.context_budgets).remove(session_id);
        lock_agent(&self.agent_limits).remove(session_id);
        // Keep live_http_logs across runs within the same session so the agent can review prior probes.
    }

    pub fn clear_live_http_log(&self, session_id: &str) {
        lock_agent(&self.live_http_logs).remove(session_id);
    }

    pub fn with_live_http_log<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut LiveHttpSessionLog) -> R,
    ) -> R {
        let mut logs = lock_agent(&self.live_http_logs);
        let log = logs.entry(session_id.to_string()).or_default();
        f(log)
    }

    pub fn get_live_http_log(&self, session_id: &str) -> LiveHttpSessionLog {
        lock_agent(&self.live_http_logs)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_agent_limits(&self, session_id: &str, limits: crate::har::types::AgentLimitsSettings) {
        lock_agent(&self.agent_limits).insert(session_id.to_string(), limits);
    }

    pub fn get_agent_limits(&self, session_id: &str) -> crate::har::types::AgentLimitsSettings {
        lock_agent(&self.agent_limits)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_context_budget(&self, session_id: &str, budget: ContextBudget) {
        lock_agent(&self.context_budgets).insert(session_id.to_string(), budget);
    }

    pub fn get_context_budget(&self, session_id: &str) -> ContextBudget {
        lock_agent(&self.context_budgets)
            .get(session_id)
            .copied()
            .unwrap_or_else(ContextBudget::fallback)
    }

    pub fn request_cancel(&self, session_id: &str, mode: ChatCancelMode) -> bool {
        lock_agent(&self.cancel_modes).insert(session_id.to_string(), mode);

        let cancelled = if let Some(token) = lock_agent(&self.active).get(session_id) {
            token.store(true, Ordering::SeqCst);
            true
        } else {
            false
        };

        if mode == ChatCancelMode::Abort {
            lock_agent(&self.pending).remove(session_id);
        }

        cancelled
    }

    pub fn take_cancel_mode(&self, session_id: &str) -> ChatCancelMode {
        lock_agent(&self.cancel_modes)
            .remove(session_id)
            .unwrap_or(ChatCancelMode::Abort)
    }

    pub fn take_pending(&self, session_id: &str) -> Option<PendingChatAgent> {
        lock_agent(&self.pending).remove(session_id)
    }

    pub fn set_pending(&self, session_id: String, pending: PendingChatAgent) {
        lock_agent(&self.pending).insert(session_id, pending);
    }

    pub fn clear_pending(&self, session_id: &str) {
        lock_agent(&self.pending).remove(session_id);
    }

    pub fn get_script(&self, session_id: &str) -> Option<SessionScript> {
        lock_agent(&self.scripts).get(session_id).cloned()
    }

    pub fn set_script(&self, session_id: &str, script: SessionScript) {
        lock_agent(&self.scripts).insert(session_id.to_string(), script);
    }

    pub fn clear_script(&self, session_id: &str) {
        lock_agent(&self.scripts).remove(session_id);
        lock_agent(&self.script_runs).remove(session_id);
        lock_agent(&self.last_script_diff).remove(session_id);
        lock_agent(&self.last_script_status).remove(session_id);
        lock_agent(&self.script_history).remove(session_id);
    }

    pub fn push_script_to_history(&self, session_id: &str, script: SessionScript) {
        let mut hist = lock_agent(&self.script_history);
        hist.entry(session_id.to_string())
            .or_default()
            .push(&script);
    }

    pub fn get_script_history(&self, session_id: &str) -> ScriptHistory {
        lock_agent(&self.script_history)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn format_script_version_history(&self, session_id: &str, rev: u32, max_prev: usize) -> String {
        let hist = self.get_script_history(session_id);
        hist.format_history(rev, max_prev)
    }

    pub fn set_last_script_diff(&self, session_id: &str, diff: String) {
        lock_agent(&self.last_script_diff).insert(session_id.to_string(), diff);
    }

    pub fn take_last_script_diff(&self, session_id: &str) -> Option<String> {
        lock_agent(&self.last_script_diff).remove(session_id)
    }

    pub fn get_last_script_diff(&self, session_id: &str) -> Option<String> {
        lock_agent(&self.last_script_diff).get(session_id).cloned()
    }

    pub fn reset_script_run_tracker(&self, session_id: &str) {
        lock_agent(&self.script_runs).remove(session_id);
        lock_agent(&self.last_script_diff).remove(session_id);
        lock_agent(&self.last_script_status).remove(session_id);
    }

    pub fn set_script_run_status(&self, session_id: &str, status: ScriptRunStatus) {
        lock_agent(&self.last_script_status).insert(session_id.to_string(), status);
    }

    pub fn get_script_run_status(&self, session_id: &str) -> Option<ScriptRunStatus> {
        lock_agent(&self.last_script_status).get(session_id).cloned()
    }

    pub fn script_last_run_succeeded(&self, session_id: &str) -> bool {
        lock_agent(&self.last_script_status)
            .get(session_id)
            .is_some_and(|s| s.success)
    }

    pub fn should_block_script_run(
        &self,
        session_id: &str,
        force: bool,
        script_revision: u32,
    ) -> Option<String> {
        if force {
            return None;
        }
        let tracker = lock_agent(&self.script_runs);
        let Some(t) = tracker.get(session_id) else {
            return None;
        };
        let max_runs = self.get_agent_limits(session_id).max_script_runs_per_reply;
        if t.runs >= max_runs && !t.had_success {
            return Some(format!(
                "run_script blocked: {max_runs} runs this reply without success. \
                 Fix the workspace with replacements/append_code (must change rev {script_revision}), \
                 use execute_http_request for live API checks, or explain the blocker to the user. \
                 Pass force=true only after a materially different script edit."
            ));
        }
        if t.consecutive_failures >= MAX_SAME_FAILURE_REPEATS
            && t.last_revision == script_revision
        {
            return Some(format!(
                "run_script blocked: same script (rev {script_revision}) failed {MAX_SAME_FAILURE_REPEATS} times with the same error. \
                 Do NOT re-run unchanged code. Apply a different fix via replacements/append_code, \
                 or switch to execute_http_request / explain to the user. Use force=true only after editing the script."
            ));
        }
        None
    }

    pub fn record_script_run(
        &self,
        session_id: &str,
        script_revision: u32,
        success: bool,
        failure_sig: Option<&str>,
    ) -> Option<String> {
        let mut runs = lock_agent(&self.script_runs);
        let t = runs.entry(session_id.to_string()).or_default();
        t.runs += 1;
        t.last_revision = script_revision;

        if success {
            t.had_success = true;
            t.consecutive_failures = 0;
            t.last_failure_sig = None;
            return None;
        }

        let sig = failure_sig.unwrap_or("unknown").to_string();
        if t.last_failure_sig.as_deref() == Some(sig.as_str()) {
            t.consecutive_failures += 1;
        } else {
            t.consecutive_failures = 1;
            t.last_failure_sig = Some(sig);
        }

        if t.consecutive_failures >= 2 {
            Some(format!(
                "Script failed {} time(s) with similar errors (rev {script_revision}). \
                 Do NOT call run_script again without a real code change — use replacements/append_code, \
                 read stderr line numbers, or try execute_http_request.",
                t.consecutive_failures
            ))
        } else {
            None
        }
    }

    pub fn is_cancelled(token: &AtomicBool) -> bool {
        token.load(Ordering::SeqCst)
    }

    pub fn add_token_input(&self, session_id: &str, chars: usize) {
        lock_agent(&self.token_usage)
            .entry(session_id.to_string())
            .or_default()
            .add_input(chars);
    }

    pub fn add_token_output(&self, session_id: &str, chars: usize) {
        lock_agent(&self.token_usage)
            .entry(session_id.to_string())
            .or_default()
            .add_output(chars);
    }

    pub fn get_token_usage(&self, session_id: &str) -> TokenUsage {
        lock_agent(&self.token_usage)
            .get(session_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn clear_token_usage(&self, session_id: &str) {
        lock_agent(&self.token_usage).remove(session_id);
    }

    pub fn get_knowledge_tree(&self, session_id: &str) -> KnowledgeTree {
        lock_agent(&self.knowledge_trees)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn update_knowledge_tree<F>(&self, session_id: &str, update_fn: F)
    where
        F: FnOnce(&mut KnowledgeTree),
    {
        let mut trees = lock_agent(&self.knowledge_trees);
        let tree = trees.entry(session_id.to_string()).or_default();
        update_fn(tree);
    }

    pub fn clear_knowledge_tree(&self, session_id: &str) {
        lock_agent(&self.knowledge_trees).remove(session_id);
    }

    pub fn get_todo_list(&self, session_id: &str) -> TodoList {
        lock_agent(&self.todo_lists)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_todo_list(&self, session_id: &str, items: Vec<TodoItem>) {
        lock_agent(&self.todo_lists).insert(session_id.to_string(), TodoList { items });
    }

    pub fn update_todo_item(&self, session_id: &str, index: usize, status: Option<String>, notes: Option<String>) -> Result<(), String> {
        let mut lists = lock_agent(&self.todo_lists);
        let list = lists.get_mut(session_id)
            .ok_or_else(|| format!("No to-do list for session {session_id}"))?;
        let len = list.items.len();
        let item = list.items.get_mut(index)
            .ok_or_else(|| format!("Invalid index {index} (list has {len} items)"))?;
        if let Some(s) = status {
            item.status = s;
        }
        if let Some(n) = notes {
            item.notes = n;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub title: String,
    pub status: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TodoList {
    pub items: Vec<TodoItem>,
}
