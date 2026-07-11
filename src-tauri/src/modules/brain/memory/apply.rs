//! Materialize an APPROVED memory proposal onto disk (D2). The Librarian only ever
//! PROPOSES; approval is the exclusive writer — this module is that writer's file
//! half. Conductr's `apply-proposals.ts` is the reference: create writes a new note,
//! archive/supersede/update edit the target note's frontmatter/body. Koden's port
//! carried the proposal QUEUE but dropped the apply half, so review-inbox Approve was
//! a no-op; this closes it.
//!
//! Split of concerns: everything here is pure filesystem (read + frontmatter/body
//! edit + crash-safe write) so it is unit-testable without a store. The DB side
//! (read the proposal, re-scan the notes table, flip the proposal to `applied`) lives
//! in `store::sqlite::apply_proposal`, which orchestrates this. The note FILES are
//! file-canonical (git-committed, not journaled); only the notes TABLE + proposal row
//! are journaled, by the store.
//!
//! Content is already clean: a proposal's title/detail came from the redacted reflect
//! digest (CONCEPT §7.1), so no new redaction is added here.

use std::path::Path;

use super::proposal::ProposalAction;

/// The note `type:` for a NEWLY created note. Reflect maps Insight/ShouldRemember →
/// Create (`reflect::proposal::action_for`), so a create-family note is an insight.
/// Supersede reuses the SUPERSEDED note's own type instead (a JWT decision supersedes
/// a sessions decision — same kind), falling back to this only if the target has none.
const DEFAULT_NOTE_TYPE: &str = "insight";

/// Materialize one approved proposal's file change(s). `mem_dir` is
/// `<root>/.koden-memory`; `target_path` is the resolved absolute path of the target
/// note (required for archive/update/supersede; `None` for create). `target_id` is
/// the target note's id (supersede links to it). A missing/invalid target is a SOFT
/// error (`Err`) — the caller leaves the proposal pending.
pub fn materialize(
    mem_dir: &Path,
    action: ProposalAction,
    target_path: Option<&Path>,
    target_id: Option<&str>,
    title: &str,
    detail: &str,
    now_date: &str,
) -> Result<(), String> {
    match action {
        ProposalAction::Create => {
            let id = unique_slug(mem_dir, title);
            let content = build_note(&id, DEFAULT_NOTE_TYPE, title, detail, now_date, None);
            atomic_write(&mem_dir.join(format!("{id}.md")), &content)
                .map_err(|e| format!("write new note: {e}"))
        }
        ProposalAction::Archive => {
            let tp = target_path.ok_or("archive proposal has no resolved target note")?;
            let raw = std::fs::read_to_string(tp)
                .map_err(|e| format!("read target note {}: {e}", tp.display()))?;
            let updated = set_frontmatter_field(&raw, "status", "archived")?;
            atomic_write(tp, &updated).map_err(|e| format!("write archived note: {e}"))
        }
        ProposalAction::Update => {
            let tp = target_path.ok_or("update proposal has no resolved target note")?;
            let raw = std::fs::read_to_string(tp)
                .map_err(|e| format!("read target note {}: {e}", tp.display()))?;
            // Conservative v1: APPEND a dated update section; never rewrite existing
            // prose. Appending at EOF lands inside the note body (after the closing
            // frontmatter fence), so the frontmatter and every existing line survive
            // byte-for-byte.
            let mut updated = raw;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!("\n## Update ({now_date})\n\n{}\n", detail.trim()));
            atomic_write(tp, &updated).map_err(|e| format!("write updated note: {e}"))
        }
        ProposalAction::Supersede => {
            let tp = target_path.ok_or("supersede proposal has no resolved target note")?;
            let tid = target_id.ok_or("supersede proposal has no target id")?;
            let target_raw = std::fs::read_to_string(tp)
                .map_err(|e| format!("read target note {}: {e}", tp.display()))?;
            let note_type = super::parse(&target_raw, "note")
                .note_type
                .unwrap_or_else(|| DEFAULT_NOTE_TYPE.to_string());
            // New note first — it carries `supersedes: <target_id>`.
            let new_id = unique_slug(mem_dir, title);
            let content = build_note(&new_id, &note_type, title, detail, now_date, Some(tid));
            atomic_write(&mem_dir.join(format!("{new_id}.md")), &content)
                .map_err(|e| format!("write superseding note: {e}"))?;
            // Then wire the old note's back-edge.
            let updated = set_frontmatter_field(&target_raw, "superseded_by", &new_id)?;
            atomic_write(tp, &updated).map_err(|e| format!("write superseded note: {e}"))
        }
    }
}

/// Build a new note file: `---` frontmatter (id/type/title/status/created [+ optional
/// supersedes]) `---` then a `# title` heading and the detail as the body. Title is
/// double-quoted in the frontmatter so a `:` / `#` in it can't break the YAML; the
/// body needs no escaping (it is markdown).
fn build_note(
    id: &str,
    note_type: &str,
    title: &str,
    detail: &str,
    now_date: &str,
    supersedes: Option<&str>,
) -> String {
    let mut fm = String::new();
    fm.push_str(&format!("id: {id}\n"));
    fm.push_str(&format!("type: {note_type}\n"));
    fm.push_str(&format!("title: {}\n", yaml_double_quote(title)));
    fm.push_str("status: active\n");
    fm.push_str(&format!("created: {now_date}\n"));
    // Unquoted like `id:` above — a note id is a safe slug ([a-z0-9-]).
    if let Some(s) = supersedes {
        fm.push_str(&format!("supersedes: {s}\n"));
    }
    format!("---\n{fm}---\n# {}\n\n{}\n", title.trim(), detail.trim())
}

/// Set (or replace) one scalar frontmatter field, preserving every OTHER key and the
/// body verbatim. Parses only the frontmatter block through the SAME splitter the scan
/// uses (`memory::split_frontmatter`), edits the YAML mapping (insertion order kept),
/// and re-emits — so unknown keys survive (unlike a lossy round-trip via `MemoryNote`).
fn set_frontmatter_field(raw: &str, key: &str, value: &str) -> Result<String, String> {
    let (fm_opt, body) = super::split_frontmatter(raw);
    let mut map: serde_yaml::Mapping = match fm_opt {
        Some(fm) if !fm.trim().is_empty() => {
            serde_yaml::from_str(fm).map_err(|e| format!("frontmatter parse: {e}"))?
        }
        _ => serde_yaml::Mapping::new(),
    };
    map.insert(
        serde_yaml::Value::String(key.to_string()),
        serde_yaml::Value::String(value.to_string()),
    );
    let yaml = serde_yaml::to_string(&map).map_err(|e| format!("frontmatter serialize: {e}"))?;
    // `yaml` ends with a newline; the closing fence gets its own line.
    Ok(format!("---\n{yaml}---\n{body}"))
}

/// A collision-safe note id/filename stem: the slugified title, suffixed `-2`, `-3`, …
/// until `<slug>.md` does not already exist in `mem_dir` (Conductr's create drift
/// guard, made deterministic). The frontmatter id is set to the SAME value, so the
/// notes-table id == the file stem.
fn unique_slug(mem_dir: &Path, title: &str) -> String {
    let base = slugify(title);
    if !mem_dir.join(format!("{base}.md")).exists() {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !mem_dir.join(format!("{candidate}.md")).exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Lowercase, collapse every non-`[a-z0-9]` run to a single `-`, trim leading/trailing
/// `-`, cap at 60 chars, fall back to `"note"` (Conductr `slugify` parity).
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let capped: String = out.trim_matches('-').chars().take(60).collect();
    let capped = capped.trim_matches('-');
    if capped.is_empty() {
        "note".to_string()
    } else {
        capped.to_string()
    }
}

/// Minimal YAML double-quoted scalar: escape `\` and `"`, flatten any newline to a
/// space (note titles/ids are single-line). Enough for the free-text `title`/id fields.
fn yaml_double_quote(s: &str) -> String {
    let esc = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ");
    format!("\"{esc}\"")
}

/// Crash-safe write: temp sibling + rename (the atomic-replace idiom used by
/// `modules::agent`). `std::fs::rename` replaces an existing destination on both
/// Windows (MoveFileEx REPLACE_EXISTING) and Unix, so an in-place note edit can never
/// leave a half-written file. Creates the parent dir on demand (first-ever note).
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("note.md");
    let tmp = path.with_file_name(format!(".{fname}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Epoch-ms → `YYYY-MM-DD` (UTC), the `created:`/update-section date. Pure civil-date
/// arithmetic (days-from-civil inverse, Howard Hinnant) — the exact inverse of
/// `gist::iso_date_to_epoch_ms`, so a stamped date round-trips back to the same day.
pub fn epoch_ms_to_iso_date(ms: i64) -> String {
    const DAY_MS: i64 = 86_400_000;
    let days = ms.div_euclid(DAY_MS); // days since 1970-01-01
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::gist::iso_date_to_epoch_ms;

    #[test]
    fn slugify_matches_conductr_shape() {
        assert_eq!(slugify("Stripe webhook verifies signature!"), "stripe-webhook-verifies-signature");
        assert_eq!(slugify("  Trim -- Me  "), "trim-me");
        assert_eq!(slugify("***"), "note", "empty → fallback");
        assert_eq!(slugify("Ünïcödé Ok"), "n-c-d-ok", "non-ascii → dashes (a-z0-9 only)");
        assert!(slugify(&"x".repeat(200)).len() <= 60, "capped at 60");
    }

    #[test]
    fn date_round_trips_through_the_gist_parser() {
        const DAY_MS: i64 = 86_400_000;
        for &ms in &[0i64, DAY_MS, 1_752_000_000_000, 1_600_000_000_000] {
            let midnight = ms.div_euclid(DAY_MS) * DAY_MS;
            let iso = epoch_ms_to_iso_date(midnight);
            assert_eq!(iso_date_to_epoch_ms(&iso), Some(midnight), "round-trip {iso}");
        }
        assert_eq!(epoch_ms_to_iso_date(0), "1970-01-01");
    }

    #[test]
    fn set_frontmatter_field_preserves_other_keys_and_body() {
        let raw = "---\nid: old\ntype: decision\ntitle: Auth\nanchors:\n  - src/a.rs\n---\n# Auth\n\nBody prose.\n";
        let out = set_frontmatter_field(raw, "status", "archived").unwrap();
        let note = super::super::parse(&out, "fb");
        assert_eq!(note.id, "old");
        assert_eq!(note.note_type.as_deref(), Some("decision"));
        assert_eq!(note.status.as_deref(), Some("archived"));
        assert_eq!(note.anchors, vec!["src/a.rs"], "unknown/list key survived");
        assert!(note.body.contains("Body prose."), "body preserved");
    }

    #[test]
    fn build_note_is_parseable_with_expected_fields() {
        let c = build_note("my-id", "insight", "A: tricky # title", "The detail.", "2026-07-10", None);
        let note = super::super::parse(&c, "fb");
        assert_eq!(note.id, "my-id");
        assert_eq!(note.note_type.as_deref(), Some("insight"));
        assert_eq!(note.status.as_deref(), Some("active"));
        assert_eq!(note.created.as_deref(), Some("2026-07-10"));
        assert_eq!(note.title, "A: tricky # title", "quoted title survives : and #");
        assert!(note.body.contains("The detail."));
    }

    #[test]
    fn build_note_supersede_carries_forward_edge() {
        let c = build_note("new-id", "decision", "New", "d", "2026-07-10", Some("old-id"));
        let note = super::super::parse(&c, "fb");
        assert_eq!(note.supersedes.as_deref(), Some("old-id"));
    }

    #[test]
    fn materialize_create_then_update_appends_without_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".koden-memory");
        materialize(&mem, ProposalAction::Create, None, None, "First note", "Original body.", "2026-07-10")
            .unwrap();
        let path = mem.join("first-note.md");
        let before = std::fs::read_to_string(&path).unwrap();
        materialize(
            &mem,
            ProposalAction::Update,
            Some(&path),
            Some("first-note"),
            "ignored title",
            "An added observation.",
            "2026-07-11",
        )
        .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(before.trim_end()), "original bytes preserved as a prefix");
        assert!(after.contains("## Update (2026-07-11)"));
        assert!(after.contains("An added observation."));
    }

    #[test]
    fn materialize_missing_target_is_soft_error() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".koden-memory");
        let missing = mem.join("gone.md");
        let err = materialize(
            &mem,
            ProposalAction::Archive,
            Some(&missing),
            Some("gone"),
            "t",
            "d",
            "2026-07-10",
        )
        .unwrap_err();
        assert!(err.contains("read target note"), "clear error: {err}");
    }

    #[test]
    fn unique_slug_suffixes_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".koden-memory");
        std::fs::create_dir_all(&mem).unwrap();
        assert_eq!(unique_slug(&mem, "Note"), "note");
        std::fs::write(mem.join("note.md"), "x").unwrap();
        assert_eq!(unique_slug(&mem, "Note"), "note-2");
        std::fs::write(mem.join("note-2.md"), "x").unwrap();
        assert_eq!(unique_slug(&mem, "Note"), "note-3");
    }
}
