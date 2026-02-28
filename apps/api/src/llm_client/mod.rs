/// LLM Client — the single point of entry for all Claude API calls in Templar.
///
/// ARCHITECTURAL RULE: No other module may call the Anthropic API directly.
/// All LLM interactions MUST go through this module.
///
/// Model: claude-sonnet-4-5 (hardcoded — do not make configurable to prevent drift)
use anyhow::Result;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

pub mod prompts;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The model used for all LLM calls in Templar.
/// This is intentionally hardcoded to prevent accidental drift.
pub const MODEL: &str = "claude-sonnet-4-5";
const MAX_TOKENS: u32 = 4096;
const MAX_RETRIES: u32 = 3;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Rate limited after {retries} retries")]
    RateLimited { retries: u32 },

    #[error("LLM returned empty content")]
    EmptyContent,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl LlmResponse {
    /// Extracts the text content from the first text block.
    pub fn text(&self) -> Option<&str> {
        self.content
            .iter()
            .find(|b| b.block_type == "text")
            .and_then(|b| b.text.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorBody,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorBody {
    message: String,
}

/// The single LLM client used by all services in Templar.
/// Wraps the Anthropic Messages API with retry logic and structured output helpers.
#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    api_key: String,
}

impl LlmClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("Failed to build HTTP client"),
            api_key,
        }
    }

    /// Makes a raw call to the Claude API, returning the full response object.
    /// Retries on 429 (rate limit) and 5xx errors with exponential backoff.
    pub async fn call(&self, prompt: &str, system: &str) -> Result<LlmResponse, LlmError> {
        let request_body = AnthropicRequest {
            model: MODEL,
            max_tokens: MAX_TOKENS,
            system,
            messages: vec![AnthropicMessage {
                role: "user",
                content: prompt,
            }],
        };

        let mut last_error: Option<LlmError> = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                // Exponential backoff: 1s, 2s, 4s
                let delay = std::time::Duration::from_millis(1000 * (1 << (attempt - 1)));
                warn!(
                    "LLM call attempt {} failed, retrying after {}ms...",
                    attempt,
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
            }

            let response = self
                .client
                .post(ANTHROPIC_API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&request_body)
                .send()
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(LlmError::Http(e));
                    continue;
                }
            };

            let status = response.status();

            if status.as_u16() == 429 || status.is_server_error() {
                let body = response.text().await.unwrap_or_default();
                warn!("LLM API returned {}: {}", status, body);
                last_error = Some(LlmError::Api {
                    status: status.as_u16(),
                    message: body,
                });
                continue;
            }

            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                // Try to parse error message
                let message = serde_json::from_str::<AnthropicError>(&body)
                    .map(|e| e.error.message)
                    .unwrap_or(body);
                return Err(LlmError::Api {
                    status: status.as_u16(),
                    message,
                });
            }

            let llm_response: LlmResponse = response.json().await?;

            debug!(
                "LLM call succeeded: input_tokens={}, output_tokens={}",
                llm_response.usage.input_tokens, llm_response.usage.output_tokens
            );

            return Ok(llm_response);
        }

        Err(last_error.unwrap_or(LlmError::RateLimited {
            retries: MAX_RETRIES,
        }))
    }

    /// Convenience method that calls the LLM and deserializes the text response as JSON.
    /// The prompt must instruct the model to return valid JSON.
    pub async fn call_json<T: DeserializeOwned>(
        &self,
        prompt: &str,
        system: &str,
    ) -> Result<T, LlmError> {
        let response = self.call(prompt, system).await?;

        let text = response.text().ok_or(LlmError::EmptyContent)?;

        // Strip markdown code fences if the model wraps JSON in them
        let text = strip_json_fences(text);

        serde_json::from_str(text).map_err(LlmError::Parse)
    }
}

/// Strips ```json ... ``` or ``` ... ``` code fences from LLM output.
fn strip_json_fences(text: &str) -> &str {
    let text = text.trim();
    if let Some(stripped) = text.strip_prefix("```json") {
        stripped
            .trim_start()
            .strip_suffix("```")
            .map(|s| s.trim())
            .unwrap_or(stripped.trim_start())
    } else if let Some(stripped) = text.strip_prefix("```") {
        stripped
            .trim_start()
            .strip_suffix("```")
            .map(|s| s.trim())
            .unwrap_or(stripped.trim_start())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_json_fences_with_json_tag() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_json_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_json_fences_without_tag() {
        let input = "```\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_json_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_json_fences_no_fences() {
        let input = "{\"key\": \"value\"}";
        assert_eq!(strip_json_fences(input), "{\"key\": \"value\"}");
    }
}
