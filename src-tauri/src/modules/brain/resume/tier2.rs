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

/// Decide the relaunch plan. Tier-2 requires a non-empty captured session id and
/// `agent == "claude"`; every other case is Tier-1.
pub fn resume_command(rec: &RecoveredPane, base_launch: &str) -> ResumePlan {
    match (rec.claude_session_id.as_deref(), rec.agent.as_deref()) {
        (Some(id), Some("claude")) if !id.is_empty() => {
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
}
