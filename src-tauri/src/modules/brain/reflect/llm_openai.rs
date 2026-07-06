//! OpenAI-compatible `/chat/completions` client behind [ReflectClient] — the
//! Librarian's path for every non-Anthropic provider (OpenAI, OpenRouter, Ollama,
//! LM Studio, DeepSeek, Groq, Cerebras, Mistral, xAI). Mirrors the Anthropic
//! client's network discipline (reqwest+rustls, HTTP-level timeouts, `block_on`
//! from the worker thread) and its secret-safety rule: the key and the
//! request/response BODY are never logged — only HTTP status codes surface in errors.
//!
//! Strict JSON via `response_format:{type:"json_object"}` — the broadly-supported
//! mode across hosted + local servers (the system prompt already demands a single
//! JSON object). Output is fence-stripped then handed to the same
//! `schema::parse_and_validate`, so a model that wraps JSON in ``` fences or emits
//! junk fails closed → fail-open to the deterministic doctor path, never a panic.

use std::time::Duration;

use serde_json::json;

use super::{ReflectClient, ReflectResponse};

pub struct OpenAiCompatClient {
    api_key: Option<String>,
    base_url: String,
}

impl OpenAiCompatClient {
    pub fn new(api_key: Option<String>, base_url: String) -> Self {
        Self { api_key, base_url }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

/// reqwest+rustls client with HTTP-level timeouts (no tokio time driver), matching
/// the Anthropic client.
fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// The output-limit parameter name for this endpoint. OpenAI proper deprecated
/// `max_tokens` — its newer (reasoning-family) models reject it with a 400, which
/// combined with charge-on-uncertainty would bill the estimate for a call the
/// provider never ran — and requires `max_completion_tokens`. The other compat
/// servers (Ollama, LM Studio, Groq, DeepSeek, OpenRouter, …) still key on
/// `max_tokens`, and some reject unknown params, so we send exactly ONE name.
/// ponytail: host sniff; lift to a per-provider capability flag if a third
/// parameter shape ever appears.
fn token_limit_key(base_url: &str) -> &'static str {
    if base_url.contains("api.openai.com") {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

/// Defensively unwrap a ```json … ``` fence some local models emit despite the
/// "JSON only" instruction, so `parse_and_validate` sees bare JSON.
fn strip_fences(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().to_string()
}

#[derive(serde::Deserialize, Default)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Usage,
}

#[derive(serde::Deserialize, Default)]
struct Choice {
    #[serde(default)]
    message: Message,
}

#[derive(serde::Deserialize, Default)]
struct Message {
    #[serde(default)]
    content: String,
}

#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

impl ReflectClient for OpenAiCompatClient {
    fn complete(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ReflectResponse, String> {
        let body = json!({
            "model": model,
            token_limit_key(&self.base_url): max_tokens,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "response_format": {"type": "json_object"},
        });
        let payload = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let client = build_client()?;
        let url = self.endpoint();
        let key = self.api_key.clone();

        // block_on from the worker thread (no tokio time feature; HTTP timeouts only).
        let bytes = tauri::async_runtime::block_on(async move {
            let mut req = client.post(&url).header("content-type", "application/json");
            if let Some(k) = key {
                if !k.is_empty() {
                    req = req.header("authorization", format!("Bearer {k}"));
                }
            }
            let resp = req.body(payload).send().await.map_err(|e| e.to_string())?;
            let status = resp.status();
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                // Status only — NEVER the body (it can echo the prompt) or the key.
                return Err(format!("openai-compat http {}", status.as_u16()));
            }
            Ok::<_, String>(bytes)
        })?;

        let parsed: ChatResponse =
            serde_json::from_slice(&bytes).map_err(|e| format!("openai-compat resp parse: {e}"))?;
        let raw = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or_default();
        Ok(ReflectResponse {
            json_text: strip_fences(raw),
            input_tokens: parsed.usage.prompt_tokens,
            output_tokens: parsed.usage.completion_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_joins_without_double_slash() {
        let c = OpenAiCompatClient::new(None, "https://api.openai.com/v1/".to_string());
        assert_eq!(c.endpoint(), "https://api.openai.com/v1/chat/completions");
        let c2 = OpenAiCompatClient::new(None, "http://localhost:11434/v1".to_string());
        assert_eq!(c2.endpoint(), "http://localhost:11434/v1/chat/completions");
    }

    /// OpenAI proper needs `max_completion_tokens` (400s `max_tokens` on newer
    /// models → phantom spend under charge-on-uncertainty); every other compat
    /// server keeps the classic `max_tokens`.
    #[test]
    fn token_limit_key_per_endpoint() {
        assert_eq!(token_limit_key("https://api.openai.com/v1"), "max_completion_tokens");
        for other in [
            "http://localhost:11434/v1",             // ollama
            "https://api.groq.com/openai/v1",        // groq
            "https://openrouter.ai/api/v1",          // openrouter
        ] {
            assert_eq!(token_limit_key(other), "max_tokens", "{other}");
        }
    }

    #[test]
    fn strip_fences_unwraps_json_block() {
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_fences("```\n{\"b\":2}\n```"), "{\"b\":2}");
    }
}
