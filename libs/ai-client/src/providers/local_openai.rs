use crate::{LlmRequest, LlmResponse};
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct LocalOpenAiClient {
    pub endpoint: String,
    pub default_model: String,
    pub timeout: Duration,
}

impl LocalOpenAiClient {
    pub fn new(endpoint: String, default_model: String, timeout: Duration) -> Self {
        Self {
            endpoint,
            default_model,
            timeout,
        }
    }

    pub async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let url = format!("{}/v1/chat/completions", self.endpoint);

        let mut messages = vec![];

        if let Some(ref sys) = req.system_instructions {
            messages.push(json!({
                "role": "system",
                "content": sys
            }));
        }

        messages.push(json!({
            "role": "user",
            "content": req.prompt
        }));

        let mut payload = json!({
            "model": self.default_model,
            "messages": messages,
        });

        if let Some(temp) = req.temperature {
            payload["temperature"] = json!(temp);
        }

        if req.response_schema.is_some() {
            payload["response_format"] = json!({
                "type": "json_object"
            });
        }

        let start = Instant::now();
        let res = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("oMLX/OpenAI request failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Local LLM API error ({}): {}", status, body));
        }

        let response_data: serde_json::Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenAI JSON: {}", e))?;

        let text = response_data["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| format!("OpenAI content field missing: {:?}", response_data))?
            .to_string();

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(LlmResponse {
            text,
            provider: "omlx".to_string(),
            latency_ms,
        })
    }
}
