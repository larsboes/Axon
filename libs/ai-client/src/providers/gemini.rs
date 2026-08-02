use crate::{LlmRequest, LlmResponse};
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct GeminiClient {
    pub endpoint: String,
    pub default_model: String,
    pub api_key: String,
    pub timeout: Duration,
}

impl GeminiClient {
    pub fn new(
        endpoint: String,
        default_model: String,
        api_key: String,
        timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
            default_model,
            api_key,
            timeout,
        }
    }

    pub async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let model = &self.default_model;
        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.endpoint, model, self.api_key
        );

        let contents = json!([
            {
                "parts": [
                    {
                        "text": req.prompt
                    }
                ]
            }
        ]);

        let mut payload = json!({
            "contents": contents
        });

        if let Some(ref sys) = req.system_instructions {
            payload["systemInstruction"] = json!({
                "parts": [
                    {
                        "text": sys
                    }
                ]
            });
        }

        let mut gen_config = json!({});

        if let Some(temp) = req.temperature {
            gen_config["temperature"] = json!(temp);
        }

        if let Some(ref schema) = req.response_schema {
            gen_config["responseMimeType"] = json!("application/json");
            gen_config["responseSchema"] = schema.clone();
        }

        if !gen_config.as_object().unwrap().is_empty() {
            payload["generationConfig"] = gen_config;
        }

        let start = Instant::now();
        let res = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Gemini request failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Gemini API error ({}): {}", status, body));
        }

        let response_data: serde_json::Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Gemini JSON: {}", e))?;

        let text = response_data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| format!("Gemini response text field missing: {:?}", response_data))?
            .to_string();

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(LlmResponse {
            text,
            provider: "gemini".to_string(),
            latency_ms,
        })
    }
}
