use crate::har::types::{AgentLimitsSettings, AppSettings};

pub const LIMIT_FIELD_DOCS: &[(&str, &str, &str)] = &[
    (
        "max_tools_per_run",
        "Max tool calls per chat reply",
        "Total run_script / get_entry / HTTP tools the agent may call before Continue. Default 150.",
    ),
    (
        "max_tools_per_step",
        "Max parallel tools per LLM step",
        "How many tools the model may invoke in one planning round. Default 20.",
    ),
    (
        "tool_run_limit_boost",
        "Extra tools after Continue",
        "Added to the limit when you click Continue after hitting the tool cap. Default 150.",
    ),
    (
        "max_tool_messages_kept",
        "Tool results kept in context",
        "Older tool outputs are replaced with a placeholder. Default 24.",
    ),
    (
        "max_premature_nudges",
        "Planning-only nudges",
        "How often to nudge when the model stops without calling tools. Default 3.",
    ),
    (
        "list_entries_max",
        "list_entries row cap",
        "Maximum HAR rows returned by list_entries. Default 100.",
    ),
    (
        "http_response_default_chars",
        "Live HTTP body default",
        "Default max response body chars for execute_http_request. Default 32000.",
    ),
    (
        "script_output_default_chars",
        "Script stdout+stderr default",
        "Default combined script output when max_output_chars is omitted. Default 64000.",
    ),
    (
        "script_code_max_chars",
        "Max script source size",
        "Largest workspace script the agent may run. Default 48000.",
    ),
    (
        "entry_body_full_default_chars",
        "Full HAR body default",
        "Default chars for get_entry(detail=full) and get_entry_part(mode=full). Default 48000.",
    ),
    (
        "absolute_tool_output_ceiling",
        "Hard output ceiling",
        "Absolute max chars any single tool may return. Default 512000.",
    ),
    (
        "agent_planning_timeout_secs",
        "Agent LLM timeout (sec)",
        "Max wait for each agent planning request. Default 180.",
    ),
    (
        "max_script_runs_per_reply",
        "Script runs per reply",
        "Max run_script attempts before blocking without success. Default 8.",
    ),
    (
        "script_timeout_default_secs",
        "Script timeout default (sec)",
        "Default run_script timeout when not specified. Default 45.",
    ),
    (
        "script_timeout_max_secs",
        "Script timeout max (sec)",
        "Hard cap for run_script timeout. Default 120.",
    ),
    (
        "entry_body_preview_chars",
        "HAR body preview size",
        "Chars shown for get_entry overview mode. Default 600.",
    ),
    (
        "override_default_tool_output_chars",
        "Override tool output default",
        "0 = auto from model context. Otherwise fixed default for tool output scaling.",
    ),
    (
        "override_max_tool_output_chars",
        "Override tool output max",
        "0 = auto from model context. Otherwise fixed max for tool output scaling.",
    ),
    (
        "agent_stream_idle_timeout_secs",
        "Agent stream idle timeout",
        "Abort if OpenRouter sends no tokens for this many seconds. Default 35.",
    ),
];

pub fn resolve(settings: &AppSettings) -> AgentLimitsSettings {
    settings.agent_limits.clone().normalized()
}

pub fn default_tool_run_limit(settings: &AppSettings) -> usize {
    resolve(settings).max_tools_per_run
}

pub fn boosted_tool_run_limit(settings: &AppSettings, current: usize) -> usize {
    current.saturating_add(resolve(settings).tool_run_limit_boost)
}

pub type AgentLimitsSettingsExport = AgentLimitsSettings;
