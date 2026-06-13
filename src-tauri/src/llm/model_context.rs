use super::OpenRouterModel;
use crate::har::types::AppSettings;
use std::collections::HashMap;
use std::sync::Mutex;

/// Rough chars-per-token for context budgeting (conservative for mixed JSON/code).
const CHARS_PER_TOKEN: usize = 4;

/// When OpenRouter has no context_length for a model id.
const FALLBACK_CONTEXT_TOKENS: u32 = 64_000;

static CACHE: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    pub context_tokens: u32,
    pub hard_max_chars: usize,
    pub summarize_trigger_chars: usize,
    pub compact_keep_tail_chars: usize,
    /// Max chars kept per tool message when pruning the agent history.
    pub tool_result_max_chars: usize,
    /// Default max chars returned by a single tool call (stdout, HTTP body, entry body, …).
    pub default_tool_output_chars: usize,
    /// Max chars returned by a single tool call when the agent passes `max_output_chars`.
    pub max_tool_output_chars: usize,
    /// How many recent tool-result messages to keep before replacing with placeholders.
    pub max_tool_messages_kept: usize,
}

impl ContextBudget {
    pub fn from_context_tokens(tokens: u32) -> Self {
        Self::from_context_tokens_with_limits(tokens, None)
    }

    pub fn from_context_tokens_with_limits(
        tokens: u32,
        limits: Option<&crate::har::types::AgentLimitsSettings>,
    ) -> Self {
        let tokens = tokens.max(4_096);
        let max_chars = (tokens as usize).saturating_mul(CHARS_PER_TOKEN);
        let limits = limits.map(|l| l.clone().normalized());

        let mut default_tool_output_chars = (max_chars / 10).clamp(60_000, 400_000);
        let mut max_tool_output_chars = (max_chars / 4).clamp(120_000, 2_000_000);
        let mut max_tool_messages_kept = 32usize;

        if let Some(l) = &limits {
            if l.override_default_tool_output_chars > 0 {
                default_tool_output_chars = l.override_default_tool_output_chars;
            }
            if l.override_max_tool_output_chars > 0 {
                max_tool_output_chars = l.override_max_tool_output_chars;
            }
            max_tool_messages_kept = l.max_tool_messages_kept;
        }

        Self {
            context_tokens: tokens,
            hard_max_chars: pct(max_chars, 78).clamp(12_000, 900_000),
            summarize_trigger_chars: pct(max_chars, 62).clamp(18_000, 700_000),
            compact_keep_tail_chars: pct(max_chars, 26).clamp(8_000, 250_000),
            tool_result_max_chars: (max_chars / 6).clamp(60_000, 500_000),
            default_tool_output_chars,
            max_tool_output_chars,
            max_tool_messages_kept,
        }
    }

    pub fn fallback() -> Self {
        Self::from_context_tokens(FALLBACK_CONTEXT_TOKENS)
    }
}

fn pct(value: usize, percent: usize) -> usize {
    value.saturating_mul(percent) / 100
}

pub fn cache_model_contexts(models: &[OpenRouterModel]) {
    let mut map = HashMap::new();
    for m in models {
        if let Some(tokens) = m.context_length {
            map.insert(m.id.clone(), tokens);
        }
    }
    if let Ok(mut cache) = CACHE.lock() {
        let mut merged = cache.take().unwrap_or_default();
        merged.extend(map);
        *cache = Some(merged);
    }
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

fn cached_tokens(model_id: &str) -> Option<u32> {
    let stripped = strip_provider_suffix(model_id);
    let cache = CACHE.lock().ok()?;
    let map = cache.as_ref()?;
    map.get(stripped).copied()
}

pub fn context_tokens_for_model(model_id: &str) -> u32 {
    cached_tokens(model_id).unwrap_or(FALLBACK_CONTEXT_TOKENS)
}

pub fn budget_for_model(model_id: &str) -> ContextBudget {
    ContextBudget::from_context_tokens(context_tokens_for_model(model_id))
}

pub fn budget_for_model_and_settings(model_id: &str, settings: &AppSettings) -> ContextBudget {
    ContextBudget::from_context_tokens_with_limits(
        context_tokens_for_model(model_id),
        Some(&settings.agent_limits),
    )
}

pub async fn ensure_model_context_for_settings(
    api_key: &str,
    model_id: &str,
    settings: &AppSettings,
) -> ContextBudget {
    let stripped = strip_provider_suffix(model_id);
    if cached_tokens(stripped).is_none() && !api_key.is_empty() {
        if let Ok(models) = super::fetch_models_raw(api_key).await {
            cache_model_contexts(&models);
        }
    }
    budget_for_model_and_settings(stripped, settings)
}

pub async fn ensure_model_context(api_key: &str, model_id: &str) -> ContextBudget {
    let stripped = strip_provider_suffix(model_id);
    if cached_tokens(stripped).is_some() {
        return budget_for_model(stripped);
    }
    if !api_key.is_empty() {
        if let Ok(models) = super::fetch_models_raw(api_key).await {
            cache_model_contexts(&models);
        }
    }
    budget_for_model(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_context_summarizes_later_than_small() {
        let small = ContextBudget::from_context_tokens(8_192);
        let large = ContextBudget::from_context_tokens(200_000);
        assert!(large.summarize_trigger_chars > small.summarize_trigger_chars);
        assert!(large.hard_max_chars > small.hard_max_chars);
    }

    #[test]
    fn large_context_allows_large_tool_output() {
        let budget = ContextBudget::from_context_tokens(128_000);
        assert!(budget.default_tool_output_chars >= 30_000);
        assert!(budget.max_tool_output_chars >= 80_000);
        assert!(budget.tool_result_max_chars >= 30_000);
    }
}
