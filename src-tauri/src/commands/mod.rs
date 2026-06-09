use crate::chat::agent_state::{ChatAgentState, PendingChatAgent, CHAT_CANCELLED_ERROR};
use crate::chat::{self, AgentRunOutcome};
use crate::har::parser::stream_parse_har_with_events;
use crate::har::types::{
    build_chunks_from_entries, AnalysisProgress, AnalysisSession,
    AppSettings, ChatAgentLimitEvent, ChatContext, ChatMessage, ChatSendResult, ChatStreamEvent,
    ChatToolEvent, HarChunk, HarEntryDetail, HarEntrySummary, HarParseComplete, LlmStreamChunk,
};
use crate::llm::{self, prompt_for_chunk_type, ChatRequestMessage, OpenRouterModel, CHAT_SYSTEM_PROMPT};
use crate::AppState;
use futures::future::join_all;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Semaphore;

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<AppSettings, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_settings()
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: AppSettings) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.save_settings(&settings)
}

#[tauri::command]
pub fn list_sessions(state: State<AppState>) -> Result<Vec<AnalysisSession>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_sessions()
}

#[tauri::command]
pub fn get_session(
    state: State<AppState>,
    session_id: String,
) -> Result<Option<AnalysisSession>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_session(&session_id)
}

#[tauri::command]
pub fn get_session_entries(
    state: State<AppState>,
    session_id: String,
) -> Result<Vec<HarEntrySummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_session_entries(&session_id)
}

#[tauri::command]
pub fn get_entry_detail(
    state: State<AppState>,
    session_id: String,
    entry_index: usize,
) -> Result<Option<HarEntryDetail>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_entry_detail(&session_id, entry_index)
}

#[tauri::command]
pub fn get_session_chunks(
    state: State<AppState>,
    session_id: String,
) -> Result<Vec<HarChunk>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_session_chunks(&session_id)
}

#[tauri::command]
pub fn get_chat_messages(
    state: State<AppState>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_chat_messages(&session_id)
}

#[tauri::command]
pub async fn open_har_file(app: AppHandle) -> Result<Option<String>, String> {
    let file = app
        .dialog()
        .file()
        .add_filter("HAR files", &["har", "json"])
        .blocking_pick_file();

    Ok(file.map(|p| p.to_string()))
}

#[tauri::command]
pub async fn parse_har_file(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
) -> Result<HarParseComplete, String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err("File does not exist".to_string());
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.har")
        .to_string();

    let total_bytes = std::fs::metadata(&path)
        .map_err(|e| e.to_string())?
        .len();

    let settings = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_settings()?
    };

    let session_id = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.create_session(&file_path, &file_name, total_bytes)?
    };

    let entries = stream_parse_har_with_events(
        &app,
        &path,
        settings.filter_static_assets,
        settings.analyze_javascript,
    )?;

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.insert_entries(&session_id, &entries)?;
    }

    let complete = HarParseComplete {
        session_id: session_id.clone(),
        file_path,
        file_name,
        total_entries: entries.len(),
        total_bytes,
    };

    let _ = app.emit("har-parse-complete", &complete);
    Ok(complete)
}

#[tauri::command]
pub fn build_chunks(state: State<AppState>, session_id: String) -> Result<Vec<HarChunk>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = db.get_settings()?;
    let entries = db.get_session_entry_details(&session_id)?;

    db.clear_chunks(&session_id)?;

    let chunks = build_chunks_from_entries(&session_id, &entries, settings.chunk_max_tokens);
    db.insert_chunks(&chunks)?;
    Ok(chunks)
}

async fn run_final_synthesis(
    app: &AppHandle,
    state: &State<'_, AppState>,
    session_id: &str,
    settings: &AppSettings,
    chunk_summaries: &[(usize, String)],
    chunks_total: usize,
) -> Result<String, String> {
    let synthesis_total = llm::count_synthesis_calls(chunk_summaries.len());
    let session_id_owned = session_id.to_string();
    let app = app.clone();

    let emit_synthesis_progress = |update: llm::SynthesisProgressUpdate| {
        let message = if update.completed_calls == 0 {
            format!(
                "Preparing final report ({synthesis_total} LLM steps from {} summaries)...",
                chunk_summaries.len()
            )
        } else if update.completed_calls >= update.total_calls {
            "Finalizing report...".to_string()
        } else {
            format!(
                "Final report step {} of {} (round {}, batch {}/{})",
                update.completed_calls,
                update.total_calls,
                update.round,
                update.batch_index,
                update.batches_in_round
            )
        };

        let _ = app.emit(
            "analysis-progress",
            AnalysisProgress {
                session_id: session_id_owned.clone(),
                phase: "final".to_string(),
                chunks_done: chunks_total,
                chunks_total,
                current_chunk: None,
                message,
                synthesis_done: Some(update.completed_calls),
                synthesis_total: Some(update.total_calls),
                synthesis_round: Some(update.round),
            },
        );
    };

    let final_summary =
        llm::synthesize_final_report(settings, chunk_summaries, emit_synthesis_progress).await?;

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_session_summary(session_id, &final_summary)?;
    }

    let _ = app.emit(
        "analysis-progress",
        AnalysisProgress {
            session_id: session_id.to_string(),
            phase: "complete".to_string(),
            chunks_done: chunks_total,
            chunks_total,
            current_chunk: None,
            message: "Analysis complete".to_string(),
            synthesis_done: None,
            synthesis_total: None,
            synthesis_round: None,
        },
    );

    let _ = app.emit(
        "llm-stream",
        LlmStreamChunk {
            session_id: session_id.to_string(),
            chunk_index: -1,
            content: final_summary.clone(),
            done: true,
        },
    );

    Ok(final_summary)
}

#[tauri::command]
pub async fn finalize_analysis(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let (settings, chunks) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = db.get_settings()?;
        let chunks = db.get_session_chunks(&session_id)?;
        (settings, chunks)
    };

    if chunks.is_empty() {
        return Err("No chunks found for this session. Run Analyze first.".to_string());
    }

    let chunk_summaries: Vec<(usize, String)> = chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            c.summary
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| (i, s.clone()))
        })
        .collect();

    if chunk_summaries.is_empty() {
        return Err("No chunk summaries available. Analyze chunks before generating a report.".to_string());
    }

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_session_status(&session_id, "analyzing")?;
    }

    run_final_synthesis(
        &app,
        &state,
        &session_id,
        &settings,
        &chunk_summaries,
        chunks.len(),
    )
    .await
}

#[tauri::command]
pub async fn start_analysis(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let (settings, chunks) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = db.get_settings()?;
        let mut chunks = db.get_session_chunks(&session_id)?;
        if chunks.is_empty() {
            let entries = db.get_session_entry_details(&session_id)?;
            let built = build_chunks_from_entries(&session_id, &entries, settings.chunk_max_tokens);
            db.insert_chunks(&built)?;
            chunks = built;
        }
        (settings, chunks)
    };

    let chunks_total = chunks.len();
    let already_done = chunks
        .iter()
        .filter(|c| c.status == "done" && c.summary.as_ref().is_some_and(|s| !s.is_empty()))
        .count();
    let pending_total = chunks_total.saturating_sub(already_done);

    if pending_total == 0 {
        let chunk_summaries: Vec<(usize, String)> = chunks
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                c.summary
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| (i, s.clone()))
            })
            .collect();

        if chunk_summaries.len() != chunks_total {
            return Err(format!(
                "Only {}/{} chunks have summaries. Missing chunks will be analyzed first — press Analyze again or reset.",
                chunk_summaries.len(),
                chunks_total
            ));
        }

        {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.update_session_status(&session_id, "analyzing")?;
        }

        return run_final_synthesis(
            &app,
            &state,
            &session_id,
            &settings,
            &chunk_summaries,
            chunks_total,
        )
        .await;
    }

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_session_status(&session_id, "analyzing")?;
    }

    let progress_message = if already_done > 0 {
        format!(
            "Analyzing {pending_total} remaining chunks ({already_done} cached, up to {} concurrent)...",
            settings.max_concurrent_requests
        )
    } else {
        format!(
            "Analyzing {chunks_total} chunks in parallel (up to {} concurrent)...",
            settings.max_concurrent_requests
        )
    };

    let _ = app.emit(
        "analysis-progress",
        AnalysisProgress {
            session_id: session_id.clone(),
            phase: "chunks".to_string(),
            chunks_done: already_done,
            chunks_total,
            current_chunk: None,
            message: progress_message,
            synthesis_done: None,
            synthesis_total: None,
            synthesis_round: None,
        },
    );

    let semaphore = Arc::new(Semaphore::new(settings.max_concurrent_requests));
    let chunks_done = Arc::new(AtomicUsize::new(already_done));
    let settings = Arc::new(settings);
    let app = Arc::new(app);
    let session_id_arc = Arc::new(session_id.clone());

    let tasks: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let sem = semaphore.clone();
            let settings = settings.clone();
            let app = app.clone();
            let session_id = session_id_arc.clone();
            let chunks_done = chunks_done.clone();
            let chunk = chunk.clone();
            let chunks_total = chunks_total;

            async move {
                if chunk.status == "done" {
                    if let Some(summary) = chunk.summary.filter(|s| !s.is_empty()) {
                        let _ = app.emit(
                            "llm-stream",
                            LlmStreamChunk {
                                session_id: (*session_id).clone(),
                                chunk_index: i as i32,
                                content: summary.clone(),
                                done: true,
                            },
                        );
                        return Ok::<_, String>((i, chunk.id.clone(), summary, false));
                    }
                }

                let _permit = sem.acquire().await.map_err(|e| e.to_string())?;

                let system = prompt_for_chunk_type(&chunk.chunk_type);
                let user_content = format!(
                    "Chunk {} of {} ({} entries, type: {}):\n\n{}",
                    i + 1,
                    chunks_total,
                    chunk.entry_count,
                    chunk.chunk_type,
                    chunk.payload
                );

                let summary = llm::analyze_chunk(settings.as_ref(), system, &user_content).await?;

                let done = chunks_done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit(
                    "analysis-progress",
                    AnalysisProgress {
                        session_id: (*session_id).clone(),
                        phase: "chunks".to_string(),
                        chunks_done: done,
                        chunks_total,
                        current_chunk: Some(i),
                        message: format!(
                            "Completed chunk {} of {} ({done}/{chunks_total})",
                            i + 1,
                            chunks_total
                        ),
                        synthesis_done: None,
                        synthesis_total: None,
                        synthesis_round: None,
                    },
                );

                let _ = app.emit(
                    "llm-stream",
                    LlmStreamChunk {
                        session_id: (*session_id).clone(),
                        chunk_index: i as i32,
                        content: summary.clone(),
                        done: true,
                    },
                );

                Ok::<_, String>((i, chunk.id.clone(), summary, true))
            }
        })
        .collect();

    let results = join_all(tasks).await;
    let mut chunk_summaries: Vec<(usize, String)> = Vec::new();

    for result in results {
        let (index, chunk_id, summary, was_analyzed) = result?;
        if was_analyzed {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.update_chunk_summary(&chunk_id, &summary, "done")?;
        }
        chunk_summaries.push((index, summary));
    }

    chunk_summaries.sort_by_key(|(i, _)| *i);

    run_final_synthesis(
        &app,
        &state,
        &session_id,
        settings.as_ref(),
        &chunk_summaries,
        chunks_total,
    )
    .await
}

#[tauri::command]
pub fn reset_session_analysis(state: State<AppState>, session_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reset_session_analysis(&session_id)
}

#[tauri::command]
pub async fn send_chat_message(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    message: String,
    context: Option<ChatContext>,
    thinking_mode: bool,
) -> Result<ChatSendResult, String> {
    let (settings, session, pinned_entry_index) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = db.get_settings()?;
        let session = db
            .get_session(&session_id)?
            .ok_or_else(|| "Session not found".to_string())?;

        let pinned_entry_index = context.as_ref().and_then(|ctx| {
            if ctx.context_type == "entry" {
                ctx.entry_index
            } else {
                None
            }
        });

        (settings, session, pinned_entry_index)
    };

    let context_ref = context
        .as_ref()
        .and_then(|c| c.entry_index.map(|i| i.to_string()));

    let user_message = message.clone();
    let model = llm::resolve_chat_model(&settings, thinking_mode);

    let history = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.insert_chat_message(
            &session_id,
            "user",
            &user_message,
            context.as_ref().map(|c| c.context_type.as_str()),
            context_ref.as_deref(),
        )?;
        db.get_chat_messages(&session_id)?
    };

    state
        .chat_agents
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&session_id);

    let messages = build_chat_messages(&session, &history, pinned_entry_index);
    let step_limit = chat::resolve_agent_max_steps(&settings);

    if thinking_mode && !settings.thinking_model.trim().is_empty() {
        return run_streaming_chat(
            &app,
            &state,
            &settings,
            &session_id,
            &model,
            messages,
            thinking_mode,
            step_limit,
        )
        .await;
    }

    run_chat_agent(
        &app,
        &state,
        &settings,
        &session,
        &session_id,
        &model,
        false,
        messages,
        step_limit,
        0,
    )
    .await
}

#[tauri::command]
pub async fn continue_chat_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    thinking_mode: bool,
) -> Result<ChatSendResult, String> {
    let pending = state
        .chat_agents
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&session_id)
        .ok_or_else(|| "No pending chat agent for this session".to_string())?;

    let (settings, session) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = db.get_settings()?;
        let session = db
            .get_session(&session_id)?
            .ok_or_else(|| "Session not found".to_string())?;
        (settings, session)
    };

    let step_limit = chat::resolve_agent_max_steps(&settings);
    let model = pending.model;

    run_chat_agent(
        &app,
        &state,
        &settings,
        &session,
        &session_id,
        &model,
        thinking_mode || pending.thinking_mode,
        pending.messages,
        step_limit,
        pending.steps_used,
    )
    .await
}

#[tauri::command]
pub async fn finalize_chat_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    thinking_mode: bool,
) -> Result<ChatSendResult, String> {
    let pending = state
        .chat_agents
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&session_id)
        .ok_or_else(|| "No pending chat agent for this session".to_string())?;

    let settings = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_settings()?
    };

    let step_limit = chat::resolve_agent_max_steps(&settings);
    let model = pending.model;
    let use_thinking = thinking_mode || pending.thinking_mode;

    run_chat_agent_finalize(
        &app,
        &state,
        &settings,
        &session_id,
        &model,
        use_thinking,
        pending.messages,
        pending.reasoning_accum,
        pending.steps_used,
        step_limit,
    )
    .await
}

#[tauri::command]
pub fn cancel_chat_agent(state: State<AppState>, session_id: String) -> Result<(), String> {
    state.chat_agents.request_cancel(&session_id);
    Ok(())
}

fn emit_chat_cancelled(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        "chat-stream",
        ChatStreamEvent {
            session_id: session_id.to_string(),
            content: String::new(),
            reasoning: String::new(),
            done: true,
            message_id: None,
        },
    );
    let _ = app.emit("chat-cancelled", session_id.to_string());
}

async fn run_chat_agent(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &AppSettings,
    session: &AnalysisSession,
    session_id: &str,
    model: &str,
    thinking_mode: bool,
    messages: Vec<ChatRequestMessage>,
    step_limit: usize,
    step_offset: usize,
) -> Result<ChatSendResult, String> {
    let session_id_emit = session_id.to_string();
    let app_emit = app.clone();
    let cancel = state.chat_agents.begin_run(session_id);

    let emit_tool = |id: &str, step: usize, tool: &str, status: &str, detail: &str| {
        let _ = app_emit.emit(
            "chat-tool",
            ChatToolEvent {
                session_id: session_id_emit.clone(),
                id: id.to_string(),
                step,
                tool: tool.to_string(),
                status: status.to_string(),
                detail: detail.to_string(),
            },
        );
    };

    let outcome = chat::run_session_agent(
        state,
        settings,
        model,
        session,
        messages,
        step_limit,
        step_offset,
        thinking_mode,
        cancel,
        |id, step, tool, status, detail| emit_tool(id, step, tool, status, detail),
        |content, reasoning| {
            let _ = app_emit.emit(
                "chat-stream",
                ChatStreamEvent {
                    session_id: session_id_emit.clone(),
                    content: content.to_string(),
                    reasoning: reasoning.to_string(),
                    done: false,
                    message_id: None,
                },
            );
        },
    )
    .await;

    state.chat_agents.end_run(session_id);

    let outcome = match outcome {
        Ok(value) => value,
        Err(err) if err == CHAT_CANCELLED_ERROR => {
            emit_chat_cancelled(app, session_id);
            return Err(err);
        }
        Err(err) => return Err(err),
    };

    match outcome {
        AgentRunOutcome::Complete {
            content,
            reasoning,
            steps_used,
        } => {
            let reply = llm::format_chat_reply(&content, &reasoning, thinking_mode);

            let assistant_message = {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                db.insert_chat_message(session_id, "assistant", &reply, None, None)?
            };

            let _ = app.emit(
                "chat-stream",
                ChatStreamEvent {
                    session_id: session_id.to_string(),
                    content,
                    reasoning,
                    done: true,
                    message_id: Some(assistant_message.id),
                },
            );

            Ok(ChatSendResult {
                message: Some(assistant_message),
                needs_continue: false,
                steps_used,
                step_limit,
            })
        }
        AgentRunOutcome::StepLimitReached {
            messages,
            reasoning,
            steps_used,
        } => {
            state
                .chat_agents
                .pending
                .lock()
                .map_err(|e| e.to_string())?
                .insert(
                    session_id.to_string(),
                    PendingChatAgent {
                        messages,
                        reasoning_accum: reasoning,
                        model: model.to_string(),
                        thinking_mode,
                        steps_used,
                    },
                );

            let _ = app.emit(
                "chat-agent-limit",
                ChatAgentLimitEvent {
                    session_id: session_id.to_string(),
                    steps_used,
                    step_limit,
                },
            );

            Ok(ChatSendResult {
                message: None,
                needs_continue: true,
                steps_used,
                step_limit,
            })
        }
    }
}

async fn run_streaming_chat(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &AppSettings,
    session_id: &str,
    model: &str,
    mut messages: Vec<ChatRequestMessage>,
    thinking_mode: bool,
    step_limit: usize,
) -> Result<ChatSendResult, String> {
    if let Some(first) = messages.first_mut() {
        if first.role == "system" {
            if let Some(content) = first.content.as_mut() {
                content.push_str(llm::THINKING_CHAT_SUPPLEMENT);
            }
        }
    }

    let session_id_emit = session_id.to_string();
    let app_emit = app.clone();
    let cancel = state.chat_agents.begin_run(session_id);

    let stream_result = llm::stream_chat_cancellable(
        settings,
        model,
        messages,
        || ChatAgentState::is_cancelled(&cancel),
        |content, reasoning| {
            let _ = app_emit.emit(
                "chat-stream",
                ChatStreamEvent {
                    session_id: session_id_emit.clone(),
                    content: content.to_string(),
                    reasoning: reasoning.to_string(),
                    done: false,
                    message_id: None,
                },
            );
        },
    )
    .await;

    state.chat_agents.end_run(session_id);

    let (content, reasoning) = match stream_result {
        Ok(value) => value,
        Err(err) if err == CHAT_CANCELLED_ERROR => {
            emit_chat_cancelled(app, session_id);
            return Err(err);
        }
        Err(err) => return Err(err),
    };

    let reply = llm::format_chat_reply(&content, &reasoning, thinking_mode);

    let assistant_message = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.insert_chat_message(session_id, "assistant", &reply, None, None)?
    };

    let _ = app.emit(
        "chat-stream",
        ChatStreamEvent {
            session_id: session_id.to_string(),
            content,
            reasoning,
            done: true,
            message_id: Some(assistant_message.id),
        },
    );

    Ok(ChatSendResult {
        message: Some(assistant_message),
        needs_continue: false,
        steps_used: 0,
        step_limit,
    })
}

async fn run_chat_agent_finalize(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &AppSettings,
    session_id: &str,
    model: &str,
    thinking_mode: bool,
    messages: Vec<ChatRequestMessage>,
    reasoning_accum: String,
    steps_used: usize,
    step_limit: usize,
) -> Result<ChatSendResult, String> {
    let session_id_emit = session_id.to_string();
    let app_emit = app.clone();
    let cancel = state.chat_agents.begin_run(session_id);

    let result = chat::force_finalize_agent(
        settings,
        model,
        messages,
        reasoning_accum,
        cancel,
        |content, reasoning| {
            let _ = app_emit.emit(
                "chat-stream",
                ChatStreamEvent {
                    session_id: session_id_emit.clone(),
                    content: content.to_string(),
                    reasoning: reasoning.to_string(),
                    done: false,
                    message_id: None,
                },
            );
        },
    )
    .await;

    state.chat_agents.end_run(session_id);

    let (content, reasoning) = match result {
        Ok(value) => value,
        Err(err) if err == CHAT_CANCELLED_ERROR => {
            emit_chat_cancelled(app, session_id);
            return Err(err);
        }
        Err(err) => return Err(err),
    };

    let reply = llm::format_chat_reply(&content, &reasoning, thinking_mode);

    let assistant_message = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.insert_chat_message(session_id, "assistant", &reply, None, None)?
    };

    let _ = app.emit(
        "chat-stream",
        ChatStreamEvent {
            session_id: session_id.to_string(),
            content,
            reasoning,
            done: true,
            message_id: Some(assistant_message.id),
        },
    );

    Ok(ChatSendResult {
        message: Some(assistant_message),
        needs_continue: false,
        steps_used,
        step_limit,
    })
}

fn build_chat_messages(
    session: &AnalysisSession,
    history: &[ChatMessage],
    pinned_entry_index: Option<usize>,
) -> Vec<ChatRequestMessage> {
    let mut messages: Vec<ChatRequestMessage> = vec![ChatRequestMessage::text(
        "system",
        CHAT_SYSTEM_PROMPT,
    )];

    let mut context_block = format!(
        "HAR session metadata:\n- File: {}\n- Entries: {}\n- Status: {}\n\n\
Use tools to fetch real entry data. Do not guess URLs, headers, or bodies.\n",
        session.file_name, session.total_entries, session.status
    );

    if let Some(idx) = pinned_entry_index {
        context_block.push_str(&format!(
            "\nUser pinned entry index {idx}. Call get_entry(entry_index={idx}) for its captured request/response.\n"
        ));
    }

    messages.push(ChatRequestMessage::text(
        "user",
        format!("Background context for this HAR session:\n\n{context_block}"),
    ));
    messages.push(ChatRequestMessage::text(
        "assistant",
        "Understood. I will use the HAR tools to look up facts before answering.",
    ));

    for msg in history {
        messages.push(ChatRequestMessage::text(&msg.role, &msg.content));
    }

    messages
}

#[tauri::command]
pub fn clear_chat_messages(state: State<AppState>, session_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.clear_chat_messages(&session_id)?;
    state
        .chat_agents
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn save_report(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    let report = export_report_inner(&state, &session_id)?;

    let file = app
        .dialog()
        .file()
        .set_file_name("har-analysis-report.md")
        .add_filter("Markdown", &["md"])
        .blocking_save_file();

    if let Some(path) = file {
        let path_str = path.to_string();
        std::fs::write(&path_str, &report).map_err(|e| format!("Failed to write file: {e}"))?;
        Ok(Some(path_str))
    } else {
        Ok(None)
    }
}

fn export_report_inner(state: &State<AppState>, session_id: &str) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let session = db
        .get_session(session_id)?
        .ok_or_else(|| "Session not found".to_string())?;

    let chunks = db.get_session_chunks(session_id)?;

    let mut report = format!(
        "# HAR Analysis Report\n\n**File:** {}\n**Entries:** {}\n**Date:** {}\n\n---\n\n",
        session.file_name, session.total_entries, session.created_at
    );

    if let Some(summary) = &session.final_summary {
        report.push_str("## Final Summary\n\n");
        report.push_str(&llm::normalize_markdown_report(summary));
        report.push_str("\n\n---\n\n");
    }

    report.push_str("## Chunk Summaries\n\n");
    for chunk in &chunks {
        if let Some(summary) = &chunk.summary {
            report.push_str(&format!(
                "### Chunk {} ({} entries, {})\n\n{}\n\n",
                chunk.chunk_index + 1,
                chunk.entry_count,
                chunk.chunk_type,
                summary
            ));
        }
    }

    Ok(report)
}

#[tauri::command]
pub fn export_report(state: State<AppState>, session_id: String) -> Result<String, String> {
    export_report_inner(&state, &session_id)
}

#[tauri::command]
pub fn delete_session(state: State<AppState>, session_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_session(&session_id)
}

#[tauri::command]
pub async fn list_openrouter_models(
    state: State<'_, AppState>,
) -> Result<Vec<OpenRouterModel>, String> {
    let api_key = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_settings()?.openrouter_api_key
    };
    llm::list_models(&api_key).await
}
