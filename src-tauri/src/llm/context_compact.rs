use super::model_context::{ensure_model_context, ContextBudget};
use super::{
    http_client, message_content_len, post_chat_with_retry, preview_body, prune_agent_messages,
    ChatRequestMessage,
};
use crate::har::types::AppSettings;
use std::collections::HashSet;

const COMPACT_KEEP_TOOL_ROUNDS: usize = 2;
const PROTECTED_HEAD_MESSAGES: usize = 3;
const SUMMARIZE_OUTPUT_TOKENS: u32 = 1_200;

const SUMMARIZE_SYSTEM: &str = "You compress prior HAR analysis chat and agent tool output for context limits. \
Output a dense factual summary only — no preamble. Preserve: HAR entry indices, exact URLs, HTTP status codes, \
curl/minimize findings, 504/5xx errors, JWT/auth notes, what the user asked for, and what was already tried. \
If PRIMARY USER MESSAGES are provided, include them verbatim in a **User request** bullet. \
Do not invent data. Use bullet points.";

fn is_session_bootstrap(msg: &ChatRequestMessage) -> bool {
    msg.role == "user"
        && msg.content.as_ref().is_some_and(|c| {
            c.starts_with("Background context for this HAR session:")
                || c.starts_with("[System]")
        })
}

fn collect_pinned_user_indices(
    messages: &[ChatRequestMessage],
    head: usize,
    tail_start: usize,
) -> Vec<usize> {
    let mut first_ask = None;
    let mut latest_ask = None;

    for (i, m) in messages.iter().enumerate() {
        if m.role != "user" || is_session_bootstrap(m) {
            continue;
        }
        if first_ask.is_none() {
            first_ask = Some(i);
        }
        latest_ask = Some(i);
    }

    let mut indices = HashSet::new();
    for idx in [first_ask, latest_ask].into_iter().flatten() {
        if idx >= head && idx < tail_start {
            indices.insert(idx);
        }
    }

    let mut sorted: Vec<usize> = indices.into_iter().collect();
    sorted.sort();
    sorted
}

fn serialize_range_excluding(
    msgs: &[ChatRequestMessage],
    exclude: &HashSet<usize>,
    offset: usize,
    summarize_input_max: usize,
) -> String {
    let mut out = String::new();
    for (local_i, m) in msgs.iter().enumerate() {
        if exclude.contains(&(offset + local_i)) {
            continue;
        }
        if let Some(calls) = m.tool_calls.as_ref() {
            if !calls.is_empty() {
                out.push_str("\n--- assistant (tool calls) ---\n");
                for call in calls {
                    out.push_str(&format!(
                        "{}({})\n",
                        call.function.name,
                        preview_body(&call.function.arguments, 300)
                    ));
                }
            }
        }
        if let Some(body) = m.content.as_ref() {
            if !body.is_empty() {
                out.push_str(&format!("\n--- {} ---\n{body}\n", m.role));
            }
        }
    }

    if out.len() <= summarize_input_max {
        out
    } else {
        format!(
            "{}\n\n[... truncated before summarization ...]",
            preview_body(&out, summarize_input_max.saturating_sub(48))
        )
    }
}

#[derive(Debug, Clone)]
pub struct CompactReport {
    pub removed_chars: usize,
    pub summary_preview: String,
    pub context_tokens: u32,
}

pub fn messages_total_len(messages: &[ChatRequestMessage]) -> usize {
    messages.iter().map(message_content_len).sum()
}

pub fn should_summarize_messages(messages: &[ChatRequestMessage], budget: ContextBudget) -> bool {
    messages_total_len(messages) > budget.summarize_trigger_chars
}

pub async fn compact_messages_if_needed(
    settings: &AppSettings,
    model: &str,
    messages: &mut Vec<ChatRequestMessage>,
) -> Result<Option<CompactReport>, String> {
    let budget = ensure_model_context(&settings.openrouter_api_key, model).await;
    let total = messages_total_len(messages);
    if total <= budget.summarize_trigger_chars {
        return Ok(None);
    }

    if settings.openrouter_api_key.is_empty() {
        prune_agent_messages(messages, budget);
        return Ok(None);
    }

    let head = PROTECTED_HEAD_MESSAGES.min(messages.len());
    let tail_start = find_tail_boundary(messages, head, budget);
    if tail_start <= head + 1 {
        prune_agent_messages(messages, budget);
        return Ok(None);
    }

    let pinned_indices = collect_pinned_user_indices(messages, head, tail_start);
    let pinned_set: HashSet<usize> = pinned_indices.iter().copied().collect();
    let pinned_users: Vec<ChatRequestMessage> = pinned_indices
        .iter()
        .filter_map(|i| messages.get(*i).cloned())
        .collect();

    let summarize_input_max = (budget.summarize_trigger_chars / 2).clamp(8_000, 32_000);
    let chunk = serialize_range_excluding(
        &messages[head..tail_start],
        &pinned_set,
        head,
        summarize_input_max,
    );
    if chunk.len() < 400 && pinned_users.is_empty() {
        prune_agent_messages(messages, budget);
        return Ok(None);
    }

    let summary = summarize_context(settings, &chunk, &pinned_users).await?;
    let removed_chars = messages[head..tail_start]
        .iter()
        .map(message_content_len)
        .sum();

    let summary_msg = ChatRequestMessage::text(
        "user",
        format!(
            "[Context summary — earlier chat & tool results compressed (~{}K token model budget)]\n\n{summary}\n\n\
             Messages and tool output after this summary are complete and take precedence.",
            budget.context_tokens / 1000
        ),
    );

    let mut replacement = vec![summary_msg];
    replacement.extend(pinned_users);

    messages.splice(head..tail_start, replacement);
    prune_agent_messages(messages, budget);

    Ok(Some(CompactReport {
        removed_chars,
        summary_preview: preview_body(&summary, 240),
        context_tokens: budget.context_tokens,
    }))
}

pub async fn prepare_agent_messages(
    settings: &AppSettings,
    model: &str,
    messages: &mut Vec<ChatRequestMessage>,
) -> Result<Option<CompactReport>, String> {
    let budget = ensure_model_context(&settings.openrouter_api_key, model).await;
    match compact_messages_if_needed(settings, model, messages).await? {
        Some(report) => Ok(Some(report)),
        None => {
            prune_agent_messages(messages, budget);
            Ok(None)
        }
    }
}

fn find_tail_boundary(messages: &[ChatRequestMessage], head: usize, budget: ContextBudget) -> usize {
    if messages.len() <= head {
        return head;
    }

    let segments = segment_messages(messages, head);
    if segments.is_empty() {
        return head;
    }

    let mut kept_chars = 0usize;
    let mut kept_tool_rounds = 0usize;
    let mut seg_idx = segments.len();
    let keep_tail = budget.compact_keep_tail_chars;
    let tail_overflow = keep_tail.saturating_add(4_000);

    while seg_idx > 0 {
        let (start, end) = segments[seg_idx - 1];
        let seg_len: usize = messages[start..end].iter().map(message_content_len).sum();
        let is_tool_round = messages[start]
            .tool_calls
            .as_ref()
            .is_some_and(|t| !t.is_empty());

        if kept_chars > 0
            && kept_chars + seg_len > keep_tail
            && kept_tool_rounds >= COMPACT_KEEP_TOOL_ROUNDS
        {
            break;
        }
        if kept_chars > tail_overflow {
            break;
        }

        kept_chars += seg_len;
        if is_tool_round {
            kept_tool_rounds += 1;
        }
        seg_idx -= 1;
    }

    if seg_idx >= segments.len() {
        return head;
    }

    segments[seg_idx].0
}

fn segment_messages(messages: &[ChatRequestMessage], start: usize) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut i = start;

    while i < messages.len() {
        if messages[i].role == "assistant"
            && messages[i]
                .tool_calls
                .as_ref()
                .is_some_and(|t| !t.is_empty())
        {
            let start_i = i;
            i += 1;
            let expected = messages[start_i]
                .tool_calls
                .as_ref()
                .map(|calls| calls.iter().map(|c| c.id.clone()).collect::<HashSet<_>>())
                .unwrap_or_default();
            let mut seen = HashSet::new();
            while i < messages.len() {
                if messages[i].role != "tool" {
                    break;
                }
                if let Some(id) = messages[i].tool_call_id.as_ref() {
                    seen.insert(id.clone());
                }
                i += 1;
                if !expected.is_empty() && seen.len() >= expected.len() {
                    break;
                }
            }
            segments.push((start_i, i));
            continue;
        }

        segments.push((i, i + 1));
        i += 1;
    }

    segments
}

async fn summarize_context(
    settings: &AppSettings,
    input: &str,
    pinned_users: &[ChatRequestMessage],
) -> Result<String, String> {
    let client = http_client()?;
    let mut user_body = String::new();
    if !pinned_users.is_empty() {
        user_body.push_str("PRIMARY USER MESSAGES (must appear verbatim under **User request** in your summary):\n");
        for m in pinned_users {
            if let Some(text) = m.content.as_ref() {
                user_body.push_str(text);
                user_body.push_str("\n---\n");
            }
        }
        user_body.push_str("\n");
    }
    if !input.trim().is_empty() {
        user_body.push_str("PRIOR CONTEXT TO SUMMARIZE:\n");
        user_body.push_str(input);
    } else if pinned_users.is_empty() {
        return Err("Nothing to summarize".to_string());
    }

    let messages = vec![
        ChatRequestMessage::text("system", SUMMARIZE_SYSTEM),
        ChatRequestMessage::text("user", user_body),
    ];
    let api_model = settings.resolve_api_model(&settings.default_model);
    post_chat_with_retry(
        &client,
        &settings.openrouter_api_key,
        &api_model,
        messages,
        Some(SUMMARIZE_OUTPUT_TOKENS),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ContextBudget;

    #[test]
    fn pins_first_and_latest_user_ask() {
        let messages = vec![
            ChatRequestMessage::text("system", "sys"),
            ChatRequestMessage::text("user", "Background context for this HAR session:\nmeta"),
            ChatRequestMessage::text("assistant", "ok"),
            ChatRequestMessage::text("user", "minimize entry 9 please"),
            ChatRequestMessage::text("assistant", "working"),
            ChatRequestMessage::text("user", "also check entry 1"),
            ChatRequestMessage::text("assistant", "done"),
        ];
        let pinned = collect_pinned_user_indices(&messages, 3, 6);
        assert_eq!(pinned, vec![3, 5]);
    }

    #[test]
    fn segments_tool_rounds() {
        let messages = vec![
            ChatRequestMessage::text("system", "sys"),
            ChatRequestMessage::text("user", "ctx"),
            ChatRequestMessage::text("assistant", "ok"),
            ChatRequestMessage::text("user", "hi"),
            ChatRequestMessage::assistant_tool_calls(
                vec![crate::llm::ToolCall {
                    id: "c1".into(),
                    call_type: "function".into(),
                    function: crate::llm::FunctionCall {
                        name: "list_entries".into(),
                        arguments: "{}".into(),
                    },
                }],
                None,
            ),
            ChatRequestMessage::tool_result("c1", "entries"),
            ChatRequestMessage::text("user", "latest"),
        ];
        let segs = segment_messages(&messages, 3);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], (3, 4));
        assert_eq!(segs[1], (4, 6));
        assert_eq!(segs[2], (6, 7));
    }

    #[test]
    fn large_model_triggers_summarize_later() {
        let budget_small = ContextBudget::from_context_tokens(8_192);
        let budget_large = ContextBudget::from_context_tokens(200_000);
        let messages = vec![ChatRequestMessage::text("user", "x".repeat(25_000))];
        assert!(should_summarize_messages(&messages, budget_small));
        assert!(!should_summarize_messages(&messages, budget_large));
    }
}
