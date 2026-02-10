use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use serde_json::json;

/// Token usage returned by the LLM API.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Result of a completion call — text output + token usage.
#[derive(Debug)]
pub struct CompletionResult {
    pub text: String,
    pub usage: TokenUsage,
}

/// Trait for LLM providers. All providers must implement async completion.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        prompt: &str,
        model: &str,
        temperature: f64,
    ) -> Result<CompletionResult>;
}

// ─── OpenAI ──────────────────────────────────────────────────────────────────

pub struct OpenAiProvider {
    api_key: String,
    client: Client,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set in environment"))?;
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());
        Ok(Self {
            api_key,
            client: Client::new(),
            base_url,
        })
    }

    /// Create a provider with a custom base URL (useful for testing with mock servers).
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
            base_url,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(
        &self,
        prompt: &str,
        model: &str,
        temperature: f64,
    ) -> Result<CompletionResult> {
        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": temperature,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow::anyhow!("OpenAI API error ({}): {}", status, text));
        }

        let json: serde_json::Value = serde_json::from_str(&text)?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Unexpected OpenAI response format: {}", text))?;

        let usage = TokenUsage {
            prompt_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResult {
            text: content.to_string(),
            usage,
        })
    }
}

// ─── Anthropic ───────────────────────────────────────────────────────────────

pub struct AnthropicProvider {
    api_key: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set in environment"))?;
        Ok(Self {
            api_key,
            client: Client::new(),
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(
        &self,
        prompt: &str,
        model: &str,
        temperature: f64,
    ) -> Result<CompletionResult> {
        let body = json!({
            "model": model,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": temperature,
        });

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {}",
                status,
                text
            ));
        }

        let json: serde_json::Value = serde_json::from_str(&text)?;
        let content = json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Unexpected Anthropic response format: {}", text))?;

        let usage = TokenUsage {
            prompt_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32
                + json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResult {
            text: content.to_string(),
            usage,
        })
    }
}

// ─── Webhook (Custom) ────────────────────────────────────────────────────────

/// A custom provider that sends prompts to any HTTP endpoint.
///
/// The webhook server must accept POST with JSON body:
///   `{"prompt": "...", "model": "...", "temperature": 0.7}`
///
/// And return JSON:
///   `{"text": "...", "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}}`
///
/// The `usage` field is optional.
pub struct WebhookProvider {
    url: String,
    client: Client,
}

impl WebhookProvider {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl LlmProvider for WebhookProvider {
    async fn complete(
        &self,
        prompt: &str,
        model: &str,
        temperature: f64,
    ) -> Result<CompletionResult> {
        let body = json!({
            "prompt": prompt,
            "model": model,
            "temperature": temperature,
        });

        let resp = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow::anyhow!("Webhook error ({}): {}", status, text));
        }

        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Webhook returned invalid JSON: {}", e))?;

        // Primary: {"text": "..."}
        // Fallback: {"choices": [{"message": {"content": "..."}}]} (OpenAI-compatible)
        let content = json["text"]
            .as_str()
            .or_else(|| json["choices"][0]["message"]["content"].as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Webhook response must contain 'text' or 'choices[0].message.content': {}",
                    text
                )
            })?;

        let usage = TokenUsage {
            prompt_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResult {
            text: content.to_string(),
            usage,
        })
    }
}

// ─── Factory ─────────────────────────────────────────────────────────────────

/// Create a provider instance by name.
/// For "webhook", pass the URL via `WEBHOOK_URL` env var or via `provider_url` in config.
pub fn create_provider(name: &str) -> Result<Box<dyn LlmProvider>> {
    match name {
        "openai" => Ok(Box::new(OpenAiProvider::new()?)),
        "anthropic" => Ok(Box::new(AnthropicProvider::new()?)),
        "webhook" => {
            let url = std::env::var("WEBHOOK_URL").map_err(|_| {
                anyhow::anyhow!(
                    "Provider 'webhook' requires WEBHOOK_URL env var (e.g. http://localhost:8080/complete)"
                )
            })?;
            Ok(Box::new(WebhookProvider::new(url)))
        }
        other => Err(anyhow::anyhow!(
            "Unknown provider: '{}'. Known: openai, anthropic, webhook",
            other
        )),
    }
}

/// Cost per 1M tokens for popular models (input, output) in USD.
pub fn cost_per_million_tokens(model: &str) -> (f64, f64) {
    match model {
        // OpenAI
        "gpt-4o" => (2.50, 10.00),
        "gpt-4o-mini" => (0.15, 0.60),
        "gpt-4-turbo" | "gpt-4-turbo-preview" => (10.00, 30.00),
        "gpt-4" => (30.00, 60.00),
        "gpt-3.5-turbo" => (0.50, 1.50),
        "o1" => (15.00, 60.00),
        "o1-mini" => (3.00, 12.00),
        "o3-mini" => (1.10, 4.40),
        // Anthropic
        "claude-3-5-sonnet-20241022" | "claude-3-5-sonnet-latest" => (3.00, 15.00),
        "claude-3-5-haiku-20241022" | "claude-3-5-haiku-latest" => (0.80, 4.00),
        "claude-3-opus-20240229" | "claude-3-opus-latest" => (15.00, 75.00),
        _ => (0.0, 0.0),
    }
}

/// Calculate cost in USD for a given model and token usage.
pub fn calculate_cost(model: &str, usage: &TokenUsage) -> f64 {
    let (input_rate, output_rate) = cost_per_million_tokens(model);
    let input_cost = (usage.prompt_tokens as f64 / 1_000_000.0) * input_rate;
    let output_cost = (usage.completion_tokens as f64 / 1_000_000.0) * output_rate;
    input_cost + output_cost
}
