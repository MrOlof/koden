//! Tier-2 resume command rewrite (EXECUTION_PLAN §4.3). A pure function over a
//! [RecoveredPane]: emit `claude --resume <id>` ONLY when a Claude session id was
//! actually captured AND the agent is `claude`; otherwise fall back to Tier-1
//! (plain re-launch in the recovered cwd). Safe-by-default — `--resume` is never
//! emitted with an unverified id.
//!
//! Capture: the UserPromptSubmit hook payload carries `session_id`; the frontend
//! bus reader forwards it through `brain_record_turn`, and the worker stores it on
//! the pane's `LiveSession` + journals it (ADR-022 gap 1). Both seams gate on
//! [valid_session_id].

use super::cursor::RecoveredPane;

/// How to relaunch a recovered pane.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "tier", rename_all = "lowercase")]
pub enum ResumePlan {
    /// `base_launch` with `--resume <id>` spliced in.
    Tier2 { command: String },
    /// Plain re-launch in the recovered cwd; no resume id available.
    Tier1 { cwd: String },
}

/// Allowlist for a captured session id, shared by the ingest seam
/// (`brain_record_turn`, the worker) and the splice below: `[A-Za-z0-9._-]{8,128}`
/// (Claude ids are UUIDs or 25-char alphanumerics; the frontend reader mirrors
/// this exactly). The capture source is agent-bus content, attacker-influencable,
/// so anything else (shell metachars, spaces, paths) is rejected outright, never
/// sanitized.
pub fn valid_session_id(id: &str) -> bool {
    (8..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Decide the relaunch plan. Tier-2 requires an allowlisted captured session id
/// ([valid_session_id]) and `agent == "claude"`; every other case — including an
/// id that fails the allowlist — is Tier-1 (the unverified id is never used).
// ponytail: an id whose transcript was deleted since is spliced anyway and
// `claude --resume` errors on it; transcript existence is not verified here.
pub fn resume_command(rec: &RecoveredPane, base_launch: &str) -> ResumePlan {
    match (rec.claude_session_id.as_deref(), rec.agent.as_deref()) {
        (Some(id), Some("claude")) if valid_session_id(id) => {
            ResumePlan::Tier2 { command: format!("{base_launch} --resume {id}") }
        }
        _ => ResumePlan::Tier1 { cwd: rec.cwd.clone() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(agent: Option<&str>, sid: Option<&str>) -> RecoveredPane {
        RecoveredPane {
            key: "k".into(),
            last_kind: "working".into(),
            agent: agent.map(String::from),
            cwd: "/work/proj".into(),
            project: Some("p".into()),
            claude_session_id: sid.map(String::from),
            last_ts: 0,
        }
    }

    #[test]
    fn tier2_only_for_claude_with_captured_id() {
        let plan = resume_command(&pane(Some("claude"), Some("sess-abc-123")), "claude");
        assert_eq!(plan, ResumePlan::Tier2 { command: "claude --resume sess-abc-123".into() });
        // The 25-char alphanumeric shape Claude Code also emits.
        let short = "01C6fAURUuomAXbxsbYFRB2Rh";
        assert_eq!(
            resume_command(&pane(Some("claude"), Some(short)), "claude"),
            ResumePlan::Tier2 { command: format!("claude --resume {short}") }
        );
    }

    #[test]
    fn session_id_allowlist_matches_the_frontend_reader() {
        for ok in ["0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60", "01C6fAURUuomAXbxsbYFRB2Rh", "abcd.efgh", &"a".repeat(128)] {
            assert!(valid_session_id(ok), "{ok:?} must pass");
        }
        for bad in [
            "",
            "../x",
            "abc 123 def",
            "abcdefg", // 7 chars: under the floor
            &"a".repeat(129),
            &"a".repeat(200),
            "abc;rm -rf ~",
            "abc/def/ghi",
            "abc$(whoami)",
            "0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60\n",
        ] {
            assert!(!valid_session_id(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn falls_back_to_tier1_without_capture() {
        // no id
        assert_eq!(
            resume_command(&pane(Some("claude"), None), "claude"),
            ResumePlan::Tier1 { cwd: "/work/proj".into() }
        );
        // empty id
        assert_eq!(
            resume_command(&pane(Some("claude"), Some("")), "claude"),
            ResumePlan::Tier1 { cwd: "/work/proj".into() }
        );
        // wrong agent (even with an id)
        assert_eq!(
            resume_command(&pane(Some("codex"), Some("abc")), "codex"),
            ResumePlan::Tier1 { cwd: "/work/proj".into() }
        );
    }

    #[test]
    fn rejects_session_id_outside_allowlist() {
        let tier1 = ResumePlan::Tier1 { cwd: "/work/proj".into() };
        // Shell metachars / spaces / paths must never reach the command line.
        for bad in [
            "abc; rm -rf ~",
            "abc && curl evil",
            "abc$(whoami)",
            "abc`id`",
            "../../etc/passwd",
            "abc 123",
            "abc\"def",
        ] {
            assert_eq!(resume_command(&pane(Some("claude"), Some(bad)), "claude"), tier1);
        }
        // Over-long id rejected too.
        let long = "a".repeat(129);
        assert_eq!(resume_command(&pane(Some("claude"), Some(&long)), "claude"), tier1);
        // A UUID-shaped id still resumes.
        let uuid = "0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60";
        assert_eq!(
            resume_command(&pane(Some("claude"), Some(uuid)), "claude"),
            ResumePlan::Tier2 { command: format!("claude --resume {uuid}") }
        );
    }
}
