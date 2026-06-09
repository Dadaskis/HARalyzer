use crate::llm::ChatRequestMessage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const CHAT_CANCELLED_ERROR: &str = "Chat cancelled by user";

pub struct PendingChatAgent {
    pub messages: Vec<ChatRequestMessage>,
    pub reasoning_accum: String,
    pub model: String,
    pub thinking_mode: bool,
    pub steps_used: usize,
}

pub struct ChatAgentState {
    pub pending: Mutex<HashMap<String, PendingChatAgent>>,
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ChatAgentState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn begin_run(&self, session_id: &str) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        let mut active = self.active.lock().expect("chat agent active lock");
        active.insert(session_id.to_string(), Arc::clone(&token));
        token
    }

    pub fn end_run(&self, session_id: &str) {
        self.active
            .lock()
            .expect("chat agent active lock")
            .remove(session_id);
    }

    pub fn request_cancel(&self, session_id: &str) -> bool {
        let cancelled = if let Some(token) = self
            .active
            .lock()
            .expect("chat agent active lock")
            .get(session_id)
        {
            token.store(true, Ordering::SeqCst);
            true
        } else {
            false
        };

        self.pending
            .lock()
            .expect("chat agent pending lock")
            .remove(session_id);

        cancelled
    }

    pub fn is_cancelled(token: &AtomicBool) -> bool {
        token.load(Ordering::SeqCst)
    }
}
