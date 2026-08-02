use crate::providers::gemini::GeminiClient;
use crate::providers::local_openai::LocalOpenAiClient;
use crate::{LlmRequest, LlmResponse, LlmRouter, Priority};
use serde::{Deserialize, Serialize};
use std::fs;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub endpoint: String,
    pub default_model: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    pub provider: String,
    pub endpoint: String,
    pub default_model: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub task: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    pub default_policy: String,
    pub local: LocalConfig,
    pub cloud: CloudConfig,
    pub routing_rules: Vec<RoutingRule>,
}

pub struct ConfigurableLlmClient {
    pub config: RouterConfig,
    pub api_key: String,
}

impl ConfigurableLlmClient {
    pub fn load_from_env_or_file() -> Self {
        let default_config = RouterConfig {
            default_policy: "hybrid".to_string(),
            local: LocalConfig {
                endpoint: std::env::var("LOCAL_AI_HOST")
                    .unwrap_or_else(|_| "http://localhost:8000".to_string()),
                default_model: std::env::var("LOCAL_AI_MODEL")
                    .unwrap_or_else(|_| "llama3:8b".to_string()),
                timeout_ms: 5000,
            },
            cloud: CloudConfig {
                provider: "gemini".to_string(),
                endpoint: "https://generativelanguage.googleapis.com".to_string(),
                default_model: "gemini-1.5-flash".to_string(),
                timeout_ms: 10000,
            },
            routing_rules: vec![],
        };

        // No config file exists in Axon's shape yet -- AI_ROUTER_CONFIG_PATH is opt-in
        // (README.md#dynamic-paths-and-current-facts: no hardcoded path pointing at a location that doesn't exist).
        // Once a real consumer needs file-based routing rules, this becomes an
        // axon-overlay-overlay-resolved path, same pattern as every other capability's
        // config.
        let config = match std::env::var("AI_ROUTER_CONFIG_PATH") {
            Ok(path) => match fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or(default_config),
                Err(_) => default_config,
            },
            Err(_) => default_config,
        };

        let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();

        Self { config, api_key }
    }
}

impl LlmRouter for ConfigurableLlmClient {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, String> {
        let provider = match req.priority {
            Priority::Speed => "local",
            Priority::Reasoning => "cloud",
            Priority::Cost => "local",
        };

        if provider == "local" {
            let client = LocalOpenAiClient::new(
                self.config.local.endpoint.clone(),
                self.config.local.default_model.clone(),
                Duration::from_millis(self.config.local.timeout_ms),
            );
            match client.generate(&req).await {
                Ok(res) => Ok(res),
                Err(e) => {
                    if req.priority == Priority::Cost {
                        let cloud_client = GeminiClient::new(
                            self.config.cloud.endpoint.clone(),
                            self.config.cloud.default_model.clone(),
                            self.api_key.clone(),
                            Duration::from_millis(self.config.cloud.timeout_ms),
                        );
                        cloud_client.generate(&req).await
                    } else {
                        Err(e)
                    }
                }
            }
        } else {
            let client = GeminiClient::new(
                self.config.cloud.endpoint.clone(),
                self.config.cloud.default_model.clone(),
                self.api_key.clone(),
                Duration::from_millis(self.config.cloud.timeout_ms),
            );
            client.generate(&req).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config() {
        let client = ConfigurableLlmClient::load_from_env_or_file();
        assert_eq!(client.config.default_policy, "hybrid");
        assert_eq!(client.config.local.default_model, "llama3:8b");
    }
}
