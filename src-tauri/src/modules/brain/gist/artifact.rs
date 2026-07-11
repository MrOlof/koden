//! ADR-019 — real-time memory injection. The worker maintains, per project, a
//! DERIVED file `<root>/.koden-memory/.koden-gist.json` holding the COMPLETE
//! `UserPromptSubmit` hook stdout JSON:
//!
//! ```json
//! {"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"<gist>"}}
//! ```
//!
//! A second Koden-owned hook group (agent.rs `gist_inject_hook_cmd`) walks up
//! from `$PWD`, `cat`s the artifact if found, and injects the gist into the turn
//! — so a LIVE session picks up Brain memory in real time, without MCP wiring
//! and without a session restart. The JSON is pre-escaped HERE by serde (never
//! by shell printf interpolation): gist bytes carry quotes/newlines/arbitrary
//! redacted note titles, exactly what shell-side escaping gets wrong.
//!
//! Contracts:
//! - CACHE-STABLE: emission is write-only-if-bytes-differ (the reflect
//!   digest-pin idiom, keyed on the artifact's full bytes) so an unchanged
//!   memory state never touches the file — mtime/content stay fixed and the
//!   turn context stays byte-identical (ADR-006 P3's byte-identity guarantee
//!   only pays off if the emitted file is stable too).
//! - NEVER SELF-FEEDING: the artifact must not enter the index (its freshness
//!   line embeds the project fingerprint — indexed, every emit would rotate the
//!   fingerprint → rewrite → reindex, an unbounded oscillation) nor the notes
//!   table (the gist would quote itself; the Librarian would pay to reflect on
//!   it). Exclusions: `walk::is_reserved_artifact` (full walk + watcher gate)
//!   and the basename skip in `memory::scan_project_memory` — both keyed on
//!   [is_hook_artifact_name], which also covers the atomic-write temp sibling.
//! - ATOMIC + LF-ONLY: temp+rename via `std::fs` (a CRLF/BOM'd artifact cat'd
//!   by the POSIX hook would embed literal `\r` bytes inside the stdout JSON).
//! - FAIL-OPEN: an unindexed project (file count 0) emits nothing; the hook
//!   tolerates a missing artifact silently.

use std::path::{Path, PathBuf};

use crate::modules::brain::memory::MEMORY_DIR;
use crate::modules::brain::store;

/// Reserved basename of the derived hook artifact. Leading dot + `koden-` so a
/// collision with user-authored files is implausible; NON-`.md` so the memory
/// note scan's extension gate excludes it even without the explicit skip.
/// Shared by the walker/watcher exclusions and the agent.rs hook installer
/// (where it doubles as the group-ownership marker).
pub const HOOK_ARTIFACT_BASENAME: &str = ".koden-gist.json";

/// Token budget for the emitted gist — the chat/spawn default (commands.rs).
pub const HOOK_GIST_BUDGET_TOKENS: usize = 800;

/// `<root>/.koden-memory/.koden-gist.json`.
pub fn hook_artifact_path(root: &Path) -> PathBuf {
    root.join(MEMORY_DIR).join(HOOK_ARTIFACT_BASENAME)
}

/// True for the artifact basename AND anything derived from it (prefix match
/// covers the `.koden-gist.json.koden-tmp` atomic-write sibling), so the walk,
/// the watcher gate, and the note scan agree on every spelling — any source
/// honored on one side but not the other re-opens index/prune oscillation
/// (walk.rs's own module contract).
pub fn is_hook_artifact_name(name: &str) -> bool {
    name.starts_with(HOOK_ARTIFACT_BASENAME)
}

/// Outcome of one emission — [EmitOutcome::Unchanged] is the common steady
/// state and MUST not touch the file (the cache-stability contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitOutcome {
    /// Bytes differed (or no artifact yet) — written atomically.
    Written,
    /// Byte-identical to what's on disk — no write, no mtime change.
    Unchanged,
    /// Project has no indexed files yet — nothing emitted (fail-open; a
    /// freshness-only "0 files" gist injected every turn would be noise).
    NotReady,
}

/// Render the complete hook stdout document for one gist. Pre-escaped by serde
/// (the ONE JSON producer for this file); exactly one line + trailing `\n`, so
/// `cat` yields a single well-formed JSON doc on the hook's stdout.
pub fn render_hook_stdout(gist_text: &str) -> String {
    let doc = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": gist_text,
        }
    });
    let mut s = serde_json::to_string(&doc).expect("serialize a plain JSON value");
    s.push('\n');
    s
}

/// Build the cold-start gist (blank intent → deterministic synthesis) and
/// emit the artifact, write-only-if-bytes-differ. The compare key is free:
/// the gist is a deterministic function of index state (ADR-006 P3), so
/// re-derived bytes equal on-disk bytes exactly when memory/index state is
/// unchanged — no stored pin needed, restart-safe by construction.
pub fn emit(
    db_path: &Path,
    project_id: &str,
    project_name: &str,
    root: &Path,
) -> std::io::Result<EmitOutcome> {
    if store::file_count_readonly(db_path, project_id).unwrap_or(0) == 0 {
        return Ok(EmitOutcome::NotReady);
    }
    let gist = super::build_gist_auto(db_path, project_id, project_name, "", HOOK_GIST_BUDGET_TOKENS);
    let rendered = render_hook_stdout(&gist.bytes);
    let path = hook_artifact_path(root);
    if let Ok(existing) = std::fs::read(&path) {
        if existing == rendered.as_bytes() {
            return Ok(EmitOutcome::Unchanged);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Sibling temp + rename (the settings.json idiom) so a crash mid-write can't
    // leave a torn artifact for the hook to cat. The temp shares the reserved
    // basename PREFIX, so every exclusion covers it too.
    let tmp = path.with_file_name(format!("{HOOK_ARTIFACT_BASENAME}.koden-tmp"));
    std::fs::write(&tmp, rendered.as_bytes())?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(EmitOutcome::Written)
}

/// Delete a project's artifact (+ any orphaned temp). Used when injection is
/// toggled OFF and when a project is unregistered — the hook then finds nothing
/// (fail-open), which beats injecting frozen memory. Returns whether the
/// artifact itself was removed. Never touches user-authored files.
pub fn remove(root: &Path) -> bool {
    let path = hook_artifact_path(root);
    let _ = std::fs::remove_file(path.with_file_name(format!("{HOOK_ARTIFACT_BASENAME}.koden-tmp")));
    std::fs::remove_file(&path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::memory::scan_project_memory;
    use crate::modules::brain::store::SqliteIndex;
    use crate::modules::brain::worker;

    /// JSON-escaping property (ADR-019 risk "JSON escaping of the artifact"):
    /// gist bytes with quotes / newlines / backslashes / unicode / CRLF must
    /// round-trip through the rendered document, which must be exactly one
    /// well-formed JSON line — a shell-printf producer fails several of these.
    #[test]
    fn render_round_trips_hostile_gist_text() {
        let hostile = "# Koden Brain · \"proj\" · fp:ab\\cd\n- No code hits for \"x \u{1b}[0m…\"\r\nnul:\u{0} é🦀\ttab";
        let doc = render_hook_stdout(hostile);
        assert!(doc.ends_with('\n'), "trailing newline");
        assert_eq!(doc.trim_end_matches('\n').lines().count(), 1, "single JSON line");
        let v: serde_json::Value = serde_json::from_str(&doc).expect("valid JSON");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert_eq!(
            v["hookSpecificOutput"]["additionalContext"].as_str().unwrap(),
            hostile,
            "text survives byte-for-byte"
        );
        // No raw control bytes leak into the file (they must be \u-escaped);
        // the one allowed raw newline is the trailing one.
        let body = doc.trim_end_matches('\n');
        assert!(!body.bytes().any(|b| b < 0x20), "control chars escaped: {body}");
        assert_eq!(render_hook_stdout(hostile), doc, "deterministic");
    }

    #[test]
    fn reserved_name_covers_artifact_and_temp_only() {
        assert!(is_hook_artifact_name(HOOK_ARTIFACT_BASENAME));
        assert!(is_hook_artifact_name(".koden-gist.json.koden-tmp"));
        assert!(!is_hook_artifact_name("koden-gist.json"), "no leading dot → user file");
        assert!(!is_hook_artifact_name("note.md"));
        assert!(!is_hook_artifact_name(""));
        let p = hook_artifact_path(Path::new("/x"));
        assert!(p.ends_with(Path::new(".koden-memory/.koden-gist.json")), "{p:?}");
    }

    /// The full lifecycle against a real store + project dir: emit writes once,
    /// re-emit with unchanged memory is a byte-stable NO-write (the prompt-cache
    /// contract), a memory change re-emits, a full re-index + note re-scan never
    /// picks the artifact up (the self-feed guard: emit stays Unchanged after),
    /// and remove() deletes it.
    #[test]
    fn emit_is_byte_stable_change_aware_and_never_self_feeds() {
        let store_dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        let db = store_dir.path().join("i.sqlite");
        let idx = SqliteIndex::open(&db).unwrap();

        // Unindexed project → NotReady, nothing created.
        assert_eq!(emit(&db, "p", "proj", root).unwrap(), EmitOutcome::NotReady);
        assert!(!hook_artifact_path(root).exists());

        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        let mem = root.join(MEMORY_DIR);
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("first.md"), "---\nid: first\ntitle: First Decision\n---\nBody.").unwrap();
        worker::index_dir(&idx, "p", root);
        scan_project_memory(&idx, "p", root);

        assert_eq!(emit(&db, "p", "proj", root).unwrap(), EmitOutcome::Written);
        let path = hook_artifact_path(root);
        let first = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&first).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("# Koden Brain · proj ·"), "freshness line first: {ctx}");
        assert!(ctx.contains("First Decision"), "memory layer present: {ctx}");

        // Steady state: byte-identical re-derivation → no write.
        assert_eq!(emit(&db, "p", "proj", root).unwrap(), EmitOutcome::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        // SELF-FEED GUARD: re-index + re-scan with the artifact ON DISK — it must
        // enter neither the files index nor the notes table, so the gist (and its
        // embedded fingerprint) is unchanged and the next emit is still a no-write.
        worker::index_dir(&idx, "p", root);
        scan_project_memory(&idx, "p", root);
        assert_eq!(idx.existing_note_ids("p").unwrap(), vec!["first".to_string()]);
        assert_eq!(
            emit(&db, "p", "proj", root).unwrap(),
            EmitOutcome::Unchanged,
            "indexing the project with the artifact present must not move the gist"
        );

        // A real memory change re-emits with the new content.
        std::fs::write(mem.join("second.md"), "---\nid: second\ntitle: Second Insight\n---\nMore.").unwrap();
        worker::index_dir(&idx, "p", root);
        scan_project_memory(&idx, "p", root);
        assert_eq!(emit(&db, "p", "proj", root).unwrap(), EmitOutcome::Written);
        let second = std::fs::read_to_string(&path).unwrap();
        assert_ne!(second, first);
        let v2: serde_json::Value = serde_json::from_str(&second).unwrap();
        assert!(v2["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Second Insight"));

        // Toggle-off / unregister cleanup.
        assert!(remove(root));
        assert!(!path.exists());
        assert!(!remove(root), "second remove is a no-op");
    }
}
