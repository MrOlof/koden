//! Per-pane append-only JSONL journal (EXECUTION_PLAN §4.2.1). One line per agent
//! signal; the durable-JSONL-tail pattern the orchestration bus already proves.
//! Fail-open: a write error is logged by the caller and dropped (resume is
//! best-effort and must never take down live status routing).

use std::io::Write;
use std::path::Path;

use super::sessionkey::SessionKey;

/// Hard cap on a single journal's line count; on overflow it is compacted to the
/// last [RESUME_COMPACT_LINES] (recovery only reads the tail). §4.4.
pub const RESUME_MAX_LINES: usize = 2000;
const RESUME_COMPACT_LINES: usize = 200;
/// Only bother counting lines for compaction once the file exceeds this size, so
/// the common path (a handful of short lines) is a single append, no read.
const COMPACT_CHECK_BYTES: u64 = 256 * 1024;

/// One journaled agent-lifecycle event. Carries no file content or secrets — just
/// the pane's resolved identity + lifecycle kind (secret-safe, §4.2.5).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResumeRecord {
    pub ts: i64,
    pub kind: String, // started|working|attention|finished|exited
    #[serde(default)]
    pub agent: Option<String>,
    pub cwd: String,
    #[serde(default)]
    pub project: Option<String>,
    /// Populated only if Tier-2 capture fires (§4.3); `None` under Tier-1.
    #[serde(default)]
    pub claude_session_id: Option<String>,
}

/// Append one record to `<resume_dir>/<key>.jsonl` (creating the dir/file). The
/// append is `O_APPEND` + a single `writeln!`; on the rare oversized journal it is
/// compacted to its tail afterwards (atomic tmp+rename).
pub fn record_event(resume_dir: &Path, key: &SessionKey, rec: &ResumeRecord) -> Result<(), String> {
    std::fs::create_dir_all(resume_dir).map_err(|e| e.to_string())?;
    let path = resume_dir.join(key.file_name());
    let line = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
    }
    compact_if_needed(&path)?;
    Ok(())
}

/// Compact an oversized journal to its last [RESUME_COMPACT_LINES] via tmp+rename.
/// Size-gated so it is a no-op for the common small journal.
fn compact_if_needed(path: &Path) -> Result<(), String> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size < COMPACT_CHECK_BYTES {
        return Ok(());
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= RESUME_MAX_LINES {
        return Ok(());
    }
    let tail = &lines[lines.len() - RESUME_COMPACT_LINES..];
    let tmp = path.with_extension("jsonl.tmp");
    let mut body = tail.join("\n");
    body.push('\n');
    {
        // fsync the tmp before the atomic rename so a crash can't leave a renamed-
        // but-empty journal (resume is best-effort, but cheap to make durable).
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
    fn append_writes_one_line_per_event() {
        let dir = tempfile::tempdir().unwrap();
        let key = SessionKey::derive("/work/proj", "claude", None);
        record_event(dir.path(), &key, &rec("started")).unwrap();
        record_event(dir.path(), &key, &rec("working")).unwrap();
        record_event(dir.path(), &key, &rec("exited")).unwrap();
        let content = std::fs::read_to_string(dir.path().join(key.file_name())).unwrap();
        assert_eq!(content.lines().count(), 3, "one line per event");
        assert!(content.lines().all(|l| serde_json::from_str::<ResumeRecord>(l).is_ok()));
    }

    #[test]
    fn oversized_journal_compacts_to_tail() {
        let dir = tempfile::tempdir().unwrap();
        let key = SessionKey::derive("/work/proj", "claude", None);
        // Write enough bytes to cross the size gate AND exceed the line cap.
        let big = ResumeRecord { cwd: "x".repeat(200), ..rec("working") };
        for _ in 0..RESUME_MAX_LINES + 500 {
            record_event(dir.path(), &key, &big).unwrap();
        }
        let content = std::fs::read_to_string(dir.path().join(key.file_name())).unwrap();
        assert!(content.lines().count() <= RESUME_MAX_LINES, "compacted: {}", content.lines().count());
    }
}
