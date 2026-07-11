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

/// ADR-018 curation modes: `autonomous` — the worker APPLIES proposals itself
/// (snapshot-undo recorded, everything revertible); `review` — proposals wait for
/// a human in the inbox (the pre-ADR-018 behavior).
pub const CURATION_AUTONOMOUS: &str = "autonomous";
pub const CURATION_REVIEW: &str = "review";

/// True for a recognized curation mode token (the only two the store accepts).
pub fn is_valid_curation_mode(mode: &str) -> bool {
    mode == CURATION_AUTONOMOUS || mode == CURATION_REVIEW
}

/// Read the persisted curation mode. Fails soft to AUTONOMOUS (the ADR-018
/// default — also what the seeded column defaults to), and normalizes any
/// unrecognized stored token to it, so the worker's mode gate never errors.
pub fn curation_mode(conn: &Connection) -> String {
    let stored: Option<String> = conn
        .query_row("SELECT curation_mode FROM brain_librarian WHERE id=1", [], |r| r.get(0))
        .ok();
    match stored {
        Some(m) if is_valid_curation_mode(&m) => m,
        _ => CURATION_AUTONOMOUS.to_string(),
    }
}

/// Persist the curation mode (writer-side; the worker calls this). Rejects
/// unknown tokens so a bad frontend payload can never wedge the mode gate.
pub fn set_curation_mode(conn: &Connection, mode: &str, now: i64) -> Result<(), String> {
    if !is_valid_curation_mode(mode) {
        return Err(format!("unknown curation mode '{mode}' (expected autonomous | review)"));
    }
    conn.execute(
        "UPDATE brain_librarian SET curation_mode=?1, updated_at=?2 WHERE id=1",
        rusqlite::params![mode, now],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Read the ADR-019 memory-injection toggle: whether the worker maintains the
/// per-project gist hook artifact that live agent sessions pick up per turn.
/// Fails soft to ON (the default — also what the seeded column defaults to),
/// so a pre-column store or a read race never silently disables injection.
pub fn inject_gist(conn: &Connection) -> bool {
    conn.query_row("SELECT inject_gist FROM brain_librarian WHERE id=1", [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|v| v != 0)
    .unwrap_or(true)
}

/// Persist the ADR-019 memory-injection toggle (writer-side; the worker calls
/// this and then emits/deletes the artifacts to match).
pub fn set_inject_gist(conn: &Connection, on: bool, now: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE brain_librarian SET inject_gist=?1, updated_at=?2 WHERE id=1",
        rusqlite::params![i64::from(on), now],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Read the persisted delta-gate pin (the digest hash of the last round a project
/// reflected on) — the durable half of `worker::LibrarianAuto.digest_hash`. `None`
/// when the project has no pinned round yet. Fails soft (a pre-table DB or read race
/// yields `None`, so the delta gate simply runs a round rather than erroring), and an
/// empty stored hash is treated as absent. [LIB-SPEND-01]
pub fn pin(conn: &Connection, project_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT digest_hash FROM brain_librarian_pin WHERE project_id=?1",
        [project_id],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .filter(|h| !h.is_empty())
}

/// Persist (upsert) the delta-gate pin for a project. Written after every round so
/// the "Unchanged => $0" short-circuit survives a worker restart — the in-memory pin
/// lives in a HashMap rebuilt empty on each boot (worker.rs:232). [LIB-SPEND-01]
pub fn set_pin(conn: &Connection, project_id: &str, digest_hash: &str, now: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO brain_librarian_pin(project_id, digest_hash, updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(project_id) DO UPDATE SET digest_hash=excluded.digest_hash, updated_at=excluded.updated_at",
        rusqlite::params![project_id, digest_hash, now],
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
    fn curation_mode_defaults_autonomous_and_roundtrips() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(curation_mode(&conn), CURATION_AUTONOMOUS, "ADR-018 default");
        set_curation_mode(&conn, CURATION_REVIEW, 1).unwrap();
        assert_eq!(curation_mode(&conn), CURATION_REVIEW);
        set_curation_mode(&conn, CURATION_AUTONOMOUS, 2).unwrap();
        assert_eq!(curation_mode(&conn), CURATION_AUTONOMOUS);
        // Unknown tokens are rejected at write and normalized at read.
        assert!(set_curation_mode(&conn, "yolo", 3).is_err());
        conn.execute("UPDATE brain_librarian SET curation_mode='garbage' WHERE id=1", []).unwrap();
        assert_eq!(curation_mode(&conn), CURATION_AUTONOMOUS, "garbage normalizes to the default");
    }

    /// ADR-019: the injection toggle defaults ON, round-trips, and fails soft
    /// to ON when the column/row can't be read.
    #[test]
    fn inject_gist_defaults_on_and_roundtrips() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert!(inject_gist(&conn), "ADR-019 default is ON");
        set_inject_gist(&conn, false, 1).unwrap();
        assert!(!inject_gist(&conn));
        set_inject_gist(&conn, true, 2).unwrap();
        assert!(inject_gist(&conn));
        // Fail-soft: an unreadable state (no table) reads as ON.
        let bare = Connection::open_in_memory().unwrap();
        assert!(inject_gist(&bare));
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
