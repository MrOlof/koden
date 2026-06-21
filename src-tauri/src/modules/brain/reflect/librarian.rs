//! The Librarian's LLM selection — which provider/model the budgeted reflect +
//! curate path talks to. Persisted in the `brain_librarian` singleton (CANONICAL,
//! seeded to the historical Anthropic Haiku default so existing installs are
//! unchanged). Lets the user point the cheap background memory work at any
//! OpenAI-compatible provider (OpenAI, OpenRouter, Ollama, LM Studio, …) or keep
//! Anthropic — reusing the same `koden-ai` keyring keys the main AI stores.

use rusqlite::Connection;

/// Persisted Librarian model selection. Rates are $/million-tokens (the frontend
/// `MODEL_PRICING` unit); the reflect path converts to $/token. Local/free
/// providers carry 0 rates.
#[derive(Clone, Debug, PartialEq)]
pub struct LibrarianConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub in_rate_mtok: f64,
    pub out_rate_mtok: f64,
}

impl Default for LibrarianConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: super::DEFAULT_MODEL.to_string(),
            base_url: String::new(),
            in_rate_mtok: 1.0,
            out_rate_mtok: 5.0,
        }
    }
}

/// Read the persisted Librarian selection. Fails soft to the Anthropic default
/// (a pre-table DB or a read race never blocks the reflect path).
pub fn config(conn: &Connection) -> LibrarianConfig {
    conn.query_row(
        "SELECT provider, model, base_url, in_rate_mtok, out_rate_mtok FROM brain_librarian WHERE id=1",
        [],
        |r| {
            Ok(LibrarianConfig {
                provider: r.get(0)?,
                model: r.get(1)?,
                base_url: r.get(2)?,
                in_rate_mtok: r.get(3)?,
                out_rate_mtok: r.get(4)?,
            })
        },
    )
    .unwrap_or_default()
}

/// Persist the Librarian selection (writer-side; the worker calls this). Rates are
/// clamped non-negative so a bad input can never make the budget gate undercount.
pub fn set(conn: &Connection, cfg: &LibrarianConfig, now: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE brain_librarian SET provider=?1, model=?2, base_url=?3, in_rate_mtok=?4, out_rate_mtok=?5, updated_at=?6 WHERE id=1",
        rusqlite::params![
            cfg.provider,
            cfg.model,
            cfg.base_url,
            cfg.in_rate_mtok.max(0.0),
            cfg.out_rate_mtok.max(0.0),
            now
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The `koden-ai` keyring account for a provider (matches the frontend
/// `ai/config.ts` `keyringAccount`). Empty for keyless local providers.
pub fn keyring_account_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "anthropic-api-key",
        "openai" => "openai-api-key",
        "google" => "google-api-key",
        "xai" => "xai-api-key",
        "cerebras" => "cerebras-api-key",
        "groq" => "groq-api-key",
        "deepseek" => "deepseek-api-key",
        "mistral" => "mistral-api-key",
        "openrouter" => "openrouter-api-key",
        "openai-compatible" => "openai-compatible-api-key",
        _ => "", // lmstudio / mlx / ollama — keyless
    }
}

/// Providers that need no API key (local servers, or a key-optional gateway).
pub fn is_keyless(provider: &str) -> bool {
    matches!(provider, "lmstudio" | "mlx" | "ollama" | "openai-compatible")
}

/// Canonical OpenAI-compatible base URL (incl. the `/v1` segment) for a provider,
/// used when no explicit `base_url` is stored. Anthropic is absent — it uses its
/// own native client, not this table.
pub fn canonical_base_url(provider: &str) -> &'static str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "deepseek" => "https://api.deepseek.com/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "cerebras" => "https://api.cerebras.ai/v1",
        "mistral" => "https://api.mistral.ai/v1",
        "xai" => "https://api.x.ai/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta/openai",
        "ollama" => "http://localhost:11434/v1",
        "lmstudio" => "http://localhost:1234/v1",
        "mlx" => "http://127.0.0.1:8080/v1",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::store::migrate::migrate;

    #[test]
    fn defaults_to_anthropic_haiku() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let c = config(&conn);
        assert_eq!(c, LibrarianConfig::default());
        assert_eq!(c.provider, "anthropic");
        assert_eq!(c.model, super::super::DEFAULT_MODEL);
    }

    #[test]
    fn set_then_read_roundtrips() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let want = LibrarianConfig {
            provider: "openai".into(),
            model: "gpt-x-mini".into(),
            base_url: String::new(),
            in_rate_mtok: 0.15,
            out_rate_mtok: 0.6,
        };
        set(&conn, &want, 42).unwrap();
        assert_eq!(config(&conn), want);
    }

    #[test]
    fn accounts_and_keyless_and_urls() {
        assert_eq!(keyring_account_for("openai"), "openai-api-key");
        assert_eq!(keyring_account_for("anthropic"), "anthropic-api-key");
        assert_eq!(keyring_account_for("ollama"), "");
        assert!(is_keyless("ollama"));
        assert!(is_keyless("openai-compatible"));
        assert!(!is_keyless("anthropic"));
        assert!(canonical_base_url("openai").starts_with("https://"));
        assert!(canonical_base_url("ollama").contains("11434"));
        assert_eq!(canonical_base_url("anthropic"), "");
    }
}
