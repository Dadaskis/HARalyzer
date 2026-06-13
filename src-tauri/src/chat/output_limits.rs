use crate::llm::ContextBudget;
use serde_json::Value;

/// Default cap for `get_entry` / `get_entry_part` full body mode when the agent omits `max_output_chars`.
pub const ENTRY_BODY_FULL_DEFAULT: usize = 200_000;

/// Absolute ceiling for a single tool payload regardless of model (safety rail).
pub const ABSOLUTE_TOOL_OUTPUT_CEILING: usize = 2_000_000;

pub fn parse_max_output_chars(args: &Value) -> Option<usize> {
    args.get("max_output_chars")
        .or_else(|| args.get("max_body_chars"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .filter(|n| *n > 0)
}

/// Resolve how many characters a tool may return for this model and optional agent override.
pub fn effective_output_limit(budget: &ContextBudget, requested: Option<usize>) -> usize {
    let ceiling = budget
        .max_tool_output_chars
        .min(ABSOLUTE_TOOL_OUTPUT_CEILING);
    match requested {
        Some(n) => n.clamp(2_000, ceiling),
        None => budget.default_tool_output_chars.min(ceiling),
    }
}

pub fn truncate_output(text: &str, max_chars: usize, hint: &str) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(hint.len() + 80);
    format!(
        "{}\n\n[... truncated at {max_chars} chars ({} total). {hint} ...]",
        super::preview_chars(text, keep),
        text.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ContextBudget;

    #[test]
    fn agent_can_request_more_within_ceiling() {
        let budget = ContextBudget::from_context_tokens(128_000);
        let limit = effective_output_limit(&budget, Some(100_000));
        assert!(limit >= 50_000);
        assert!(limit <= budget.max_tool_output_chars);
    }

    #[test]
    fn default_is_much_larger_than_old_six_k() {
        let budget = ContextBudget::from_context_tokens(128_000);
        assert!(budget.default_tool_output_chars > 12_000);
        assert!(budget.tool_result_max_chars > 12_000);
    }
}
