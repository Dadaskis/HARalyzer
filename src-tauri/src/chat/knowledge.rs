use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A hierarchical knowledge tree that persists across agent steps and conversation turns.
/// Stores key observations, inferred facts, and discovered patterns about the HAR session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeTree {
    /// Top-level categories (e.g., "auth", "endpoints", "schemas", "errors")
    pub categories: HashMap<String, KnowledgeCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCategory {
    /// Human-readable category name
    pub name: String,
    /// Key-value pairs of facts within this category
    pub facts: HashMap<String, KnowledgeFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFact {
    /// The fact/observation itself
    pub content: String,
    /// Confidence level: "observed" (from tool results), "inferred" (educated guess), "confirmed" (verified)
    pub confidence: String,
    /// When this was discovered (ISO timestamp)
    pub discovered_at: String,
    /// Optional source reference (e.g., "entry #42", "get_auth_flow result")
    pub source: Option<String>,
}

impl KnowledgeTree {
    pub fn new() -> Self {
        Self {
            categories: HashMap::new(),
        }
    }

    /// Update or create a fact in the knowledge tree
    pub fn update_fact(
        &mut self,
        category: &str,
        key: &str,
        content: &str,
        confidence: &str,
        source: Option<&str>,
    ) {
        let cat = self.categories.entry(category.to_string()).or_insert_with(|| {
            KnowledgeCategory {
                name: category.to_string(),
                facts: HashMap::new(),
            }
        });

        cat.facts.insert(
            key.to_string(),
            KnowledgeFact {
                content: content.to_string(),
                confidence: confidence.to_string(),
                discovered_at: chrono::Utc::now().to_rfc3339(),
                source: source.map(|s| s.to_string()),
            },
        );
    }

    /// Remove a fact from the knowledge tree
    pub fn remove_fact(&mut self, category: &str, key: &str) -> bool {
        if let Some(cat) = self.categories.get_mut(category) {
            cat.facts.remove(key).is_some()
        } else {
            false
        }
    }

    /// Format the knowledge tree as a readable document for the system prompt
    pub fn format_for_prompt(&self) -> String {
        if self.categories.is_empty() {
            return "(No knowledge accumulated yet)".to_string();
        }

        let mut output = String::new();
        let mut sorted_categories: Vec<_> = self.categories.iter().collect();
        sorted_categories.sort_by_key(|(name, _)| *name);

        for (cat_name, category) in sorted_categories {
            output.push_str(&format!("## {}\n\n", cat_name));

            let mut sorted_facts: Vec<_> = category.facts.iter().collect();
            sorted_facts.sort_by_key(|(key, _)| *key);

            for (key, fact) in sorted_facts {
                let confidence_marker = match fact.confidence.as_str() {
                    "observed" => "✓",
                    "confirmed" => "✓✓",
                    "inferred" => "?",
                    _ => "•",
                };

                output.push_str(&format!("**{}** [{}]: {}\n", key, confidence_marker, fact.content));

                if let Some(ref source) = fact.source {
                    output.push_str(&format!("  _Source: {}_\n", source));
                }
                output.push('\n');
            }
        }

        output.trim().to_string()
    }

    /// Get a summary of the knowledge tree (number of categories and facts)
    pub fn summary(&self) -> String {
        let total_facts: usize = self.categories.values().map(|c| c.facts.len()).sum();
        format!(
            "{} categories, {} facts",
            self.categories.len(),
            total_facts
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_tree_basic() {
        let mut tree = KnowledgeTree::new();

        tree.update_fact(
            "auth",
            "jwt_structure",
            "JWT contains user_id, role, and exp claims",
            "observed",
            Some("entry #42"),
        );

        tree.update_fact(
            "endpoints",
            "api_base",
            "Base URL is https://api.example.com/v2",
            "confirmed",
            None,
        );

        let formatted = tree.format_for_prompt();
        assert!(formatted.contains("## auth"));
        assert!(formatted.contains("jwt_structure"));
        assert!(formatted.contains("## endpoints"));
        assert!(formatted.contains("api_base"));
    }

    #[test]
    fn test_knowledge_tree_remove() {
        let mut tree = KnowledgeTree::new();

        tree.update_fact("test", "key1", "value1", "observed", None);
        assert!(tree.remove_fact("test", "key1"));
        assert!(!tree.remove_fact("test", "key1")); // Already removed
        assert!(!tree.remove_fact("nonexistent", "key1"));
    }

    #[test]
    fn test_knowledge_tree_empty() {
        let tree = KnowledgeTree::new();
        assert_eq!(tree.format_for_prompt(), "(No knowledge accumulated yet)");
        assert_eq!(tree.summary(), "0 categories, 0 facts");
    }
}
