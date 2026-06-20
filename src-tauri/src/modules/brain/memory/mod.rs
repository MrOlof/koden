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

#[derive(Default, serde::Deserialize)]
struct Frontmatter {
    id: Option<String>,
    title: Option<String>,
    #[serde(rename = "type")]
    note_type: Option<String>,
    scope: Option<String>,
    provenance: Option<String>,
    status: Option<String>,
    created: Option<String>,
    revalidate_after: Option<String>,
    supersedes: Option<String>,
    superseded_by: Option<String>,
    #[serde(default)]
    anchors: Vec<String>,
}

/// Parse a markdown note (optional leading `---` YAML frontmatter + body).
/// `fallback_id` (typically the filename stem) is used when frontmatter has no id.
pub fn parse(raw: &str, fallback_id: &str) -> MemoryNote {
    let (fm_str, body) = split_frontmatter(raw);
    let fm: Frontmatter = fm_str
        .and_then(|s| serde_yaml::from_str::<Frontmatter>(s).ok())
        .unwrap_or_default();
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
        let closes_line = after.is_empty()
            || after.starts_with('\n')
            || after.starts_with('\r');
        if at_line_start && closes_line {
            let fm = &after_open[..idx];
            let body = after.trim_start_matches(['\r', '\n']);
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
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("note");
        let note = parse(&raw, stem);
        let hash = crate::modules::brain::freshness::hash::hash_bytes(raw.as_bytes());
        let rel = format!(
            "{MEMORY_DIR}/{}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("")
        );
        if index.upsert_note(project_id, &note, &rel, &hash).is_ok() {
            count += 1;
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
}
