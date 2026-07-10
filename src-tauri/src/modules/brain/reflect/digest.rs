//! Bounded corpus digest for the reflect call. Ports Conductr's `buildDigest` +
//! `buildFindingsSummary` + `truncate` (`reflect-llm.ts:194-236,284-287`), adapted
//! to Koden's `NoteSummary` + doctor `Finding` types.
//!
//! Koden DEVIATION (by design): the per-note body proxy is index METADATA
//! (type/status/anchors), not the raw note text, so the model never sees raw note
//! bodies and the `MAX_NOTE_CHARS` truncate bounds that metadata line (not note
//! content). Digesting a *redacted* body excerpt — making the 200-char cap
//! load-bearing — is a documented refinement.
//!
//! Secret-safety ([§7.1] hard gate) — defense in depth, NOT relied on here alone:
//! note titles + anchors are redacted at scan, and `reflect_with_client` runs the
//! ENTIRE assembled message through `secrets::redact` immediately before the cloud
//! send (so even a finding `detail` interpolating raw frontmatter can't leak). Raw
//! note bodies are never sourced.

use crate::modules::brain::memory::doctor::Finding;
use crate::modules::brain::memory::NoteSummary;

use super::schema::{MAX_NOTE_CHARS, MAX_NOTES};

/// First-20 cap for the findings summary, mirroring Conductr (`reflect-llm.ts:230`).
const MAX_FINDING_LINES: usize = 20;

/// Bound on the note title echoed into a finding line — enough to identify the note,
/// short enough not to bloat the digest (or the token estimate).
const MAX_FINDING_TITLE_CHARS: usize = 80;

/// Char-cut a note title for a finding line (single U+2026 when cut).
fn cut_title(title: &str) -> String {
    let clean = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() > MAX_FINDING_TITLE_CHARS {
        format!("{}\u{2026}", clean.chars().take(MAX_FINDING_TITLE_CHARS).collect::<String>())
    } else {
        clean
    }
}

/// Normalize whitespace then cut to `MAX_NOTE_CHARS`, appending a single U+2026
/// ellipsis when cut (`truncate`, `reflect-llm.ts:284-287`). Char-based (Unicode
/// scalar) rather than UTF-16 units; identical for ASCII, the common case.
fn truncate(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() > MAX_NOTE_CHARS {
        let cut: String = clean.chars().take(MAX_NOTE_CHARS).collect();
        format!("{cut}\u{2026}")
    } else {
        clean
    }
}

/// Build the memory digest: first `MAX_NOTES` notes in the caller's order (no
/// sort — order-preserving, like Conductr), one line each. `(no notes)` when empty.
pub fn build_digest(notes: &[NoteSummary]) -> String {
    if notes.is_empty() {
        return "(no notes)".to_string();
    }
    let lines: Vec<String> = notes
        .iter()
        .take(MAX_NOTES)
        .map(|n| {
            let label = n.note_type.as_deref().unwrap_or("note");
            // Body proxy = status + anchors metadata (NOT the raw file body).
            let mut bits: Vec<String> = Vec::new();
            if let Some(s) = &n.status {
                if !s.is_empty() {
                    bits.push(format!("status={s}"));
                }
            }
            if !n.anchors.is_empty() {
                bits.push(format!("anchors={}", n.anchors.join(",")));
            }
            let text = bits.join(" ");
            if text.is_empty() {
                format!("- [{label}] {}", n.title)
            } else {
                format!("- [{label}] {}: {}", n.title, truncate(&text))
            }
        })
        .collect();
    lines.join("\n")
}

/// Summarize the doctor findings: a header (counts + per-severity breakdown) plus
/// the first `MAX_FINDING_LINES` findings (em-dashes U+2014). Ports
/// `buildFindingsSummary` (`reflect-llm.ts:222-236`), using Koden's own severities.
pub fn build_findings_summary(note_count: usize, findings: &[Finding]) -> String {
    let mut by_sev: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for f in findings {
        *by_sev.entry(f.severity).or_insert(0) += 1;
    }
    let breakdown = if by_sev.is_empty() {
        "none".to_string()
    } else {
        by_sev.iter().map(|(s, n)| format!("{s}: {n}")).collect::<Vec<_>>().join(", ")
    };
    let header = format!(
        "{note_count} note(s), {} finding(s) \u{2014} {breakdown}",
        findings.len()
    );
    if findings.is_empty() {
        return format!("{header}\n(no findings)");
    }
    let lines: Vec<String> = findings
        .iter()
        .take(MAX_FINDING_LINES)
        .map(|f| {
            // Name the target note (id + title) so the model can write an actionable
            // card ("archive note n7 'Sessions expire…'") instead of the anonymous
            // "a revalidate-dated note is expired". Global findings have no note.
            let where_ = match (f.note_id.as_deref(), f.title.trim()) {
                (Some(id), t) if !t.is_empty() => format!("{id}: {}", cut_title(t)),
                (Some(id), _) => id.to_string(),
                (None, _) => "global".to_string(),
            };
            format!("- [{}] {} ({where_}) \u{2014} {}", f.severity, f.check, f.detail)
        })
        .collect();
    format!("{header}\n{}", lines.join("\n"))
}

/// Assemble the full reflect user message (Conductr `reflect-llm.ts:90`).
pub fn build_user_message(notes: &[NoteSummary], findings: &[Finding]) -> String {
    format!(
        "## Memory Digest\n\n{}\n\n## Doctor Findings\n\n{}",
        build_digest(notes),
        build_findings_summary(notes.len(), findings)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::memory::proposal::ProposalAction;

    fn note(id: &str, title: &str) -> NoteSummary {
        NoteSummary {
            id: id.into(),
            title: title.into(),
            note_type: Some("decision".into()),
            status: Some("active".into()),
            path: format!(".koden-memory/{id}.md"),
            anchors: vec![],
        }
    }

    #[test]
    fn digest_caps_to_max_notes_in_order() {
        let notes: Vec<NoteSummary> = (0..MAX_NOTES + 10).map(|i| note(&format!("n{i}"), &format!("Title {i}"))).collect();
        let d = build_digest(&notes);
        assert_eq!(d.lines().count(), MAX_NOTES, "capped to MAX_NOTES");
        assert!(d.starts_with("- [decision] Title 0"), "order preserved: {}", &d[..40]);
    }

    #[test]
    fn truncate_adds_single_ellipsis_after_cap() {
        let long = "x".repeat(MAX_NOTE_CHARS + 50);
        let t = truncate(&long);
        assert_eq!(t.chars().count(), MAX_NOTE_CHARS + 1, "200 chars + 1 ellipsis");
        assert!(t.ends_with('\u{2026}'));
    }

    #[test]
    fn empty_corpus_renders_placeholders() {
        assert_eq!(build_digest(&[]), "(no notes)");
        let s = build_findings_summary(0, &[]);
        assert!(s.contains("0 note(s), 0 finding(s)") && s.contains("(no findings)"), "{s}");
    }

    #[test]
    fn finding_line_names_its_target_note_id_and_title() {
        let findings = vec![
            Finding {
                check: "stale_revalidate",
                severity: "medium",
                note_id: Some("n7".into()),
                title: "Sessions expire after a 24h TTL".into(),
                detail: "revalidate_after has passed".into(),
                action: ProposalAction::Archive,
            },
            Finding {
                check: "orphan_project",
                severity: "low",
                note_id: None,
                title: String::new(),
                detail: "project-level note".into(),
                action: ProposalAction::Update,
            },
        ];
        let s = build_findings_summary(1, &findings);
        // The note-scoped finding carries BOTH id and title so a card can name it.
        assert!(s.contains("(n7: Sessions expire after a 24h TTL)"), "id+title present: {s}");
        // A global finding still renders with the `global` marker, no title.
        assert!(s.contains("(global)"), "global finding unchanged: {s}");
    }

    #[test]
    fn long_finding_title_is_bounded() {
        let findings = vec![Finding {
            check: "stale_revalidate",
            severity: "low",
            note_id: Some("n1".into()),
            title: "x".repeat(200),
            detail: "d".into(),
            action: ProposalAction::Archive,
        }];
        let s = build_findings_summary(1, &findings);
        assert!(s.contains('\u{2026}'), "over-long title is ellipsized: {s}");
    }

    #[test]
    fn findings_summary_buckets_by_severity_and_caps() {
        let findings: Vec<Finding> = (0..25)
            .map(|i| Finding {
                check: "missing_type",
                severity: if i % 2 == 0 { "low" } else { "medium" },
                note_id: Some(format!("n{i}")),
                title: format!("t{i}"),
                detail: format!("d{i}"),
                action: ProposalAction::Update,
            })
            .collect();
        let s = build_findings_summary(3, &findings);
        assert!(s.contains("3 note(s), 25 finding(s)"), "{s}");
        assert!(s.contains("low: 13") && s.contains("medium: 12"), "breakdown: {s}");
        // header + first 20 finding lines.
        assert_eq!(s.lines().count(), 1 + MAX_FINDING_LINES);
    }
}
