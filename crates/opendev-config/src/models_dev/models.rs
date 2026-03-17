//! Core data types: RegistryError, ModelInfo, ProviderInfo.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("failed to read cache: {0}")]
    CacheRead(#[from] std::io::Error),
    #[error("failed to parse JSON: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("network fetch failed: {0}")]
    NetworkError(String),
}

/// Information about a specific model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_length: u64,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub pricing_input: f64,
    #[serde(default)]
    pub pricing_output: f64,
    #[serde(default = "default_pricing_unit")]
    pub pricing_unit: String,
    #[serde(default)]
    pub serverless: bool,
    #[serde(default)]
    pub tunable: bool,
    #[serde(default)]
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default = "default_true")]
    pub supports_temperature: bool,
    #[serde(default = "default_api_type")]
    pub api_type: String,
}

fn default_pricing_unit() -> String {
    "per million tokens".to_string()
}
fn default_true() -> bool {
    true
}
fn default_api_type() -> String {
    "chat".to_string()
}

impl ModelInfo {
    /// Format pricing for display.
    pub fn format_pricing(&self) -> String {
        if self.pricing_input == 0.0 && self.pricing_output == 0.0 {
            return "N/A".to_string();
        }
        format!(
            "${:.2} in / ${:.2} out {}",
            self.pricing_input, self.pricing_output, self.pricing_unit
        )
    }
}

impl std::fmt::Display for ModelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let caps = self.capabilities.join(", ");
        write!(
            f,
            "{}\n  Provider: {}\n  Context: {} tokens\n  Capabilities: {}",
            self.name, self.provider, self.context_length, caps
        )
    }
}

/// Information about a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub api_key_env: String,
    pub api_base_url: String,
    pub models: HashMap<String, ModelInfo>,
}

impl ProviderInfo {
    /// List all models, optionally filtered by capability.
    pub fn list_models(&self, capability: Option<&str>) -> Vec<&ModelInfo> {
        let mut models: Vec<&ModelInfo> = self.models.values().collect();
        if let Some(cap) = capability {
            models.retain(|m| m.capabilities.contains(&cap.to_string()));
        }
        models.sort_by(|a, b| b.context_length.cmp(&a.context_length));
        models
    }

    /// Get the recommended model for this provider.
    pub fn get_recommended_model(&self) -> Option<&ModelInfo> {
        self.models
            .values()
            .find(|m| m.recommended)
            .or_else(|| self.models.values().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_display() {
        let model = ModelInfo {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            provider: "OpenAI".to_string(),
            context_length: 128_000,
            capabilities: vec!["text".to_string(), "vision".to_string()],
            pricing_input: 30.0,
            pricing_output: 60.0,
            pricing_unit: "per million tokens".to_string(),
            serverless: false,
            tunable: false,
            recommended: true,
            max_tokens: Some(4096),
            supports_temperature: true,
            api_type: "chat".to_string(),
        };
        let display = format!("{}", model);
        assert!(display.contains("GPT-4"));
        assert!(display.contains("128000"));
    }

    #[test]
    fn test_model_info_pricing() {
        let model = ModelInfo {
            id: "test".to_string(),
            name: "Test".to_string(),
            provider: "Test".to_string(),
            context_length: 4096,
            capabilities: vec![],
            pricing_input: 1.5,
            pricing_output: 2.0,
            pricing_unit: "per million tokens".to_string(),
            serverless: false,
            tunable: false,
            recommended: false,
            max_tokens: None,
            supports_temperature: true,
            api_type: "chat".to_string(),
        };
        assert_eq!(
            model.format_pricing(),
            "$1.50 in / $2.00 out per million tokens"
        );

        let free = ModelInfo {
            pricing_input: 0.0,
            pricing_output: 0.0,
            ..model.clone()
        };
        assert_eq!(free.format_pricing(), "N/A");
    }

    #[test]
    fn test_provider_list_models() {
        let mut models = HashMap::new();
        models.insert(
            "small".to_string(),
            ModelInfo {
                id: "small".to_string(),
                name: "Small".to_string(),
                provider: "Test".to_string(),
                context_length: 4096,
                capabilities: vec!["text".to_string()],
                pricing_input: 0.0,
                pricing_output: 0.0,
                pricing_unit: "per million tokens".to_string(),
                serverless: false,
                tunable: false,
                recommended: false,
                max_tokens: None,
                supports_temperature: true,
                api_type: "chat".to_string(),
            },
        );
        models.insert(
            "large".to_string(),
            ModelInfo {
                id: "large".to_string(),
                name: "Large".to_string(),
                provider: "Test".to_string(),
                context_length: 128_000,
                capabilities: vec!["text".to_string(), "vision".to_string()],
                pricing_input: 0.0,
                pricing_output: 0.0,
                pricing_unit: "per million tokens".to_string(),
                serverless: false,
                tunable: false,
                recommended: true,
                max_tokens: None,
                supports_temperature: true,
                api_type: "chat".to_string(),
            },
        );

        let provider = ProviderInfo {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "Test provider".to_string(),
            api_key_env: "TEST_API_KEY".to_string(),
            api_base_url: "https://api.test.com".to_string(),
            models,
        };

        let all = provider.list_models(None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].context_length, 128_000); // sorted by context desc

        let vision = provider.list_models(Some("vision"));
        assert_eq!(vision.len(), 1);
        assert_eq!(vision[0].id, "large");

        assert_eq!(provider.get_recommended_model().unwrap().id, "large");
    }
}
