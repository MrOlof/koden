//! P3 — cache-stable gist injection (the payoff). Synthesize a token-bounded,
//! fingerprint-keyed context bundle from the index and inject it into agents via
//! the existing `--append-system-prompt` channel.
//!
//! NON-NEGOTIABLE (the P3 gate): an unchanged relaunch yields a BYTE-IDENTICAL
//! gist. The gist sits in the cacheable prompt prefix; a per-launch-mutating gist
//! busts the agent's prompt cache (~90% input-cost penalty). We guarantee this by
//! deriving the gist purely from deterministic index state (search is now fully
//! ordered, notes/symbols are sorted) and keying it by
//! `blake3(project_fingerprint ‖ intent ‖ budget ‖ schema_version)` (CONCEPT
//! §6 Flow C, [DP-21]/[DP-22]).
//!
//! Secret-safe: the gist draws only from the index — FTS content is pre-redacted,
//! note titles are redacted at scan, symbols/paths are identifiers. No raw source
//! is re-read here. Snippet-text enrichment is a P3 refinement.
//!
//! Layered + fail-open: the freshness line is always kept; relevant files + their
//! top symbols, then top memory notes, are added while the char budget allows.

use std::path::Path;

use crate::modules::brain::store;
use crate::modules::brain::store::schema::SCHEMA_VERSION;

const MAX_FILES: usize = 12;
const MAX_SYMS_PER_FILE: usize = 6;
const MAX_NOTES: usize = 8;
/// chars-per-token heuristic ([DP-21]) — no exact cross-vendor tokenizer.
const CHARS_PER_TOKEN: usize = 4;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Gist {
    pub bytes: String,
    /// Cache key — `blake3(fingerprint ‖ intent ‖ budget ‖ schema_version)`. An
    /// unchanged relaunch reproduces this key (and byte-identical `bytes`).
    pub fingerprint: String,
    pub sources: Vec<String>,
}

/// Build the gist for `intent` in `project_id` under a `budget_tokens` ceiling.
/// Zero tokens to build (pure index reads); deterministic → byte-stable.
pub fn build_gist(
    db_path: &Path,
    project_id: &str,
    project_name: &str,
    intent: &str,
    budget_tokens: usize,
) -> Gist {
    let fp = store::project_fingerprint_readonly(db_path, project_id).unwrap_or_default();
    let key = blake3::hash(
        format!("{fp}\u{0}{intent}\u{0}{budget_tokens}\u{0}{SCHEMA_VERSION}").as_bytes(),
    )
    .to_hex()
    .to_string();
    let file_count = store::file_count_readonly(db_path, project_id).unwrap_or(0);
    let fp_short = fp.get(..12).unwrap_or(&fp);

    // Always-kept freshness line (never trimmed).
    let freshness = format!("# Koden Brain · {project_name} · {file_count} files · fp:{fp_short}");
    let max_chars = budget_tokens
        .saturating_mul(CHARS_PER_TOKEN)
        .max(freshness.len() + 1);
    let mut out = freshness;
    let mut sources: Vec<String> = Vec::new();

    // Code layer: relevant files + their top symbols.
    let hits = store::search_readonly(db_path, Some(project_id), intent, MAX_FILES).unwrap_or_default();
    if !hits.is_empty() && push_line(&mut out, max_chars, "## Relevant files") {
        for h in hits.iter().take(MAX_FILES) {
            let syms = store::symbols_for_path_readonly(db_path, project_id, &h.path, MAX_SYMS_PER_FILE)
                .unwrap_or_default();
            let line = if syms.is_empty() {
                format!("- {}", h.path)
            } else {
                format!("- {} [{}]", h.path, syms.join(", "))
            };
            if push_line(&mut out, max_chars, &line) {
                sources.push(h.path.clone());
            } else {
                break;
            }
        }
    }

    // Memory layer: top notes (titles already redacted at scan).
    let notes = store::list_notes_readonly(db_path, Some(project_id)).unwrap_or_default();
    if !notes.is_empty() && push_line(&mut out, max_chars, "## Memory") {
        for n in notes.iter().take(MAX_NOTES) {
            let line = match &n.note_type {
                Some(t) => format!("- {} ({t})", n.title),
                None => format!("- {}", n.title),
            };
            if !push_line(&mut out, max_chars, &line) {
                break;
            }
        }
    }

    Gist { bytes: out, fingerprint: key, sources }
}

/// Append `line` (with a newline) iff it fits the char budget. Returns whether it
/// was added — callers stop a layer once a line is rejected (proportional trim).
fn push_line(out: &mut String, max_chars: usize, line: &str) -> bool {
    if out.len() + 1 + line.len() <= max_chars {
        out.push('\n');
        out.push_str(line);
        true
    } else {
        false
    }
}
