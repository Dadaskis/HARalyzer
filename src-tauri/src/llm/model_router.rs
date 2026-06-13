use super::model_context::context_tokens_for_model;
use super::OpenRouterModel;
use crate::har::types::AppSettings;
use crate::llm::ChatRequestMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Fast,
    Balanced,
    Advanced,
}

#[derive(Debug, Clone, Default)]
pub struct AgentRoutingContext {
    pub step: usize,
    pub tools_executed: usize,
    pub script_run_attempted: bool,
    pub script_last_failed: bool,
    pub script_stub_detected: bool,
    pub user_wants_script: bool,
    pub last_tool_name: Option<String>,
    pub estimated_context_chars: usize,
}

pub fn resolve_tier_model(settings: &AppSettings, tier: ModelTier) -> String {
    let pick = |primary: &str, fallback: &str| {
        if primary.trim().is_empty() {
            fallback.to_string()
        } else {
            primary.to_string()
        }
    };
    match tier {
        ModelTier::Fast => pick(&settings.tier1_model, &settings.default_model),
        ModelTier::Balanced => pick(&settings.tier2_model, &settings.default_model),
        ModelTier::Advanced => {
            if !settings.tier3_model.trim().is_empty() {
                settings.tier3_model.clone()
            } else if !settings.thinking_model.trim().is_empty() {
                settings.thinking_model.clone()
            } else {
                pick(&settings.tier2_model, &settings.default_model)
            }
        }
    }
}

pub fn select_agent_model(
    settings: &AppSettings,
    models: &[OpenRouterModel],
    ctx: &AgentRoutingContext,
) -> (String, &'static str) {
    if !settings.smart_model_routing {
        return (
            resolve_tier_model(settings, ModelTier::Balanced),
            "fixed (routing off)",
        );
    }

    if ctx.estimated_context_chars > 120_000 {
        if let Some(id) = pick_largest_context(settings, models) {
            return (id, "large-context tier");
        }
    }

    let code_focus = ctx.user_wants_script
        || ctx.script_run_attempted
        || ctx.last_tool_name.as_deref() == Some("run_script")
        || ctx.last_tool_name.as_deref() == Some("check_python_environment");

    if code_focus {
        if ctx.script_stub_detected || ctx.script_last_failed {
            if let Some(id) = pick_code_specialist(settings, models) {
                return (id, "tier-3 code recovery");
            }
            return (
                resolve_tier_model(settings, ModelTier::Advanced),
                "tier-3 script fix",
            );
        }
        if ctx.step <= 1 {
            return (
                resolve_tier_model(settings, ModelTier::Balanced),
                "tier-2 script start",
            );
        }
        return (
            resolve_tier_model(settings, ModelTier::Advanced),
            "tier-3 script",
        );
    }

    if ctx.step <= 1 && ctx.tools_executed < 4 && !ctx.user_wants_script {
        return (
            resolve_tier_model(settings, ModelTier::Fast),
            "tier-1 discovery",
        );
    }

    if ctx.step >= 6 || ctx.script_last_failed {
        return (
            resolve_tier_model(settings, ModelTier::Advanced),
            "tier-3 escalated",
        );
    }

    (
        resolve_tier_model(settings, ModelTier::Balanced),
        "tier-2 balanced",
    )
}

fn tier_candidates(settings: &AppSettings) -> Vec<String> {
    [
        settings.tier1_model.as_str(),
        settings.tier2_model.as_str(),
        settings.tier3_model.as_str(),
        settings.default_model.as_str(),
        settings.thinking_model.as_str(),
    ]
    .into_iter()
    .filter(|s| !s.trim().is_empty())
    .map(str::to_string)
    .collect()
}

fn pick_largest_context(settings: &AppSettings, models: &[OpenRouterModel]) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for id in tier_candidates(settings) {
        let tokens = model_context_len(&id, models);
        if best.as_ref().is_none_or(|(t, _)| tokens > *t) {
            best = Some((tokens, id));
        }
    }
    best.map(|(_, id)| id)
}

fn pick_code_specialist(settings: &AppSettings, models: &[OpenRouterModel]) -> Option<String> {
    for m in models {
        if tier_candidates(settings).iter().any(|id| id == &m.id) && m.capabilities.code_focused {
            return Some(m.id.clone());
        }
    }
    for id in tier_candidates(settings) {
        let lower = id.to_ascii_lowercase();
        if lower.contains("codex")
            || lower.contains("coder")
            || lower.contains("deepseek")
            || lower.contains("qwen")
        {
            return Some(id);
        }
    }
    None
}

fn model_context_len(id: &str, models: &[OpenRouterModel]) -> u32 {
    let stripped = strip_provider_suffix(id);
    models
        .iter()
        .find(|m| m.id == stripped)
        .and_then(|m| m.context_length)
        .unwrap_or_else(|| context_tokens_for_model(stripped))
}

fn strip_provider_suffix(id: &str) -> &str {
    if let Some(pos) = id.rfind(':') {
        let base = &id[..pos];
        if base.contains('/') {
            return base;
        }
    }
    id
}

pub fn estimate_context_chars(messages: &[ChatRequestMessage]) -> usize {
    messages
        .iter()
        .map(|m| m.content.as_deref().map(str::len).unwrap_or(0))
        .sum()
}

pub fn user_wants_script_from_messages(messages: &[ChatRequestMessage]) -> bool {
    super::user_wants_script_prototype(messages)
}
