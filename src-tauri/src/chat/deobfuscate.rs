use crate::db::lock_db;
use crate::har::types::{AnalysisSession, AppSettings};
use crate::llm;
use crate::AppState;

/// Run LLM deobfuscation for a JS entry and persist to DB. Returns deobfuscated source.
pub async fn ensure_entry_deobfuscated(
    state: &AppState,
    settings: &AppSettings,
    session: &AnalysisSession,
    entry_index: usize,
    force: bool,
) -> Result<String, String> {
    let (source, cached) = {
        let db = lock_db(&state.db)?;
        let entry = db
            .get_entry_detail(&session.id, entry_index)?
            .ok_or_else(|| format!("Entry {entry_index} not found"))?;

        if !entry.summary.is_javascript {
            return Err(format!("Entry {entry_index} is not a JavaScript resource"));
        }
        if entry.response_body.trim().is_empty() {
            return Err("No JavaScript source body stored for this entry".to_string());
        }

        let cached = entry
            .deobfuscated_js
            .filter(|c| !c.trim().is_empty() && !force)
            .map(|c| c.clone());
        (entry.response_body.clone(), cached)
    };

    if let Some(code) = cached {
        return Ok(code);
    }

    if settings.openrouter_api_key.is_empty() {
        return Err("OpenRouter API key is not configured".to_string());
    }

    let model = if !settings.tier3_model.trim().is_empty() {
        settings.tier3_model.clone()
    } else if !settings.default_model.trim().is_empty() {
        settings.default_model.clone()
    } else {
        return Err("No model configured for deobfuscation".to_string());
    };

    let api_model = settings.resolve_api_model(&model);
    let code = llm::deobfuscate_javascript(&settings, &api_model, &source).await?;
    if code.trim().is_empty() {
        return Err("Deobfuscation returned empty output".to_string());
    }

    {
        let db = lock_db(&state.db)?;
        db.set_deobfuscated_js(&session.id, entry_index, &code)?;
    }

    Ok(code)
}
