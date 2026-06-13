use crate::har::jwt::{
    decode_jwt_token, format_decoded_jwt, normalize_jwt_token, scan_entry_for_jwts,
};
use crate::har::js_analyzer::llm_body;
use crate::har::types::{AnalysisSession, HeaderPair};
use scraper::{Html, Selector};
use scraper::node::Node as ScraperNode;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::AppState;
use super::entry_format::{format_body, format_body_limited, BodyViewMode};
use super::output_limits;
use super::{arg_entry_index, format_entry_line, preview_chars, safe_byte_slice, with_db};

pub fn extra_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "edit_script",
                "description": "Edit the workspace script WITHOUT executing it. Use this to create, refine, or fix the script — then call run_script to execute. Rules: first call uses code= (full script). Later calls use append_code and/or replacements. Pass reset=true to start over.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Full script source — first call only (creates workspace)" },
                        "review": { "type": "boolean", "description": "Return the full current workspace script for review (default false)" },
                        "replacements": {
                            "type": "array",
                            "description": "Search/replace edits applied in order on the workspace script",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "find": { "type": "string" },
                                    "replace": { "type": "string" }
                                },
                                "required": ["find", "replace"]
                            }
                        },
                        "append_code": { "type": "string", "description": "Lines to append to the workspace script" },
                        "reset": { "type": "boolean", "description": "Clear workspace before applying edits" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_script_history",
                "description": "Show previous script versions from this session's workspace history. Call before writing a replacement script (reset=true) to avoid repeating past mistakes or regressing improvements.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "max_prev": { "type": "integer", "description": "Max previous versions to show (default 5, max 10)" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "compare_sessions",
                "description": "Compare the current HAR session with another HAR file. Open a different HAR, then use this tool to get a summary of differences: entry counts, unique/overlapping URLs, status distributions, new or missing endpoints.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "other_session_id": { "type": "string", "description": "Session ID of the other HAR file to compare against" }
                    },
                    "required": ["other_session_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_token_usage",
                "description": "Get estimated token usage and cost for the current chat session. Returns input/output token estimates and approximate cost at the model's rate.",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_entry_part",
                "description": "Fetch one slice of a HAR entry: headers, cookies, or a single body. For bodies, default mode=preview (~600 chars). Use mode=summary for JSON/HTML structure; mode=full for a larger slice. Pass max_output_chars to raise the cap (scales with model context).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "part": {
                            "type": "string",
                            "enum": ["request_headers", "response_headers", "request_body", "response_body", "cookies", "all_headers"]
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["preview", "summary", "full"],
                            "description": "Body parts only. preview (default), summary (structure/stats), full (large slice — use max_output_chars for crawlers)."
                        },
                        "max_output_chars": {
                            "type": "integer",
                            "description": "Optional body char limit for mode=full (default scales with model context)."
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." },
                        "data_path": { "type": "string", "description": "Optional dot-notation path to extract a subset from JSON bodies (e.g. 'results.0.name', 'items[*].id'). Use when bodies are too large to dump entirely." }
                    },
                    "required": ["entry_index", "part"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "summarize_entries",
                "description": "Compact overview of multiple entries by index (method, URL, status, size).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_indices": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Up to 50 entry indices"
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_indices"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "trace_cookies",
                "description": "Trace Set-Cookie and Cookie headers across the session.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name_filter": { "type": "string" },
                        "limit": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "trace_storage",
                "description": "Find localStorage/sessionStorage/document.cookie usage in JS and Set-Cookie headers.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_js_scripts",
                "description": "Overview of all JavaScript entries with detected pattern counts.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_js_call_map",
                "description": "Map JS entries to URLs/endpoints they reference (fetch, XHR, axios, WebSocket).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "Optional single JS entry" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_chunk_details",
                "description": "Chunk metadata plus payload preview showing which entries were analyzed in that chunk.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "chunk_index": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["chunk_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_endpoints",
                "description": "Group unique URL paths with methods and entry indices.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "limit": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "search_bodies",
                "description": "Search request/response bodies in this HAR for a substring (tokens, API keys, error messages, cookie names). Use when debugging 403/auth or finding how the client builds requests.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "in_request": { "type": "boolean" },
                        "in_response": { "type": "boolean" },
                        "limit": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "compare_entries",
                "description": "Compare two entries: status, body sizes, request header diffs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index_a": { "type": "integer" },
                        "entry_index_b": { "type": "integer" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index_a", "entry_index_b"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_auth_flow",
                "description": "Auth signals: Authorization headers, API keys, Set-Cookie, 401/403 responses.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_deobfuscated_js",
                "description": "Get LLM-deobfuscated JavaScript for an entry. Automatically runs deobfuscation via OpenRouter when not cached, then saves the result.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "JavaScript entry index" },
                        "force": { "type": "boolean", "description": "Re-run deobfuscation even if cached (default false)" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "decode_jwt",
                "description": "Decode JWT header and payload from a raw token and/or scan HAR entries for JWTs. NEVER guess JWT claims — always use this tool and include decoded JSON in your answer.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string", "description": "Raw JWT or Bearer-prefixed token to decode" },
                        "entry_index": { "type": "integer", "description": "Scan one entry's headers/bodies for JWTs and decode each" },
                        "scan_session": { "type": "boolean", "description": "Scan all session entries for JWTs (default false)" },
                        "limit": { "type": "integer", "description": "Max JWTs to decode when scanning (default 10, max 25)" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_js_snippet",
                "description": "Extract a line-numbered JavaScript snippet from raw or deobfuscated source. Use to quote exact code in answers.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer" },
                        "start_line": { "type": "integer", "description": "1-based start line (optional)" },
                        "end_line": { "type": "integer", "description": "1-based end line inclusive (optional)" },
                        "search": { "type": "string", "description": "Find first line containing this text and return surrounding context" },
                        "context_lines": { "type": "integer", "description": "Lines of context around search hit (default 8)" },
                        "use_deobfuscated": { "type": "boolean", "description": "Prefer deobfuscated source when available (default true)" },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_knowledge",
                "description": "Retrieve the persistent knowledge tree for this session. Returns accumulated observations, inferred facts, and discovered patterns about the HAR (auth flows, endpoints, schemas, errors). Call this BEFORE starting a new task to avoid re-discovering facts.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "update_knowledge",
                "description": "Store important discoveries in the persistent knowledge tree. Use this IMMEDIATELY when you find reusable facts: auth patterns, API endpoints, error causes, data schemas. Each update needs: category (e.g., 'auth', 'endpoints'), key (unique identifier), content (the fact), confidence ('observed'/'inferred'/'confirmed'), and optional source reference.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "updates": {
                            "type": "array",
                            "description": "List of knowledge updates to apply",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "category": { "type": "string", "description": "Category name (e.g., 'auth', 'endpoints', 'schemas', 'errors')" },
                                    "key": { "type": "string", "description": "Unique key within the category" },
                                    "content": { "type": "string", "description": "The fact/observation to store" },
                                    "confidence": { "type": "string", "description": "Confidence level: 'observed' (from tool), 'inferred' (educated guess), 'confirmed' (verified)" },
                                    "source": { "type": "string", "description": "Optional source reference (e.g., 'entry #42', 'get_auth_flow result')" }
                                },
                                "required": ["category", "key", "content", "confidence"]
                            }
                        }
                    },
                    "required": ["updates"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "set_todo_list",
                "description": "Create or replace the task planning to-do list. Break complex multi-step work into manageable items. Each item has: title (what to do), status (pending/in_progress/done/blocked), notes (discoveries, blockers, or hints for the next step).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "array",
                            "description": "List of to-do items",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "title": { "type": "string", "description": "What needs to be done" },
                                    "status": { "type": "string", "description": "One of: pending, in_progress, done, blocked" },
                                    "notes": { "type": "string", "description": "Optional notes, discoveries, or blockers" }
                                },
                                "required": ["title", "status"]
                            }
                        }
                    },
                    "required": ["items"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "update_todo_item",
                "description": "Update the status and/or notes of a specific to-do item by its index.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer", "description": "Zero-based index of the item to update" },
                        "status": { "type": "string", "description": "New status: pending, in_progress, done, blocked" },
                        "notes": { "type": "string", "description": "Updated notes for this item" }
                    },
                    "required": ["index"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "walk_json",
                "description": "Explore a JSON request/response body piece by piece at a given dot-notation path. Use to discover the structure of huge JSONs (15 MB+) without dumping them into context. Each call shows: keys + types for objects, length + first-item shape for arrays, or the actual value for scalars. Walk deeper by adding path segments like 'data.users.0.profile'. Use path='__keys__' to list root keys only.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "HAR entry index" },
                        "part": {
                            "type": "string",
                            "enum": ["request_body", "response_body"],
                            "description": "Which body to explore"
                        },
                        "path": {
                            "type": "string",
                            "description": "Dot-notation path to explore (default: root). Numeric segments index into arrays, string segments index into objects. Examples: '', 'data', 'items.0', 'results.5.user.name'. Special: '__keys__' lists root keys without walking into values."
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index", "part"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "walk_html",
                "description": "Explore a huge HTML response body (500K+ chars) without loading it all into context. Discover the DOM structure, find elements by CSS selector, and extract snippets. Use 'list_tags' first to see what tags exist and how many; then 'query_selectors' to try CSS selectors on live elements; then 'extract' to pull targeted content. This avoids context blowout from massive HTML dumps.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entry_index": { "type": "integer", "description": "HAR entry index" },
                        "part": {
                            "type": "string",
                            "enum": ["request_body", "response_body"],
                            "description": "Which body to explore"
                        },
                        "action": {
                            "type": "string",
                            "enum": ["list_tags", "query_selectors", "extract"],
                            "description": "list_tags: show all unique HTML tag names with counts. query_selectors: test one or more CSS selectors, returning matching count + first-match info. extract: get text/attr from elements matching a selector."
                        },
                        "selectors": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "For query_selectors action: one or more CSS selectors to test (e.g. ['div.product', 'span.price', 'a[href*=api]']). For extract action: one selector to extract from."
                        },
                        "attr": {
                            "type": "string",
                            "description": "For extract action: attribute name to pull (e.g. 'href', 'src', 'data-id'). Omit to extract text content."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results to return (default 20, max 50)"
                        },
                        "session_id": { "type": "string", "description": "Optional: session ID of another HAR to query." }
                    },
                    "required": ["entry_index", "part", "action"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_packages",
                "description": "List all installed Python packages (pip list output as JSON). Call BEFORE writing any script with imports — check which libraries are available. Prefer specialized libraries (beautifulsoup4 for HTML, playwright for browser, jq for JSON) over reinventing the wheel.",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
    ]
}

pub fn execute_extra_tool(
    state: &AppState,
    session: &AnalysisSession,
    tool_name: &str,
    args: &Value,
) -> Result<String, String> {
    match tool_name {
        "get_entry_part" => tool_get_entry_part(state, session, args),
        "summarize_entries" => tool_summarize_entries(state, session, args),
        "trace_cookies" => tool_trace_cookies(state, session, args),
        "trace_storage" => tool_trace_storage(state, session, args),
        "list_js_scripts" => tool_list_js_scripts(state, session),
        "get_js_call_map" => tool_get_js_call_map(state, session, args),
        "get_chunk_details" => tool_get_chunk_details(state, session, args),
        "list_endpoints" => tool_list_endpoints(state, session, args),
        "search_bodies" => tool_search_bodies(state, session, args),
        "compare_entries" => tool_compare_entries(state, session, args),
        "get_auth_flow" => tool_get_auth_flow(state, session),
        "get_deobfuscated_js" => tool_get_deobfuscated_js(state, session, args),
        "get_js_snippet" => tool_get_js_snippet(state, session, args),
        "decode_jwt" => tool_decode_jwt(state, session, args),
        "edit_script" => tool_edit_script(state, session, args),
        "get_script_history" => tool_get_script_history(state, session, args),
        "compare_sessions" => tool_compare_sessions(state, session, args),
        "get_token_usage" => tool_get_token_usage(state, session),
        "get_knowledge" => tool_get_knowledge(state, session),
        "update_knowledge" => tool_update_knowledge(state, session, args),
        "set_todo_list" => tool_set_todo_list(state, session, args),
        "update_todo_item" => tool_update_todo_item(state, session, args),
        "list_packages" => tool_list_packages(state, session, args),
        "walk_json" => tool_walk_json(state, session, args),
        "walk_html" => tool_walk_html(state, session, args),
        other => Err(format!("Unknown extra tool: {other}")),
    }
}

fn tool_get_entry_part(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let part = args
        .get("part")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing part".to_string())?;

    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry {index} not found"))
    })?;

    let s = &entry.summary;
    let mut out = format!("Entry [{index}] {} {}\n\n", s.method, s.url);
    let budget = state.chat_agents.get_context_budget(&session.id);
    let body_max = Some(output_limits::effective_output_limit(
        &budget,
        output_limits::parse_max_output_chars(args),
    ));
    let data_path = args.get("data_path").and_then(|v| v.as_str());

    let maybe_extracted = |body: &str| -> Option<String> {
        let path = data_path?;
        if !body.trim().starts_with('{') && !body.trim().starts_with('[') {
            return None;
        }
        serde_json::from_str::<Value>(body).ok().and_then(|v| {
            follow_json_path(&v, path)
        })
    };

    match part {
        "request_headers" => {
            out.push_str("Request headers:\n");
            for h in &entry.request_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
        }
        "response_headers" => {
            out.push_str("Response headers:\n");
            for h in &entry.response_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
        }
        "all_headers" => {
            out.push_str("Request headers:\n");
            for h in &entry.request_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
            out.push_str("\nResponse headers:\n");
            for h in &entry.response_headers {
                out.push_str(&format!("  {}: {}\n", h.name, h.value));
            }
        }
        "cookies" => {
            out.push_str("Cookies (from headers):\n");
            for h in entry
                .request_headers
                .iter()
                .chain(entry.response_headers.iter())
            {
                let lower = h.name.to_ascii_lowercase();
                if lower == "cookie" || lower == "set-cookie" {
                    out.push_str(&format!("  {}: {}\n", h.name, h.value));
                }
            }
        }
        "request_body" => {
            let mode = BodyViewMode::parse(args.get("mode").and_then(|v| v.as_str()));
            if let Some(extracted) = maybe_extracted(&entry.request_body) {
                out.push_str(&format!("Request body (path `{}`, {} bytes total):\n```json\n{}\n```", 
                    data_path.unwrap_or("?"), entry.request_body.len(), try_format_json_string(&extracted)));
            } else {
                out.push_str(&format!("Request body ({} bytes total", entry.request_body.len()));
                append_body_part(&mut out, &entry.request_body, mode, body_max);
            }
        }
        "response_body" => {
            let mode = BodyViewMode::parse(args.get("mode").and_then(|v| v.as_str()));
            if let Some(extracted) = maybe_extracted(&entry.response_body) {
                out.push_str(&format!("Response body (path `{}`, {} bytes total):\n```json\n{}\n```", 
                    data_path.unwrap_or("?"), entry.response_body.len(), try_format_json_string(&extracted)));
            } else {
                out.push_str(&format!("Response body ({} bytes total", entry.response_body.len()));
                append_body_part(&mut out, &entry.response_body, mode, body_max);
            }
        }
        other => return Err(format!("Unknown part: {other}")),
    }

    Ok(out)
}

fn append_body_part(out: &mut String, body: &str, mode: BodyViewMode, max_chars: Option<usize>) {
    match mode {
        BodyViewMode::Summary => {
            out.push_str("):\n");
            out.push_str(&format_body(body, mode));
        }
        BodyViewMode::Preview | BodyViewMode::Full => {
            let mode_label = match mode {
                BodyViewMode::Preview => ", preview",
                BodyViewMode::Full => ", full",
                BodyViewMode::Summary => unreachable!(),
            };
            out.push_str(&format!("{mode_label}):\n```\n"));
            out.push_str(&format_body_limited(body, mode, max_chars));
            out.push_str("\n```");
        }
    }
}

fn tool_summarize_entries(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let indices: Vec<usize> = args
        .get("entry_indices")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.as_u64().map(|x| x as usize))
                .take(50)
                .collect()
        })
        .unwrap_or_default();

    if indices.is_empty() {
        return Ok("No entry_indices provided.".to_string());
    }

    let mut out = format!("Summary of {} entries:\n\n", indices.len());
    for idx in indices {
        match with_db(state, |db| db.get_entry_detail(&session.id, idx))? {
            Some(entry) => {
                out.push_str(&format_entry_line(&entry.summary));
                out.push('\n');
            }
            None => out.push_str(&format!("[{idx}] (not found)\n")),
        }
    }
    Ok(out)
}

fn cookie_events(entry: &crate::har::types::HarEntryDetail) -> Vec<String> {
    let mut events = Vec::new();
    let idx = entry.summary.index;
    for h in &entry.response_headers {
        if h.name.eq_ignore_ascii_case("set-cookie") {
            events.push(format!("[{idx}] SET {} -> {}", entry.summary.url, h.value));
        }
    }
    for h in &entry.request_headers {
        if h.name.eq_ignore_ascii_case("cookie") {
            events.push(format!("[{idx}] SEND {} -> {}", entry.summary.url, h.value));
        }
    }
    events
}

fn tool_trace_cookies(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let filter = args
        .get("name_filter")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(80)
        .min(200) as usize;

    let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
    let mut events = Vec::new();
    for entry in &details {
        for ev in cookie_events(entry) {
            if let Some(ref f) = filter {
                if !ev.to_ascii_lowercase().contains(f) {
                    continue;
                }
            }
            events.push(ev);
        }
    }

    if events.is_empty() {
        return Ok("No Cookie/Set-Cookie headers found in this session.".to_string());
    }

    events.truncate(limit);
    Ok(format!(
        "Cookie flow (showing {} events):\n\n{}",
        events.len(),
        events.join("\n")
    ))
}

fn tool_trace_storage(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(40)
        .min(100) as usize;

    let patterns = [
        "localStorage",
        "sessionStorage",
        "document.cookie",
        "indexedDB",
    ];
    let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
    let mut hits = Vec::new();

    for entry in details.iter().filter(|e| e.summary.is_javascript) {
        for pat in patterns {
            if entry.response_body.contains(pat) {
                hits.push(format!(
                    "[{}] JS {} — contains '{}'",
                    entry.summary.index, entry.summary.url, pat
                ));
                break;
            }
        }
    }

    for entry in &details {
        for h in &entry.response_headers {
            if h.name.eq_ignore_ascii_case("set-cookie") {
                hits.push(format!(
                    "[{}] Set-Cookie on {}",
                    entry.summary.index, entry.summary.url
                ));
            }
        }
    }

    hits.truncate(limit);
    if hits.is_empty() {
        return Ok("No obvious storage/cookie patterns found.".to_string());
    }
    Ok(format!("Storage & cookie signals:\n\n{}", hits.join("\n")))
}

fn tool_list_js_scripts(state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let entries = with_db(state, |db| db.get_session_entries(&session.id))?;
    let js: Vec<_> = entries.iter().filter(|e| e.is_javascript).collect();
    if js.is_empty() {
        return Ok("No JavaScript entries in this session.".to_string());
    }

    let mut out = format!("{} JavaScript entries:\n\n", js.len());
    for e in js.iter().take(state.chat_agents.get_agent_limits(&session.id).list_entries_max) {
        out.push_str(&format_entry_line(e));
        out.push('\n');
    }
    Ok(out)
}

fn tool_get_js_call_map(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let single = args.get("entry_index").and_then(|v| v.as_u64()).map(|v| v as usize);
    let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
    let mut out = String::from("JS → network pattern map:\n\n");

    for entry in details.iter().filter(|e| e.summary.is_javascript) {
        if let Some(idx) = single {
            if entry.summary.index != idx {
                continue;
            }
        }
        out.push_str(&format!(
            "### [{}] {} ({} bytes)\n",
            entry.summary.index, entry.summary.url, entry.summary.size
        ));
        if entry.js_insights.is_empty() {
            out.push_str("  (no static patterns detected)\n");
        } else {
            for insight in &entry.js_insights {
                out.push_str(&format!("  - {insight}\n"));
            }
        }
        out.push('\n');
    }
    Ok(out)
}

fn tool_get_chunk_details(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let chunk_index = args
        .get("chunk_index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "Missing chunk_index".to_string())? as usize;

    let chunks = with_db(state, |db| db.get_session_chunks(&session.id))?;
    let chunk = chunks
        .iter()
        .find(|c| c.chunk_index == chunk_index)
        .ok_or_else(|| format!("Chunk {chunk_index} not found"))?;

    let payload_preview = preview_chars(&chunk.payload, 2500);
    let summary_note = if chunk.summary.is_some() {
        "LLM summary available — use get_chunk_summaries for full analysis text."
    } else {
        "Not analyzed yet — run Analyze or read payload preview below."
    };

    Ok(format!(
        "Chunk {} ({}, {} entries, ~{} tokens, status: {})\n{}\n\n### Entries covered (payload preview)\n```\n{}\n```",
        chunk.chunk_index + 1,
        chunk.chunk_type,
        chunk.entry_count,
        chunk.estimated_tokens,
        chunk.status,
        summary_note,
        payload_preview
    ))
}

fn url_path_key(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let mut path = parsed.path().to_string();
        if path.is_empty() {
            path = "/".to_string();
        }
        format!("{} {}", parsed.host_str().unwrap_or(""), path)
    } else {
        url.split('?').next().unwrap_or(url).to_string()
    }
}

fn tool_list_endpoints(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(150) as usize;

    let entries = with_db(state, |db| db.get_session_entries(&session.id))?;
    let mut map: HashMap<String, Vec<(String, usize)>> = HashMap::new();

    for e in &entries {
        if let Some(ref q) = query {
            if !e.url.to_ascii_lowercase().contains(q) {
                continue;
            }
        }
        let key = url_path_key(&e.url);
        map.entry(key).or_default().push((e.method.clone(), e.index));
    }

    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort();
    keys.truncate(limit);

    let mut out = format!("Unique endpoints ({} shown):\n\n", keys.len());
    for key in keys {
        if let Some(items) = map.get(&key) {
            let methods: HashSet<_> = items.iter().map(|(m, _)| m.as_str()).collect();
            let indices: Vec<_> = items.iter().map(|(_, i)| i.to_string()).collect();
            out.push_str(&format!(
                "- {} [{}] (entries: {})\n",
                key,
                methods.into_iter().collect::<Vec<_>>().join(", "),
                indices.join(", ")
            ));
        }
    }
    Ok(out)
}

fn preview_snippet(text: &str, query: &str, radius: usize) -> String {
    let lower = text.to_ascii_lowercase();
    let q = query.to_ascii_lowercase();
    if let Some(pos) = lower.find(&q) {
        let start = pos.saturating_sub(radius / 2);
        let end = (pos + query.len() + radius / 2).min(text.len());
        preview_chars(safe_byte_slice(text, start, end), radius)
    } else {
        preview_chars(text, radius)
    }
}

fn tool_search_bodies(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing query".to_string())?;
    let in_req = args.get("in_request").and_then(|v| v.as_bool()).unwrap_or(true);
    let in_resp = args.get("in_response").and_then(|v| v.as_bool()).unwrap_or(true);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(25)
        .min(50) as usize;

    let snippet_radius = output_limits::parse_max_output_chars(args)
        .map(|n| (n / 8).clamp(400, 8_000))
        .unwrap_or(800);

    let q_lower = query.to_ascii_lowercase();
    let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
    let mut matches = Vec::new();

    for entry in &details {
        let mut hit = false;
        let mut snippet = String::new();
        if in_req && entry.request_body.to_ascii_lowercase().contains(&q_lower) {
            hit = true;
            snippet = preview_snippet(&entry.request_body, query, snippet_radius);
        } else if in_resp && entry.response_body.to_ascii_lowercase().contains(&q_lower) {
            hit = true;
            snippet = preview_snippet(&entry.response_body, query, snippet_radius);
        }
        if hit {
            matches.push(format!(
                "[{}] {} {} — …{}…",
                entry.summary.index, entry.summary.method, entry.summary.url, snippet
            ));
        }
        if matches.len() >= limit {
            break;
        }
    }

    if matches.is_empty() {
        return Ok(format!("No body matches for '{query}'."));
    }
    Ok(format!("Body search for '{query}':\n\n{}", matches.join("\n")))
}

fn header_map(headers: &[HeaderPair]) -> HashMap<String, String> {
    headers
        .iter()
        .map(|h| (h.name.to_ascii_lowercase(), h.value.clone()))
        .collect()
}

fn tool_compare_entries(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let a = args
        .get("entry_index_a")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| "Missing entry_index_a".to_string())?;
    let b = args
        .get("entry_index_b")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| "Missing entry_index_b".to_string())?;

    let (ea, eb) = with_db(state, |db| {
        let ea = db
            .get_entry_detail(&session.id, a)?
            .ok_or_else(|| format!("Entry {a} not found"))?;
        let eb = db
            .get_entry_detail(&session.id, b)?
            .ok_or_else(|| format!("Entry {b} not found"))?;
        Ok((ea, eb))
    })?;

    let ha = header_map(&ea.request_headers);
    let hb = header_map(&eb.request_headers);

    let mut out = format!(
        "Compare [{a}] {} {}\n    vs [{b}] {} {}\n\nStatus: {} vs {}\nBody sizes: req {} vs {}, resp {} vs {}\n\nRequest header diffs:\n",
        ea.summary.method,
        ea.summary.url,
        eb.summary.method,
        eb.summary.url,
        ea.summary.status,
        eb.summary.status,
        ea.request_body.len(),
        eb.request_body.len(),
        ea.response_body.len(),
        eb.response_body.len()
    );

    for (k, va) in &ha {
        match hb.get(k) {
            Some(vb) if vb != va => out.push_str(&format!("  {k}: '{va}' vs '{vb}'\n")),
            None => out.push_str(&format!("  {k}: only in A ('{va}')\n")),
            _ => {}
        }
    }
    for (k, vb) in &hb {
        if !ha.contains_key(k) {
            out.push_str(&format!("  {k}: only in B ('{vb}')\n"));
        }
    }
    Ok(out)
}

fn tool_get_auth_flow(state: &AppState, session: &AnalysisSession) -> Result<String, String> {
    let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
    let mut lines = Vec::new();

    for entry in &details {
        let s = &entry.summary;
        if s.status == 401 || s.status == 403 {
            lines.push(format!("[{}] {} {} — HTTP {}", s.index, s.method, s.url, s.status));
        }
        for h in entry
            .request_headers
            .iter()
            .chain(entry.response_headers.iter())
        {
            let lower = h.name.to_ascii_lowercase();
            if lower.contains("authorization")
                || lower.contains("x-api-key")
                || lower.contains("x-auth")
                || lower.contains("set-cookie")
            {
                lines.push(format!(
                    "[{}] {} {} — {}: {}",
                    s.index, s.method, s.url, h.name, preview_chars(&h.value, 80)
                ));
            }
        }
    }

    lines.truncate(80);
    if lines.is_empty() {
        return Ok("No obvious auth headers or 401/403 responses found.".to_string());
    }
    Ok(format!(
        "Auth-related signals:\n\n{}\n\n\
         When Authorization or token headers contain JWTs (Bearer eyJ…), call decode_jwt with the token or entry_index — \
         never guess payload claims. Include decoded header/payload JSON in your report.",
        lines.join("\n")
    ))
}

fn tool_get_deobfuscated_js(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry {index} not found"))
    })?;

    if !entry.summary.is_javascript {
        return Ok(format!(
            "Entry [{index}] is not JavaScript (mime: {}).",
            entry.summary.mime_type
        ));
    }

    if let Some(ref code) = entry.deobfuscated_js {
        if !code.trim().is_empty() {
            return Ok(format!(
                "Deobfuscated JavaScript for entry [{index}] {} {}\n\n```javascript\n{}\n```",
                entry.summary.method,
                entry.summary.url,
                llm_body(code)
            ));
        }
    }

    Ok(format!(
        "Entry [{index}] has not been deobfuscated yet. Ask the user to open the JS tab and click \"Deobfuscate with AI\", or use get_js_snippet / get_js_analysis with raw source.\n\nRaw excerpt:\n```javascript\n{}\n```",
        llm_body(&entry.response_body)
    ))
}

fn tool_get_js_snippet(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let use_deobfuscated = args
        .get("use_deobfuscated")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry {index} not found"))
    })?;

    if !entry.summary.is_javascript {
        return Ok(format!("Entry [{index}] is not a JavaScript entry."));
    }

    let (source, source_kind) = if use_deobfuscated {
        if let Some(ref code) = entry.deobfuscated_js {
            if !code.trim().is_empty() {
                (code.as_str(), "deobfuscated")
            } else {
                (entry.response_body.as_str(), "raw")
            }
        } else {
            (entry.response_body.as_str(), "raw")
        }
    } else {
        (entry.response_body.as_str(), "raw")
    };

    if source.trim().is_empty() {
        return Ok(format!("Entry [{index}] has no JavaScript source stored."));
    }

    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Ok(format!("Entry [{index}] source is empty."));
    }

    let context = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .min(40) as usize;

    let (from, to) = if let Some(search) = args.get("search").and_then(|v| v.as_str()) {
        if search.is_empty() {
            return Err("search must be non-empty when provided".to_string());
        }
        let hit = lines
            .iter()
            .position(|line| line.contains(search))
            .ok_or_else(|| format!("Search string not found in {source_kind} source: {search}"))?;
        (
            hit.saturating_sub(context),
            (hit + context + 1).min(lines.len()),
        )
    } else {
        let start = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let end = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(start + 40)
            .max(start)
            .min(lines.len());
        (start - 1, end)
    };

    let mut out = format!(
        "JavaScript snippet from entry [{index}] ({source_kind}, lines {}–{} of {}):\n\n",
        from + 1,
        to,
        lines.len()
    );

    for (i, line) in lines[from..to].iter().enumerate() {
        out.push_str(&format!("L{:04}: {}\n", from + i + 1, line));
    }

    Ok(out)
}

fn tool_decode_jwt(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let token = args.get("token").and_then(|v| v.as_str());
    let entry_index = args
        .get("entry_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let scan_session = args
        .get("scan_session")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 25) as usize;

    if token.is_none() && entry_index.is_none() && !scan_session {
        return Err(
            "Provide token (raw JWT), entry_index (scan one entry), or scan_session=true"
                .to_string(),
        );
    }

    let mut sections = Vec::new();

    if let Some(raw) = token {
        match decode_jwt_token(raw) {
            Ok(decoded) => sections.push(format_decoded_jwt(&decoded, "token argument", None)),
            Err(err) => sections.push(format!("token argument — decode failed: {err}")),
        }
    }

    let mut scan_targets: Vec<(usize, String, String)> = Vec::new();

    if let Some(index) = entry_index {
        let entry = with_db(state, |db| {
            db.get_entry_detail(&session.id, index)?
                .ok_or_else(|| format!("Entry {index} not found"))
        })?;
        for (location, jwt) in scan_entry_for_jwts(
            &entry.request_headers,
            &entry.response_headers,
            &entry.request_body,
            &entry.response_body,
        ) {
            scan_targets.push((index, location, jwt));
        }
    }

    if scan_session {
        let details = with_db(state, |db| db.get_session_entry_details(&session.id))?;
        let mut seen_tokens = HashSet::new();
        for entry in details {
            for (location, jwt) in scan_entry_for_jwts(
                &entry.request_headers,
                &entry.response_headers,
                &entry.request_body,
                &entry.response_body,
            ) {
                if seen_tokens.insert(jwt.clone()) {
                    scan_targets.push((entry.summary.index, location, jwt));
                }
            }
        }
    }

    scan_targets.truncate(limit);

    for (index, location, jwt) in scan_targets {
        if token.is_some_and(|t| normalize_jwt_token(t) == jwt) {
            continue;
        }
        let decoded = match decode_jwt_token(&jwt) {
            Ok(decoded) => decoded,
            Err(err) => {
                sections.push(format!(
                    "Entry [{index}] — {location} (token prefix `{}…`)\nDecode failed: {err}",
                    preview_chars(&jwt, 24)
                ));
                continue;
            }
        };
        sections.push(format_decoded_jwt(
            &decoded,
            &format!("{location} (token prefix `{}…`)", preview_chars(&jwt, 24)),
            Some(index),
        ));
    }

    if sections.is_empty() {
        return Ok("No JWT-like tokens found in the requested scope.".to_string());
    }

    Ok(format!(
        "JWT decode results ({}). Signature validity is NOT verified — treat as decoded capture data only.\n\n{}",
        sections.len(),
        sections.join("\n\n---\n\n")
    ))
}

fn tool_edit_script(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    use super::script_workspace;
    let current = state.chat_agents.get_script(&session.id);
    let review = args.get("review").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_code = args
        .get("code")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_edits = args
        .get("append_code")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
        || args
            .get("replacements")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());

    if review && !has_code && !has_edits {
        let Some(ref script) = current else {
            return Ok("No workspace script yet. Use code= to create one.".to_string());
        };
        return Ok(format!(
            "Workspace script (rev {}, {} lines):\n\n```{}\n{}\n```",
            script.revision,
            script.code.lines().count(),
            script.language,
            script.code
        ));
    }

    if !has_code && !has_edits && current.is_none() {
        return Ok("No workspace script yet. Use code= to create one.".to_string());
    }

    let (script, diff) = script_workspace::resolve_script_edit(
        current.as_ref(),
        args,
        current.as_ref().map_or("python", |s| s.language.as_str()),
    )?;

    state.chat_agents.set_script(&session.id, script.clone());
    state.chat_agents.push_script_to_history(&session.id, script.clone());

    let mut out = String::from("Script edited (not executed).\n\n");
    if let Some(d) = diff {
        out.push_str(&format!("Changes (diff):\n```diff\n{}\n```\n\n", d));
    }
    out.push_str(&format!(
        "Current workspace (rev {}, {} lines). Call run_script to execute, or edit_script again to refine further.\n\n```{}\n{}\n```",
        script.revision,
        script.code.lines().count(),
        script.language,
        script.code
    ));
    Ok(out)
}

fn tool_get_script_history(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let max_prev = args
        .get("max_prev")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10) as usize;

    let current = state.chat_agents.get_script(&session.id);
    let rev = current.as_ref().map_or(0, |s| s.revision);

    if rev == 0 {
        return Ok("No script versions recorded for this session yet.".to_string());
    }

    let history = state
        .chat_agents
        .format_script_version_history(&session.id, rev, max_prev);

    if history.is_empty() {
        if let Some(ref script) = current {
            return Ok(format!(
                "No previous versions in history. Current workspace (rev {}):\n\n```{}\n{}\n```",
                script.revision,
                script.language,
                script.code
            ));
        }
        return Ok("No script versions recorded yet.".to_string());
    }

    Ok(format!(
        "Script version history (rev {} is current):\n\n{}\n\nUse edit_script with reset=true to start over while keeping this history in mind. Avoid repeating patterns that led to previous failures.",
        rev,
        history
    ))
}

fn tool_compare_sessions(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let other_id = args
        .get("other_session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing other_session_id")?;

    let (other_session, our_entries, other_entries) = {
        let db = crate::db::lock_db(&state.db)?;
        let other = db
            .get_session(other_id)?
            .ok_or_else(|| format!("Session {other_id} not found"))?;
        let ours = db.get_session_entries(&session.id)?;
        let theirs = db.get_session_entries(other_id)?;
        (other, ours, theirs)
    };

    let our_urls: std::collections::HashSet<String> =
        our_entries.iter().map(|e| e.url.clone()).collect();
    let other_urls: std::collections::HashSet<String> =
        other_entries.iter().map(|e| e.url.clone()).collect();

    let common: Vec<_> = our_urls.intersection(&other_urls).collect();
    let only_ours: Vec<_> = our_urls.difference(&other_urls).collect();
    let only_theirs: Vec<_> = other_urls.difference(&our_urls).collect();

    let our_status_counts = count_statuses(&our_entries);
    let other_status_counts = count_statuses(&other_entries);

    let mut out = format!(
        "Comparing current session `{}` vs `{}`:\n\n",
        session.file_name, other_session.file_name
    );
    out.push_str(&format!(
        "Current: {} entries, {} unique URLs\n",
        our_entries.len(),
        our_urls.len()
    ));
    out.push_str(&format!(
        "Other:   {} entries, {} unique URLs\n",
        other_entries.len(),
        other_urls.len()
    ));
    out.push_str(&format!(
        "Common URLs: {}\nOnly in current: {}\nOnly in other: {}\n\n",
        common.len(),
        only_ours.len(),
        only_theirs.len()
    ));

    out.push_str("Status distribution (current vs other):\n");
    for (label, c1, c2) in diff_statuses(&our_status_counts, &other_status_counts) {
        out.push_str(&format!("  {label}: {c1} vs {c2}\n"));
    }

    if only_theirs.len() > 0 && only_theirs.len() <= 30 {
        out.push_str("\nURLs only in other session:\n");
        for url in only_theirs.iter().take(30) {
            out.push_str(&format!("  - {url}\n"));
        }
    }

    if only_ours.len() > 0 && only_ours.len() <= 30 {
        out.push_str("\nURLs only in current session:\n");
        for url in only_ours.iter().take(30) {
            out.push_str(&format!("  - {url}\n"));
        }
    }

    Ok(out)
}

fn count_statuses(entries: &[crate::har::types::HarEntrySummary]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for e in entries {
        let bucket = if e.status < 200 {
            "1xx"
        } else if e.status < 300 {
            "2xx"
        } else if e.status < 400 {
            "3xx"
        } else if e.status < 500 {
            "4xx"
        } else {
            "5xx"
        };
        *counts.entry(bucket.to_string()).or_default() += 1;
    }
    counts
}

fn diff_statuses(
    ours: &std::collections::HashMap<String, usize>,
    theirs: &std::collections::HashMap<String, usize>,
) -> Vec<(String, usize, usize)> {
    let mut all_keys: Vec<_> = ours.keys().chain(theirs.keys()).collect();
    all_keys.sort();
    all_keys.dedup();
    all_keys
        .into_iter()
        .map(|k| {
            let c1 = ours.get(k).copied().unwrap_or(0);
            let c2 = theirs.get(k).copied().unwrap_or(0);
            (k.clone(), c1, c2)
        })
        .collect()
}

fn tool_get_token_usage(
    state: &AppState,
    session: &AnalysisSession,
) -> Result<String, String> {
    let usage = state.chat_agents.get_token_usage(&session.id);
    let total = usage.estimated_input_tokens + usage.estimated_output_tokens;

    if total == 0 {
        return Ok("No token usage recorded for this chat session yet.".to_string());
    }

    let input_k = usage.estimated_input_tokens as f64 / 1000.0;
    let output_k = usage.estimated_output_tokens as f64 / 1000.0;
    let total_k = total as f64 / 1000.0;

    Ok(format!(
        "Token usage estimate for this chat session:\n\
         Input (prompt):  ~{input_k:.1}K tokens\n\
         Output (completion): ~{output_k:.1}K tokens\n\
         Total: ~{total_k:.1}K tokens\n\
         LLM calls: {}\n\n\
         Note: Estimates are based on character count (4 chars ≈ 1 token). Actual tokenizer counts will differ.",
        usage.calls
    ))
}

fn tool_get_knowledge(
    state: &AppState,
    session: &AnalysisSession,
) -> Result<String, String> {
    let tree = state.chat_agents.get_knowledge_tree(&session.id);
    let formatted = tree.format_for_prompt();
    
    if formatted == "(No knowledge accumulated yet)" {
        Ok("Knowledge tree is empty. Use update_knowledge to store important discoveries about this HAR session (auth flows, endpoints, schemas, errors, etc.).".to_string())
    } else {
        Ok(format!("Current knowledge tree:\n\n{}", formatted))
    }
}

fn tool_update_knowledge(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let updates = args
        .get("updates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Missing 'updates' array".to_string())?;

    if updates.is_empty() {
        return Err("'updates' array is empty".to_string());
    }

    let mut applied_count = 0;
    let mut summary_lines = Vec::new();

    state.chat_agents.update_knowledge_tree(&session.id, |tree| {
        for update in updates {
            let category = update.get("category").and_then(|v| v.as_str()).unwrap_or("general");
            let key = update.get("key").and_then(|v| v.as_str()).unwrap_or("unnamed");
            let content = update.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let confidence = update.get("confidence").and_then(|v| v.as_str()).unwrap_or("observed");
            let source = update.get("source").and_then(|v| v.as_str());

            if content.is_empty() {
                continue;
            }

            tree.update_fact(category, key, content, confidence, source);
            applied_count += 1;
            summary_lines.push(format!("- [{}/{}] {}: {}", category, confidence, key, 
                if content.len() > 80 { format!("{}...", &content[..77]) } else { content.to_string() }
            ));
        }
    });

    if applied_count == 0 {
        return Err("No valid updates applied (all had empty content)".to_string());
    }

    Ok(format!(
        "Updated knowledge tree with {} fact(s):\n{}\n\nKnowledge tree now contains accumulated insights for this session.",
        applied_count,
        summary_lines.join("\n")
    ))
}

fn tool_set_todo_list(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let items_arr = args
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Missing 'items' array".to_string())?;

    let mut items = Vec::new();
    for item_val in items_arr {
        let title = item_val.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let status = item_val.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
        let notes = item_val.get("notes").and_then(|v| v.as_str()).unwrap_or("");
        items.push(crate::chat::agent_state::TodoItem {
            title: title.to_string(),
            status: status.to_string(),
            notes: notes.to_string(),
        });
    }

    state.chat_agents.set_todo_list(&session.id, items.clone());

    let mut out = format!("To-do list set with {} items:\n", items.len());
    for (i, item) in items.iter().enumerate() {
        out.push_str(&format!("  [{}] {} ({})\n", i, item.status, item.title));
        if !item.notes.is_empty() {
            out.push_str(&format!("      Notes: {}\n", item.notes));
        }
    }
    Ok(out)
}

fn tool_update_todo_item(
    state: &AppState,
    session: &AnalysisSession,
    args: &Value,
) -> Result<String, String> {
    let index = args.get("index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "Missing 'index' parameter".to_string())? as usize;
    let status = args.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
    let notes = args.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());

    state.chat_agents.update_todo_item(&session.id, index, status.clone(), notes.clone())?;

    let list = state.chat_agents.get_todo_list(&session.id);
    let item = &list.items[index];
    Ok(format!(
        "Updated to-do item [{}]: \"{}\" — status: {}, notes: {}",
        index, item.title, item.status,
        if item.notes.is_empty() { "(none)" } else { &item.notes }
    ))
}

fn tool_list_packages(
    _state: &AppState,
    _session: &AnalysisSession,
    _args: &Value,
) -> Result<String, String> {
    static CACHE: Mutex<Option<String>> = Mutex::new(None);
    
    if let Ok(cache) = CACHE.lock() {
        if let Some(ref cached) = *cache {
            return Ok(cached.clone());
        }
    }

    let python_cmds = if cfg!(target_os = "windows") {
        vec!["python", "py", "python3"]
    } else {
        vec!["python3", "python"]
    };

    let mut pip_output = None;
    for cmd in python_cmds {
        if let Ok(output) = std::process::Command::new(cmd)
            .args(["-m", "pip", "list", "--format=json"])
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .output()
        {
            if output.status.success() {
                pip_output = Some(output);
                break;
            }
        }
    }

    let output = pip_output
        .ok_or_else(|| "Could not run pip list. Ensure Python and pip are installed.".to_string())?;

    let packages_json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse pip list output: {e}"))?;
    
    let mut result = String::from("Installed Python packages:\n");
    let count = if let Some(packages) = packages_json.as_array() {
        for pkg in packages {
            let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            result.push_str(&format!("  • {} v{}\n", name, version));
        }
        packages.len()
    } else {
        0
    };
    result.push_str(&format!("\n{} packages total.", count));

    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some(result.clone());
    }

    Ok(result)
}

fn follow_json_path(root: &Value, path: &str) -> Option<String> {
    let mut current = root;
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() { continue; }
        if segment == "*" || segment == "[*]" {
            return None;
        }
        current = if let Ok(idx) = segment.parse::<usize>() {
            current.as_array()?.get(idx)?
        } else {
            current.get(segment)?
        };
    }
    Some(serde_json::to_string_pretty(current).unwrap_or_else(|_| current.to_string()))
}

fn try_format_json_string(s: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_string())
    } else {
        s.to_string()
    }
}

fn walk_json_value(value: &Value, path_segments: &[&str], depth: usize) -> Option<Value> {
    if depth >= path_segments.len() {
        return Some(value.clone());
    }
    let seg = path_segments[depth];
    if seg.is_empty() {
        return walk_json_value(value, path_segments, depth + 1);
    }
    if let Ok(idx) = seg.parse::<usize>() {
        walk_json_value(value.as_array()?.get(idx)?, path_segments, depth + 1)
    } else {
        walk_json_value(value.get(seg)?, path_segments, depth + 1)
    }
}

fn describe_json_at(value: &Value, total_bytes: usize, path: &str) -> String {
    let path_label = if path.is_empty() { "root" } else { path };
    match value {
        Value::Object(map) => {
            let key_count = map.len();
            let mut out = format!(
                "JSON object at `{path_label}` · {total_bytes} bytes total · {key_count} keys\n\nKeys and types:"
            );
            let preview_count = key_count.min(40);
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys.iter().take(preview_count) {
                if let Some(val) = map.get(*key) {
                    let type_hint = json_value_hint_compact(val);
                    out.push_str(&format!("\n  {key}: {type_hint}"));
                }
            }
            if key_count > preview_count {
                out.push_str(&format!(
                    "\n  … (+{} more keys — walk deeper into specific keys with path=)",
                    key_count - preview_count
                ));
            }
            out.push_str("\n\nWalk deeper: append a key to the path (e.g. `{path_label}.<key>`).");
            out
        }
        Value::Array(arr) => {
            let len = arr.len();
            let mut out = format!(
                "JSON array at `{path_label}` · {total_bytes} bytes total · {len} items"
            );
            if len == 0 {
                out.push_str("\nArray is empty.");
                return out;
            }
            let max_preview = (len).min(10);
            for i in 0..max_preview {
                if let Some(item) = arr.get(i) {
                    out.push_str(&format!("\n  [{i}]: {}", json_value_hint_compact(item)));
                }
            }
            if len > max_preview {
                if let Some(last) = arr.get(len - 1) {
                    let first_shape = json_shape_sig(arr.first().unwrap());
                    let last_shape = json_shape_sig(last);
                    if first_shape == last_shape && len > 1 {
                        out.push_str(&format!(
                            "\n  … items {max_preview}–{} are homogeneous (same shape as [0])",
                            len - 2
                        ));
                    } else {
                        out.push_str(&format!("\n  … [{max_preview}] …\n  [{}]: {}", len - 1, json_value_hint_compact(last)));
                    }
                }
            }
            out.push_str(&format!("\n\nWalk deeper: use numeric index (e.g. `{path_label}.0`) to inspect an item. Max index: {}", len - 1));
            out
        }
        Value::String(s) => {
            let preview = if s.len() <= 200 {
                s.clone()
            } else {
                format!("{}…", preview_chars(s, 200))
            };
            format!(
                "JSON string at `{path_label}` · {total_bytes} bytes total · {chars} chars\nValue: \"{preview}\"",
                chars = s.len()
            )
        }
        Value::Number(n) => {
            format!("JSON number at `{path_label}`\nValue: {n}")
        }
        Value::Bool(b) => {
            format!("JSON bool at `{path_label}`\nValue: {b}")
        }
        Value::Null => {
            format!("JSON null at `{path_label}`")
        }
    }
}

fn json_value_hint_compact(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("bool ({b})"),
        Value::Number(_) => "number".into(),
        Value::String(s) => {
            if s.len() <= 60 {
                format!("string \"{s}\"")
            } else {
                format!("string ({} chars) \"{}…\"", s.len(), preview_chars(s, 60))
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                "array[0]".into()
            } else {
                format!("array[{}] of {}", arr.len(), json_value_hint_compact(&arr[0]))
            }
        }
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).take(8).collect();
            keys.sort_unstable();
            if keys.is_empty() {
                return "object {}".into();
            }
            let s = keys.join(", ");
            if map.len() > 8 {
                format!("object {{{s} … +{} more}}", map.len() - 8)
            } else {
                format!("object {{{s}}}")
            }
        }
    }
}

fn json_shape_sig(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            format!("object:{}", keys.join(","))
        }
        Value::Array(arr) => format!("array:{}", arr.len()),
        other => json_value_hint_compact(other),
    }
}

fn tool_walk_json(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let part = args
        .get("part")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing part".to_string())?;

    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry {index} not found"))
    })?;

    let body = match part {
        "request_body" => &entry.request_body,
        "response_body" => &entry.response_body,
        other => return Err(format!("Unknown part: {other}")),
    };

    if body.trim().is_empty() {
        return Ok(format!("Entry [{index}] {part} is empty."));
    }

    let total_bytes = body.len();
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Body is not valid JSON: {e}"))?;

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");

    if path == "__keys__" {
        if let Value::Object(map) = &value {
            let mut out = format!(
                "Root-level JSON keys for entry [{index}] {part} ({total_bytes} bytes):\n",
            );
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in &keys {
                if let Some(val) = map.get(*key) {
                    out.push_str(&format!("  {key}: {}\n", json_value_hint_compact(val)));
                }
            }
            return Ok(out);
        }
        return Ok(format!(
            "Root is not a JSON object — it's a {}. Use path= (empty) to inspect it.",
            match &value {
                Value::Array(_) => "JSON array",
                Value::String(_) => "JSON string",
                Value::Number(_) => "JSON number",
                Value::Bool(_) => "boolean",
                Value::Null => "null",
                _ => "value",
            }
        ));
    }

    let path_segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    let owned_target: Option<Value>;
    let target: &Value = if path_segments.is_empty() {
        &value
    } else {
        match walk_json_value(&value, &path_segments, 0) {
            Some(v) => {
                owned_target = Some(v);
                owned_target.as_ref().unwrap()
            }
            None => {
                return Ok(format_path_error(&value, &path_segments, path));
            }
        }
    };

    Ok(describe_json_at(target, total_bytes, path))
}

fn format_path_error(root: &Value, segments: &[&str], full_path: &str) -> String {
    let mut cursor = root;
    for (i, seg) in segments.iter().enumerate() {
        let prefix = segments[..i].join(".");
        match cursor {
            Value::Object(map) => {
                if let Ok(_idx) = seg.parse::<usize>() {
                    return format!(
                        "At `{prefix}`, '{seg}' was treated as array index but the value is an object. Available keys: {}",
                        map.keys().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
                    );
                }
                let display_prefix = if i == 0 { "root".to_string() } else { prefix };
                return format!(
                    "At `{display_prefix}`, key '{seg}' not found. Available keys: {}",
                    map.keys().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
            Value::Array(arr) => {
                if let Ok(idx) = seg.parse::<usize>() {
                    if idx >= arr.len() {
                        return format!(
                            "At `{prefix}`, index {idx} is out of bounds. Array has {} items (0–{}).",
                            arr.len(),
                            arr.len().saturating_sub(1)
                        );
                    }
                    cursor = &arr[idx];
                    continue;
                }
                return format!(
                    "At `{prefix}`, '{seg}' was treated as object key but the value is an array with {} items. Use a numeric index (0–{}).",
                    arr.len(),
                    arr.len().saturating_sub(1)
                );
            }
            _ => {
                return format!(
                    "At `{prefix}`, the value is a scalar ({}) — cannot descend further into '{seg}'.",
                    json_value_hint_compact(cursor)
                );
            }
        }
    }
    format!("Path `{full_path}` not found in the JSON structure.")
}

fn tool_walk_html(state: &AppState, session: &AnalysisSession, args: &Value) -> Result<String, String> {
    let index = arg_entry_index(args)?;
    let part = args
        .get("part")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing part".to_string())?;

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing action".to_string())?;

    let entry = with_db(state, |db| {
        db.get_entry_detail(&session.id, index)?
            .ok_or_else(|| format!("Entry {index} not found"))
    })?;

    let body = match part {
        "request_body" => &entry.request_body,
        "response_body" => &entry.response_body,
        other => return Err(format!("Unknown part: {other}")),
    };

    let html = Html::parse_document(body);

    match action {
        "list_tags" => {
            let mut tag_counts: HashMap<String, usize> = HashMap::new();
            for node in html.tree.nodes() {
                if let ScraperNode::Element(element) = node.value() {
                    let name = element.name.local.as_ref().to_string();
                    if name != "html" && name != "head" && name != "body" {
                        *tag_counts.entry(name).or_default() += 1;
                    }
                }
            }
            let mut tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
            tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let total_tags: usize = tags.iter().map(|(_, c)| c).sum();

            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(40)
                .min(60) as usize;

            let mut out = format!(
                "HTML tag overview for entry [{index}] {part} ({total} bytes):\n{tag_count} unique tag(s), {total_tags} total elements\n",
                total = body.len(),
                tag_count = tags.len(),
            );
            for (tag, count) in tags.iter().take(limit) {
                out.push_str(&format!("  <{tag}>: {count}\n"));
            }
            if tags.len() > limit {
                out.push_str(&format!(
                    "  … (+{} more tag types — use selectors to target specific elements)\n",
                    tags.len() - limit
                ));
            }
            Ok(out)
        }
        "query_selectors" => {
            let selectors_raw: Vec<String> = args
                .get("selectors")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .take(10)
                        .collect()
                })
                .unwrap_or_default();

            if selectors_raw.is_empty() {
                return Err("Provide at least one CSS selector in 'selectors' array (e.g. ['div.container', 'a[href]'])".to_string());
            }

            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .min(15) as usize;

            let mut out = format!(
                "CSS selector results for entry [{index}] {part} ({total} bytes):\n\n",
                total = body.len(),
            );

            for sel_str in &selectors_raw {
                let selector = match Selector::parse(sel_str) {
                    Ok(s) => s,
                    Err(e) => {
                        out.push_str(&format!("`{sel_str}`: INVALID — {e:?}\n\n"));
                        continue;
                    }
                };

                let matches: Vec<_> = html.select(&selector).collect();
                let match_count = matches.len();

                out.push_str(&format!(
                    "`{sel_str}` → {match_count} match(es)\n"
                ));

                for (i, element) in matches.iter().take(limit).enumerate() {
                    let tag = element.value().name.local.as_ref();
                    let id = element.value().id().map_or(String::new(), |id| format!(" id=\"{id}\""));
                    let classes: Vec<&str> = element.value().classes().collect();
                    let class_str = if classes.is_empty() {
                        String::new()
                    } else {
                        format!(" class=\"{}\"", classes.join(" "))
                    };
                    let text: String = element.text().collect::<Vec<_>>().join(" ");
                    let text_preview = if text.trim().len() > 120 {
                        format!("{}…", preview_chars(text.trim(), 120))
                    } else {
                        text.trim().to_string()
                    };
                    out.push_str(&format!("  [{i}] <{tag}{id}{class_str}> \"{text_preview}\"\n"));
                }
                if match_count > limit {
                    out.push_str(&format!(
                        "  … (+{} more matches — use extract with a specific attr or increase limit)\n",
                        match_count - limit
                    ));
                }
                out.push('\n');
            }

            Ok(out)
        }
        "extract" => {
            let sel_str = args
                .get("selectors")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Provide one CSS selector as the first element of 'selectors'".to_string())?;

            let selector = Selector::parse(sel_str)
                .map_err(|e| format!("Invalid CSS selector `{sel_str}`: {e:?}"))?;

            let attr = args.get("attr").and_then(|v| v.as_str());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(50) as usize;

            let matches: Vec<_> = html.select(&selector).collect();
            let match_count = matches.len();

            let mut out = format!(
                "Extract from entry [{index}] {part}: `{sel_str}` → {match_count} match(es)\n\n",
            );

            for (i, element) in matches.iter().take(limit).enumerate() {
                let tag = element.value().name.local.as_ref();
                if let Some(attr_name) = attr {
                    if let Some(val) = element.value().attr(attr_name) {
                        let preview = if val.len() > 200 {
                            format!("{}…", preview_chars(val, 200))
                        } else {
                            val.to_string()
                        };
                        out.push_str(&format!("  [{i}] <{tag}> {attr_name}=\"{preview}\"\n"));
                    } else {
                        out.push_str(&format!("  [{i}] <{tag}> (no attr \"{attr_name}\")\n"));
                    }
                } else {
                    let text: String = element.text().collect::<Vec<_>>().join(" ");
                    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    let preview = if cleaned.len() > 300 {
                        format!("{}…", preview_chars(&cleaned, 300))
                    } else {
                        cleaned
                    };
                    if preview.is_empty() {
                        let inner = element.inner_html();
                        let inner_preview = if inner.len() > 200 {
                            format!("{}…", preview_chars(&inner, 200))
                        } else {
                            inner
                        };
                        out.push_str(&format!("  [{i}] <{tag}> (no text, inner: {inner_preview})\n"));
                    } else {
                        out.push_str(&format!("  [{i}] <{tag}> \"{preview}\"\n"));
                    }
                }
            }
            if match_count > limit {
                out.push_str(&format!(
                    "\n… (+{} more — increase limit or narrow the selector)",
                    match_count - limit
                ));
            }
            Ok(out)
        }
        other => Err(format!("Unknown action: {other}. Use list_tags, query_selectors, or extract.")),
    }
}
