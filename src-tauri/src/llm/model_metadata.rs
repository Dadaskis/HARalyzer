use super::OpenRouterModel;
use serde::Deserialize;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelCapabilities {
    pub tags: Vec<String>,
    pub code_focused: bool,
    pub reasoning_focused: bool,
    pub large_context: bool,
    pub budget_tier: String,
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelData>,
}

#[derive(Debug, Deserialize)]
pub struct ModelData {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub architecture: Option<ArchitectureData>,
    #[serde(default)]
    pub top_provider: Option<TopProviderData>,
    #[serde(default)]
    pub pricing: Option<PricingData>,
    #[serde(default)]
    pub supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ArchitectureData {
    #[serde(default)]
    pub modality: Option<String>,
    #[serde(default)]
    pub tokenizer: Option<String>,
    #[serde(default)]
    pub instruct_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TopProviderData {
    #[serde(default)]
    pub context_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PricingData {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
}

pub fn infer_capabilities(id: &str, name: &str, description: &Option<String>, context: Option<u32>) -> ModelCapabilities {
    let blob = format!(
        "{} {} {}",
        id.to_ascii_lowercase(),
        name.to_ascii_lowercase(),
        description.as_deref().unwrap_or("").to_ascii_lowercase()
    );
    let mut tags = Vec::new();
    let ctx = context.unwrap_or(0);
    let large_context = ctx >= 128_000;
    if large_context {
        tags.push("large-context".to_string());
    }
    if ctx >= 200_000 {
        tags.push("xl-context".to_string());
    }

    let code_focused = blob.contains("codex")
        || blob.contains("coder")
        || blob.contains("code")
        || blob.contains("deepseek-v3")
        || blob.contains("qwen")
            && blob.contains("coder");
    if code_focused {
        tags.push("code".to_string());
    }

    let reasoning_focused = blob.contains("r1")
        || blob.contains("reason")
        || blob.contains("thinking")
        || blob.contains("o1")
        || blob.contains("o3");
    if reasoning_focused {
        tags.push("reasoning".to_string());
    }

    if blob.contains("mini") || blob.contains("flash") || blob.contains("lite") || blob.contains("haiku") {
        tags.push("fast".to_string());
    }
    if blob.contains("vision") || blob.contains("multimodal") {
        tags.push("vision".to_string());
    }

    let budget_tier = if tags.iter().any(|t| t == "fast") {
        "economy"
    } else if reasoning_focused {
        "premium-reasoning"
    } else if code_focused {
        "premium-code"
    } else if large_context {
        "premium-context"
    } else {
        "standard"
    }
    .to_string();

    ModelCapabilities {
        tags,
        code_focused,
        reasoning_focused,
        large_context,
        budget_tier,
    }
}

pub fn map_model_data(m: ModelData) -> OpenRouterModel {
    let name = m.name.unwrap_or_else(|| m.id.clone());
    let context_length = m
        .context_length
        .or_else(|| m.top_provider.as_ref().and_then(|p| p.context_length));
    let capabilities = infer_capabilities(&m.id, &name, &m.description, context_length);
    OpenRouterModel {
        id: m.id,
        name,
        context_length,
        description: m.description,
        architecture_modality: m.architecture.as_ref().and_then(|a| a.modality.clone()),
        architecture_tokenizer: m.architecture.as_ref().and_then(|a| a.tokenizer.clone()),
        architecture_instruct_type: m
            .architecture
            .as_ref()
            .and_then(|a| a.instruct_type.clone()),
        pricing_prompt: m.pricing.as_ref().and_then(|p| p.prompt.clone()),
        pricing_completion: m.pricing.as_ref().and_then(|p| p.completion.clone()),
        supported_parameters: m.supported_parameters.unwrap_or_default(),
        capabilities,
    }
}
