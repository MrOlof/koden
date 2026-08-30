//! Tier-2 resume command rewrite (EXECUTION_PLAN §4.3). A pure function over a
//! [RecoveredPane]: emit `claude --resume <id>` ONLY when a Claude session id was
//! actually captured AND the agent is `claude`; otherwise fall back to Tier-1
//! (plain re-launch in the recovered cwd). Safe-by-default — `--resume` is never
//! emitted with an unverified id.
//!
//! Capture itself (a Claude status-hook line carrying `session_id` on the agent
//! bus) is the open dependency; this rewrite is testable independently of it.

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

/// Allowlist for a captured session id spliced into the relaunch command line:
/// `[A-Za-z0-9_-]{1,64}` (Claude session ids are UUID-shaped). The planned capture
/// source is agent-bus content — attacker-influencable — so anything else (shell
/// metachars, spaces, paths) is rejected outright, never sanitized.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Decide the relaunch plan. Tier-2 requires an allowlisted captured session id
/// ([valid_session_id]) and `agent == "claude"`; every other case — including an
/// id that fails the allowlist — is Tier-1 (the unverified id is never used).
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
        let plan = resume_command(&pane(Some("claude"), Some("abc-123")), "claude");
        assert_eq!(plan, ResumePlan::Tier2 { command: "claude --resume abc-123".into() });
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
        let long = "a".repeat(65);
        assert_eq!(resume_command(&pane(Some("claude"), Some(&long)), "claude"), tier1);
        // A UUID-shaped id still resumes.
        let uuid = "0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60";
        assert_eq!(
            resume_command(&pane(Some("claude"), Some(uuid)), "claude"),
            ResumePlan::Tier2 { command: format!("claude --resume {uuid}") }
        );
    }
}
