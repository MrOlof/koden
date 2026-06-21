//! Boot read-back of the resume journals (EXECUTION_PLAN §4.2.3). Reuses, almost
//! verbatim, the bus recovery semantics: drop the trailing partial line
//! (`AgentBusBridge.tsx:76` `complete = lines.length - 1`) and JSON-parse each
//! complete line in a guarded match, skipping un-parseable fragments
//! (`subagentBus.ts` tolerance). A torn final write is simply dropped.

use std::path::Path;
use std::time::UNIX_EPOCH;

use super::journal::ResumeRecord;

/// A pane recoverable on boot — the folded tail state of one journal. Drives the
/// frontend recovery card next to each cold-rehydrated tab.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecoveredPane {
    pub key: String, // sessionKey (the journal filename stem)
    pub last_kind: String,
    pub agent: Option<String>,
    pub cwd: String,
    pub project: Option<String>,
    pub claude_session_id: Option<String>,
}

/// Recover every still-open pane from `<resume_dir>/*.jsonl`. A journal whose last
/// record is `exited` is a clean finish and is skipped (no card). Deterministic:
/// the output is a pure function of the journal contents, sorted by key.
pub fn recover_all(resume_dir: &Path) -> Vec<RecoveredPane> {
    let Ok(entries) = std::fs::read_dir(resume_dir) else {
        return Vec::new();
    };
    let mut out: Vec<RecoveredPane> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(pane) = fold_journal(&path, &content) {
            out.push(pane);
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// Fold one journal's text into its tail state (or `None` if empty / cleanly
/// exited / all-garbage).
fn fold_journal(path: &Path, content: &str) -> Option<RecoveredPane> {
    let key = path.file_stem()?.to_str()?.to_string();
    // Drop the trailing partial line: split on '\n' and take all but the last
    // element (the empty tail after a final newline, OR a torn final write).
    let parts: Vec<&str> = content.split('\n').collect();
    let complete = parts.len().saturating_sub(1);
    let mut last: Option<ResumeRecord> = None;
    for line in &parts[..complete] {
        if line.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<ResumeRecord>(line) {
            last = Some(rec); // guarded parse — skip junk, keep the latest good record
        }
    }
    let rec = last?;
    if rec.kind == "exited" {
        return None; // clean finish — nothing to recover
    }
    Some(RecoveredPane {
        key,
        last_kind: rec.kind,
        agent: rec.agent,
        cwd: rec.cwd,
        project: rec.project,
        claude_session_id: rec.claude_session_id,
    })
}

/// Boot GC: delete any journal older than `ttl_days` (by mtime). `now_ms` is passed
/// in (no wall-clock) for determinism. Returns how many were removed.
///
/// TTL-only by design: the spec's "sessionKey no longer maps to a known project"
/// clause is descoped — the key is a one-way blake3 hash (no reverse map), and the
/// recovered card already carries `project` so the UI can suppress orphans. Adding a
/// project sidecar for filename-level orphan GC is a documented refinement.
pub fn gc_resume_dir(resume_dir: &Path, now_ms: i64, ttl_days: i64) -> usize {
    let Ok(entries) = std::fs::read_dir(resume_dir) else {
        return 0;
    };
    let ttl_ms = ttl_days.saturating_mul(86_400_000);
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime_ms = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(now_ms);
        if now_ms.saturating_sub(mtime_ms) > ttl_ms && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// A file's mtime in epoch-ms (for tests / callers reasoning about journal age).
pub fn mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::super::journal::{record_event, ResumeRecord};
    use super::super::sessionkey::SessionKey;
    use super::*;

    fn rec(kind: &str) -> ResumeRecord {
        ResumeRecord {
            ts: 1,
            kind: kind.into(),
            agent: Some("claude".into()),
            cwd: "/work/proj".into(),
            project: Some("p".into()),
            claude_session_id: None,
        }
    }

    #[test]
    fn recovers_open_pane_skips_exited() {
        let dir = tempfile::tempdir().unwrap();
        let open = SessionKey::derive("/work/a", "claude", None);
        let done = SessionKey::derive("/work/b", "claude", None);
        record_event(dir.path(), &open, &rec("started")).unwrap();
        record_event(dir.path(), &open, &rec("working")).unwrap();
        record_event(dir.path(), &done, &rec("started")).unwrap();
        record_event(dir.path(), &done, &rec("exited")).unwrap();
        let recovered = recover_all(dir.path());
        assert_eq!(recovered.len(), 1, "only the still-open pane");
        assert_eq!(recovered[0].last_kind, "working");
        assert_eq!(recovered[0].key, open.as_str());
    }

    #[test]
    fn drops_trailing_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let key = SessionKey::derive("/work/a", "claude", None);
        // a complete record + a TORN final write (no trailing newline).
        let good = serde_json::to_string(&rec("started")).unwrap();
        std::fs::write(dir.path().join(key.file_name()), format!("{good}\n{{\"ts\":2,\"kind\":\"work")).unwrap();
        let recovered = recover_all(dir.path());
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].last_kind, "started", "torn final line dropped");
    }

    #[test]
    fn tolerates_garbage_lines() {
        let dir = tempfile::tempdir().unwrap();
        let key = SessionKey::derive("/work/a", "claude", None);
        let good = serde_json::to_string(&rec("attention")).unwrap();
        std::fs::write(
            dir.path().join(key.file_name()),
            format!("not json\n{good}\n{{partial garbage}}\n"),
        )
        .unwrap();
        let recovered = recover_all(dir.path());
        assert_eq!(recovered.len(), 1, "garbage skipped, good record kept");
        assert_eq!(recovered[0].last_kind, "attention");
    }

    #[test]
    fn gc_removes_only_expired_journals() {
        let dir = tempfile::tempdir().unwrap();
        let key = SessionKey::derive("/work/a", "claude", None);
        record_event(dir.path(), &key, &rec("started")).unwrap();
        let path = dir.path().join(key.file_name());
        let m = mtime_ms(&path).unwrap();
        // not yet expired (now == mtime).
        assert_eq!(gc_resume_dir(dir.path(), m, 7), 0);
        assert!(path.exists());
        // far in the future → expired.
        let removed = gc_resume_dir(dir.path(), m + 8 * 86_400_000, 7);
        assert_eq!(removed, 1);
        assert!(!path.exists());
    }
}
