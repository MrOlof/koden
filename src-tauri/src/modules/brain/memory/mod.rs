//! Native memory store (CONCEPT §4.2, ADR-006 P1). Curated knowledge notes are
//! markdown + YAML frontmatter, committed under `<project>/.koden-memory/*.md`
//! (portable, git-committed). The note FILES are already lexically searchable
//! (the walk indexes them); this module adds the STRUCTURED layer — parsed,
//! typed notes in a `notes` table — that powers memory cards, anchors, the doctor,
//! and (P4) proposals.
//!
//! Frontmatter parse is intentionally null-stripping (serde maps missing/explicit
//! -null fields to `None`), matching Conductr's Zod-acceptance parity
//! (EXECUTION_PLAN §0.3). [DP-10]

pub mod doctor;
pub mod proposal;

use std::path::Path;

use crate::modules::brain::secrets;
use crate::modules::brain::store::SqliteIndex;

/// Canonical per-project memory folder (ADR-006 proposed name).
pub const MEMORY_DIR: &str = ".koden-memory";

/// A curated knowledge note. CONCEPT §4.2 schema. [DP-11] typed memory.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MemoryNote {
    pub id: String,
    pub title: String,
    pub note_type: Option<String>,
    pub scope: Option<String>,
    pub provenance: Option<String>,
    pub status: Option<String>,
    pub created: Option<String>,
    pub revalidate_after: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub anchors: Vec<String>,
    pub body: String,
}

/// Compact note view for the review inbox / memory cards (`brain_notes`).
#[derive(Clone, Debug, serde::Serialize)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub note_type: Option<String>,
    pub status: Option<String>,
    pub path: String,
    pub anchors: Vec<String>,
}

#[derive(Default)]
struct Frontmatter {
    id: Option<String>,
    title: Option<String>,
    note_type: Option<String>,
    scope: Option<String>,
    provenance: Option<String>,
    status: Option<String>,
    created: Option<String>,
    revalidate_after: Option<String>,
    supersedes: Option<String>,
    superseded_by: Option<String>,
    anchors: Vec<String>,
}

/// Tolerant frontmatter projection: parse to a YAML value, then pull each field
/// independently. A single wrong-typed field yields `None`/`[]` for THAT field
/// only — it never discards the whole block (Conductr gray-matter parity,
/// EXECUTION_PLAN). Malformed YAML → all-None (id falls back to the stem).
fn parse_frontmatter_map(s: &str) -> Frontmatter {
    let val: serde_yaml::Value = serde_yaml::from_str(s).unwrap_or(serde_yaml::Value::Null);
    let get = |k: &str| val.get(k).and_then(scalar_to_string);
    Frontmatter {
        id: get("id"),
        title: get("title"),
        note_type: get("type"),
        scope: get("scope"),
        provenance: get("provenance"),
        status: get("status"),
        created: get("created"),
        revalidate_after: get("revalidate_after"),
        supersedes: get("supersedes"),
        superseded_by: get("superseded_by"),
        anchors: val.get("anchors").map(value_to_string_list).unwrap_or_default(),
    }
}

fn scalar_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None, // Null / Sequence / Mapping / Tagged
    }
}

fn value_to_string_list(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::Sequence(seq) => seq.iter().filter_map(scalar_to_string).collect(),
        other => scalar_to_string(other).into_iter().collect(),
    }
}

/// Parse a markdown note (optional leading `---` YAML frontmatter + body).
/// `fallback_id` (typically the filename stem) is used when frontmatter has no id.
pub fn parse(raw: &str, fallback_id: &str) -> MemoryNote {
    let (fm_str, body) = split_frontmatter(raw);
    let fm = fm_str.map(parse_frontmatter_map).unwrap_or_default();
    let title = fm
        .title
        .clone()
        .or_else(|| heading_title(body))
        .unwrap_or_else(|| fallback_id.to_string());
    MemoryNote {
        id: fm.id.unwrap_or_else(|| fallback_id.to_string()),
        title,
        note_type: fm.note_type,
        scope: fm.scope,
        provenance: fm.provenance,
        status: fm.status,
        created: fm.created,
        revalidate_after: fm.revalidate_after,
        supersedes: fm.supersedes,
        superseded_by: fm.superseded_by,
        anchors: fm.anchors,
        body: body.to_string(),
    }
}

/// Split off a leading `---`-delimited YAML frontmatter block, returning
/// `(Some(yaml), body)` or `(None, raw)` when there is no frontmatter.
fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw); // strip BOM
    let after_open = match raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n")) {
        Some(r) => r,
        None => return (None, raw),
    };
    // Closing delimiter: a line that is exactly "---".
    for (idx, _) in after_open.match_indices("---") {
        let at_line_start = idx == 0 || after_open.as_bytes()[idx - 1] == b'\n';
        let after = &after_open[idx + 3..];
        // Tolerate trailing horizontal whitespace on the closing fence ("--- ").
        let after_ws = after.trim_start_matches([' ', '\t']);
        let closes_line =
            after_ws.is_empty() || after_ws.starts_with('\n') || after_ws.starts_with('\r');
        if at_line_start && closes_line {
            let fm = &after_open[..idx];
            let body = after_ws.trim_start_matches(['\r', '\n']);
            return (Some(fm), body);
        }
    }
    (None, raw)
}

fn heading_title(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
}

/// Scan `<root>/.koden-memory/*.md`, parse each into a `MemoryNote`, and upsert
/// the structured row. Returns the count. Non-recursive (flat memory folder).
/// The note files themselves are made searchable by the code walk.
pub fn scan_project_memory(index: &SqliteIndex, project_id: &str, root: &Path) -> usize {
    let dir = root.join(MEMORY_DIR);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut count = 0usize;
    // ADR-010: deletion needs POSITIVE evidence (NotFound). Any other failure
    // (Windows AV/editor lock, permission blip) leaves a note's state UNKNOWN —
    // the scan is then PARTIAL and must not feed reconcile-delete, or a transient
    // read error would destroy notes AND their pending paid proposals (removed in
    // the same txn by `remove_note`).
    let mut complete = true;
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let Ok(entry) = entry else {
                    complete = false; // unreadable dir entry — unknown, not absent
                    continue;
                };
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&path) {
                    Ok(raw) => raw,
                    // Vanished between read_dir and read — positively gone.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => {
                        // ponytail: one unreadable note skips the whole project's note
                        // reconcile this pass (an unreadable file can't be mapped to its
                        // note id); upgrade path = resolve the id via the note's rel path
                        // in the notes table and exclude just that one from deletion.
                        log::debug!(
                            "brain: note {} unreadable ({e}); reconcile skipped this pass",
                            path.display()
                        );
                        complete = false;
                        continue;
                    }
                };
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("note");
                let mut note = parse(&raw, stem);
                // Secrets gate: the notes table is a form of indexing — redact the
                // user-controlled free-text (title + anchors) before it is stored/shown
                // or fed to the reflect digest (CONCEPT §7.1). Redacting a normal path
                // anchor is a no-op; only a secret-shaped anchor changes (and is then
                // correctly flagged broken by the doctor).
                note.title = secrets::redact(&note.title).0;
                note.anchors = note.anchors.iter().map(|a| secrets::redact(a).0).collect();
                let hash = crate::modules::brain::freshness::hash::hash_bytes(raw.as_bytes());
                let rel = format!(
                    "{MEMORY_DIR}/{}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
                );
                if index.upsert_note(project_id, &note, &rel, &hash).is_ok() {
                    count += 1;
                }
                // Even on a store error the note EXISTS on disk — keep it out of
                // the deletion set (ADR-010).
                seen.insert(note.id.clone());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No memory folder — positive evidence every note is gone ONLY if the
            // project root itself is still there; an absent/unreadable root
            // (unmounted drive) makes the whole scan UNKNOWN.
            if !root.is_dir() {
                complete = false;
            }
        }
        Err(e) => {
            log::warn!(
                "brain: memory dir {} unreadable ({e}); keeping last-good notes",
                dir.display()
            );
            complete = false;
        }
    }
    // Reconcile-delete: drop notes (and their pending proposals) no longer on disk
    // — mirrors the `files` reconcile so a deleted/renamed note doesn't linger.
    // ONLY on a complete scan (positive evidence of absence).
    if complete {
        if let Ok(existing) = index.existing_note_ids(project_id) {
            for id in existing {
                if !seen.contains(&id) {
                    let _ = index.remove_note(project_id, &id);
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let raw = "---\nid: adr-1\ntype: decision\nstatus: accepted\nanchors:\n  - foo::bar\n  - baz\n---\n# Use SQLite\n\nWe chose SQLite.\n";
        let n = parse(raw, "fallback");
        assert_eq!(n.id, "adr-1");
        assert_eq!(n.note_type.as_deref(), Some("decision"));
        assert_eq!(n.status.as_deref(), Some("accepted"));
        assert_eq!(n.anchors, vec!["foo::bar", "baz"]);
        assert_eq!(n.title, "Use SQLite");
        assert!(n.body.contains("We chose SQLite."));
    }

    #[test]
    fn null_strips_to_none() {
        let raw = "---\nid: x\nstatus: null\nscope:\n---\nbody\n";
        let n = parse(raw, "fb");
        assert_eq!(n.status, None, "explicit null → None");
        assert_eq!(n.scope, None, "empty → None");
    }

    #[test]
    fn no_frontmatter_uses_fallback_and_heading() {
        let n = parse("# A Title\n\nsome text", "fallback-id");
        assert_eq!(n.id, "fallback-id");
        assert_eq!(n.title, "A Title");
        assert!(n.note_type.is_none());
    }

    #[test]
    fn malformed_frontmatter_degrades_to_body() {
        // unterminated frontmatter → treat whole thing as body, fallback id
        let n = parse("---\nid: x\nno closing", "fb");
        assert_eq!(n.id, "fb");
    }

    #[test]
    fn one_bad_field_does_not_discard_the_rest() {
        // `type` is a sequence (wrong type) — it alone becomes None, others survive.
        let raw = "---\nid: ok\ntype:\n  - not\n  - scalar\nstatus: active\n---\nbody\n";
        let n = parse(raw, "fb");
        assert_eq!(n.id, "ok");
        assert_eq!(n.status.as_deref(), Some("active"));
        assert_eq!(n.note_type, None);
    }

    #[test]
    fn anchors_accept_scalar_or_list() {
        assert_eq!(parse("---\nid: a\nanchors: solo\n---\nb", "fb").anchors, vec!["solo"]);
        let list = parse("---\nid: a\nanchors:\n  - x\n  - y\n---\nb", "fb").anchors;
        assert_eq!(list, vec!["x", "y"]);
    }

    #[test]
    fn closing_fence_tolerates_trailing_whitespace() {
        let n = parse("---\nid: x\n--- \nbody\n", "fb");
        assert_eq!(n.id, "x");
        assert!(n.body.contains("body"));
    }
}
