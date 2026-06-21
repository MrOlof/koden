//! The real Anthropic `/v1/messages` client behind [ReflectClient]. Mirrors the
//! usage poller's network discipline (`usage/poll.rs:104-111,422`): reqwest+rustls,
//! HTTP-level timeouts (NO tokio `time` feature), driven by
//! `tauri::async_runtime::block_on` from the worker thread, single-flight.
//!
//! Provider facts (verified vs the `claude-api` skill, EXECUTION_PLAN §4.1.1):
//! `x-api-key` + `anthropic-version: 2023-06-01`; `thinking:{type:"adaptive"}`
//! (NEVER `budget_tokens` — 400 on 4.8/haiku); strict JSON via
//! `output_config.format.json_schema` (NOT prefill, NOT the deprecated
//! `output_format`); no `temperature`/`top_p` on 4.8.
//!
//! Secret-safety: the api key and the request/response BODY are never logged — only
//! HTTP status codes surface in errors.

use std::time::Duration;

use serde_json::json;

use super::{ReflectClient, ReflectResponse};

const DEFAULT_URL: &str = "https://api.anthropic.com/v1/messages";
/// Sandbox/test override (mirrors `poll.rs`'s `KODEN_USAGE_ENDPOINT` precedent).
const URL_ENV: &str = "KODEN_ANTHROPIC_URL";

pub struct AnthropicClient {
    api_key: String,
    url: String,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Self {
        let url = std::env::var(URL_ENV).unwrap_or_else(|_| DEFAULT_URL.to_string());
        Self { api_key, url }
    }
}

/// reqwest+rustls client with HTTP-level timeouts (no tokio time driver). LLM
/// completions are slower than the usage ping, so the total budget is generous.
fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// Minimal JSON Schema for `output_config.format` — `{proposals:[{...}]}`. Extra
/// keys are tolerated (loose), matching the serde parse side.
fn output_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "required": ["proposals"],
        "properties": {
            "proposals": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": true,
                    "required": ["kind", "title", "detail", "scope", "confidence"],
                    "properties": {
                        "kind": {"type": "string", "enum": ["insight", "should_remember", "stale", "conflict"]},
                        "title": {"type": "string"},
                        "detail": {"type": "string"},
                        "scope": {"type": "string", "enum": ["global", "project"]},
                        "confidence": {"type": "string", "enum": ["low", "medium", "high"]},
                        "project": {"type": "string"},
                        "evidence": {"type": "array", "items": {"type": "string"}}
                    }
                }
            }
        }
    })
}

#[derive(serde::Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Usage,
}

#[derive(serde::Deserialize)]
struct ContentBlock {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

impl ReflectClient for AnthropicClient {
    fn complete(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ReflectResponse, String> {
        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "thinking": {"type": "adaptive"},
            "system": system,
            "output_config": {"format": {"type": "json_schema", "schema": output_schema()}},
            "messages": [{"role": "user", "content": user}],
        });
        let payload = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let client = build_client()?;
        let key = self.api_key.clone();
        let url = self.url.clone();

        // block_on from the worker thread (no tokio time feature; HTTP timeouts only).
        let bytes = tauri::async_runtime::block_on(async move {
            let resp = client
                .post(&url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(payload)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = resp.status();
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                // Status only — NEVER the body (it can echo the prompt) or the key.
                return Err(format!("anthropic http {}", status.as_u16()));
            }
            Ok::<_, String>(bytes)
        })?;

        let parsed: MessagesResponse =
            serde_json::from_slice(&bytes).map_err(|e| format!("anthropic resp parse: {e}"))?;
        let json_text: String = parsed
            .content
            .iter()
            .filter(|b| b.kind == "text")
            .map(|b| b.text.as_str())
            .collect();
        Ok(ReflectResponse {
            json_text,
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        })
    }
}
