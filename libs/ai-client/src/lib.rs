#![allow(async_fn_in_trait)]
use serde::{Deserialize, Serialize};

pub mod providers;
pub mod router;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    Speed,
    Reasoning,
    Cost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub prompt: String,
    pub system_instructions: Option<String>,
    pub priority: Priority,
    pub temperature: Option<f32>,
    pub response_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub text: String,
    pub provider: String,
    pub latency_ms: u64,
}

pub trait LlmRouter {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, String>;

    async fn generate_structured<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        req: LlmRequest,
    ) -> Result<T, String> {
        let mut req_schema = req.clone();
        if req_schema.response_schema.is_none() {
            req_schema.response_schema = Some(serde_json::json!({
                "type": "object"
            }));
        }

        let res = self.generate(req_schema).await?;
        let parsed: T = serde_json::from_str(&res.text).map_err(|e| {
            format!(
                "Failed to parse structured JSON response: {}. Raw response: {}",
                e, res.text
            )
        })?;
        Ok(parsed)
    }
}
