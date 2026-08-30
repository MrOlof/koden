//! The resume session key — a stable, restart-surviving identity for a pane
//! (EXECUTION_PLAN §4.2.2). It is `blake3(cwd ‖ agent ‖ pane_uuid)`, deliberately
//! EXCLUDING the ephemeral `KODEN_SESSION` pty id (a `u32` that does not survive a
//! restart — keying on it would orphan every journal on the next boot).
//!
//! `pane_uuid` is the restart-stable identity (P4-a). Until that frontend wiring
//! lands it is `None`, and the key falls back to `cwd+agent` — the spec-sanctioned
//! fallback (collides only when two panes run the same agent in the same dir).

/// A filesystem-safe per-pane key (a blake3 hex digest → no separators, fixed len).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionKey(String);

impl SessionKey {
    /// Derive from the resolved cwd, the agent name, and the (optional) stable pane
    /// uuid. `cwd` is canonicalized so a journaled cwd matches across a restart
    /// regardless of Windows `\\?\` verbatim prefixes.
    pub fn derive(cwd: &str, agent: &str, pane_uuid: Option<&str>) -> Self {
        let cwd_n = crate::modules::fs::to_canon(std::path::Path::new(cwd));
        let material = format!("{cwd_n}\u{0}{agent}\u{0}{}", pane_uuid.unwrap_or(""));
        SessionKey(blake3::hash(material.as_bytes()).to_hex().to_string())
    }

    /// Re-hydrate a key handed back by the UI (a `RecoveredPane.key`). Only the exact
    /// blake3 hex shape is accepted: the string becomes a journal filename, so
    /// anything else (separators, `..`, a wrong length) is rejected outright.
    pub fn parse(raw: &str) -> Option<Self> {
        let ok = raw.len() == 64 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        ok.then(|| SessionKey(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The journal filename for this key (`<key>.jsonl`).
    pub fn file_name(&self) -> String {
        format!("{}.jsonl", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_only_the_exact_hex_shape() {
        let derived = SessionKey::derive("/work/proj", "claude", None);
        assert_eq!(SessionKey::parse(derived.as_str()), Some(derived.clone()));
        for bad in [
            "",
            "abc",
            "../../etc/passwd",
            &format!("{}.jsonl", derived.as_str()),
            &derived.as_str().to_uppercase(),
            &"g".repeat(64),
            &"a".repeat(65),
        ] {
            assert_eq!(SessionKey::parse(bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn derive_is_stable_for_same_inputs() {
        let a = SessionKey::derive("/work/proj", "claude", Some("uuid-1"));
        let b = SessionKey::derive("/work/proj", "claude", Some("uuid-1"));
        assert_eq!(a, b, "same inputs → byte-identical key");
        assert!(a.file_name().ends_with(".jsonl"));
    }

    #[test]
    fn derive_excludes_ephemeral_pty_id() {
        // The pty id is NOT an input — deriving for the "same pane" across a
        // restart (new pty id) yields the same key by construction.
        let before = SessionKey::derive("/work/proj", "claude", Some("uuid-1"));
        let after = SessionKey::derive("/work/proj", "claude", Some("uuid-1"));
        assert_eq!(before, after);
    }

    #[test]
    fn cwd_agent_uuid_each_change_the_key() {
        let base = SessionKey::derive("/work/proj", "claude", Some("u"));
        assert_ne!(base, SessionKey::derive("/work/other", "claude", Some("u")));
        assert_ne!(base, SessionKey::derive("/work/proj", "codex", Some("u")));
        assert_ne!(base, SessionKey::derive("/work/proj", "claude", Some("v")));
        // pane_uuid present vs absent (the fallback) are distinct keys.
        assert_ne!(base, SessionKey::derive("/work/proj", "claude", None));
    }
}
