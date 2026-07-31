//! Flow G stale-ADR detection + the two-stage significance gate (CONCEPT §5.4/§6).
//! Stage 1 is a free, transparent heuristic over per-note signals; it decides
//! SKIP / ESCALATE (let the Tier-2 LLM judge) / ACT (propose without paying the LLM).
//! Detection REUSES the P1 doctor's `check()` (broken_anchor, stale_revalidate) and
//! adds `superseded_present` (EITHER supersession edge resolves, §6 Flow G step 1).
//!
//! Deferred Flow G signals (CONCEPT:308-310, not yet implemented): "high churn in
//! the referenced area" (needs git/churn data) and "(LLM-only) direct contradiction
//! by a newer note" (a paid escalate path). The three free signals here are a
//! disclosed subset, not the full set.

use std::collections::{BTreeMap, HashSet};

use crate::modules::brain::memory::doctor::{check, NoteRecord};

pub const SIG_PASSED_REVALIDATE: &str = "passed_revalidate";
pub const SIG_BROKEN_ANCHOR: &str = "broken_anchor";
pub const SIG_SUPERSEDED_PRESENT: &str = "superseded_present";

/// Significance weights (CONCEPT [DP-18] — transparent + tunable, not learned).
const W_REVALIDATE: f64 = 1.0;
const W_BROKEN_ANCHOR: f64 = 0.6; // per broken anchor
const W_SUPERSEDED: f64 = 1.5; // a newer note supersedes this one — the strongest signal

/// Score bands. Below LOW: ignore. LOW..HIGH: escalate to the LLM (borderline —
/// worth a paid judgment). ≥ HIGH: act directly (signals already decisive; propose
/// the preserve-biased archive without spending a token).
///
/// Tuned so `broken_anchor` (0.6, saturated to one unit per note) can NEVER cross
/// LOW on its own — the P1 doctor already proposes re-anchoring, so curation must not
/// double-propose; it only contributes once stacked with another signal. A single
/// strong signal (passed_revalidate 1.0 or superseded_present 1.5) ESCALATEs (the LLM
/// earns its keep on the keep-as-history vs obsolete call). A pair reaches ACT ($0)
/// only when it includes superseded_present (e.g. superseded 1.5 + revalidate 1.0 =
/// 2.5 ≥ HIGH); weaker pairs (broken_anchor + revalidate = 1.6) still ESCALATE.
const LOW: f64 = 0.7;
const HIGH: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Skip,
    Escalate,
    Act,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub note_id: String,
    pub signals: Vec<&'static str>,
    pub score: f64,
    pub band: Band,
    /// The newer note id when `superseded_present` tripped (for the proposal detail).
    pub superseded_by: Option<String>,
}

fn band_for(score: f64) -> Band {
    if score < LOW {
        Band::Skip
    } else if score < HIGH {
        Band::Escalate
    } else {
        Band::Act
    }
}

/// Detect curation candidates over a project's notes. Pure + deterministic given
/// `now_date` (ISO `YYYY-MM-DD`; `None` disables the date-dependent staleness
/// signal). Returns only notes that crossed the SKIP floor, sorted by id.
pub fn detect_candidates(
    records: &[NoteRecord],
    indexed_paths: &HashSet<String>,
    now_date: Option<&str>,
) -> Vec<Candidate> {
    // Per-note signal accumulation. Reuse the doctor's findings for the two signals
    // it already computes (broken_anchor counted per occurrence; stale_revalidate).
    let mut score: BTreeMap<String, f64> = BTreeMap::new();
    let mut signals: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    let add = |id: &str, sig: &'static str, w: f64, score: &mut BTreeMap<String, f64>, signals: &mut BTreeMap<String, Vec<&'static str>>| {
        *score.entry(id.to_string()).or_insert(0.0) += w;
        let v = signals.entry(id.to_string()).or_default();
        if !v.contains(&sig) {
            v.push(sig);
        }
    };

    // broken_anchor is added at most ONCE per note (saturated), so a note with N
    // broken anchors can never cross LOW on that signal alone — the doctor already
    // proposes one re-anchor per broken anchor, and curation must not double-propose.
    let mut anchor_counted: HashSet<String> = HashSet::new();
    for f in check(records, indexed_paths, now_date) {
        let Some(id) = f.note_id.as_deref() else { continue };
        match f.check {
            "stale_revalidate" => add(id, SIG_PASSED_REVALIDATE, W_REVALIDATE, &mut score, &mut signals),
            // clippy wants this collapsed into a match guard, but `insert` mutates
            // `anchor_counted`. A side-effecting guard hides the dedupe; keep it explicit.
            #[allow(clippy::collapsible_match)]
            "broken_anchor" => {
                if anchor_counted.insert(id.to_string()) {
                    add(id, SIG_BROKEN_ANCHOR, W_BROKEN_ANCHOR, &mut score, &mut signals);
                }
            }
            _ => {} // missing_type / broken_supersession aren't curation (archive) signals
        }
    }

    // superseded_present: a note is superseded if EITHER edge resolves (CONCEPT
    // Flow G defines the signal via the NEWER note's forward `supersedes`; the OLD
    // note's back-link `superseded_by` is the common-but-not-guaranteed companion).
    // Union of both, self-references excluded (malformed data, not a real signal).
    let ids: HashSet<&str> = records.iter().map(|r| r.id.as_str()).collect();
    let mut superseded_by: BTreeMap<String, String> = BTreeMap::new();
    let mark_superseded = |stale: &str, newer: &str, score: &mut BTreeMap<String, f64>, signals: &mut BTreeMap<String, Vec<&'static str>>, superseded_by: &mut BTreeMap<String, String>| {
        if stale == newer {
            return; // self-supersession — not a curation signal
        }
        if superseded_by.contains_key(stale) {
            return; // already marked via the other edge — don't double-weight
        }
        add(stale, SIG_SUPERSEDED_PRESENT, W_SUPERSEDED, score, signals);
        superseded_by.insert(stale.to_string(), newer.to_string());
    };
    for r in records {
        // back-link: r.superseded_by → r is the stale one (target must exist).
        if let Some(sb) = &r.superseded_by {
            if ids.contains(sb.as_str()) {
                mark_superseded(&r.id, sb, &mut score, &mut signals, &mut superseded_by);
            }
        }
        // forward edge: r.supersedes → that target is the stale one (must exist).
        if let Some(sup) = &r.supersedes {
            if ids.contains(sup.as_str()) {
                mark_superseded(sup, &r.id, &mut score, &mut signals, &mut superseded_by);
            }
        }
    }

    score
        .into_iter()
        .filter_map(|(id, s)| {
            let band = band_for(s);
            if band == Band::Skip {
                return None;
            }
            Some(Candidate {
                signals: signals.remove(&id).unwrap_or_default(),
                superseded_by: superseded_by.remove(&id),
                note_id: id,
                score: s,
                band,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::brain::memory::doctor::NoteRecord;

    fn note(id: &str) -> NoteRecord {
        NoteRecord { id: id.into(), note_type: Some("decision".into()), revalidate_after: None, supersedes: None, superseded_by: None, anchors: vec![] }
    }

    #[test]
    fn forward_supersedes_edge_flags_the_target() {
        // Spec-correct corpus: NEW note b carries `supersedes: a`; a has NO back-link.
        // a must still become the candidate (the forward edge resolves to it).
        let a = note("a");
        let b = NoteRecord { supersedes: Some("a".into()), ..note("b") };
        let cands = detect_candidates(&[a, b], &HashSet::new(), None);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].note_id, "a", "the SUPERSEDED note is the candidate, not the newer one");
        assert!(cands[0].signals.contains(&SIG_SUPERSEDED_PRESENT));
        assert_eq!(cands[0].superseded_by.as_deref(), Some("b"));
    }

    #[test]
    fn both_edges_present_do_not_double_weight() {
        // a.superseded_by=b AND b.supersedes=a — the same relation from both sides
        // must mark `a` exactly once (score 1.5, ESCALATE — not 3.0/ACT).
        let a = NoteRecord { superseded_by: Some("b".into()), ..note("a") };
        let b = NoteRecord { supersedes: Some("a".into()), ..note("b") };
        let cands = detect_candidates(&[a, b], &HashSet::new(), None);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].note_id, "a");
        assert_eq!(cands[0].band, Band::Escalate, "single relation, not double-counted: {}", cands[0].score);
    }

    #[test]
    fn self_supersession_is_ignored() {
        let a = NoteRecord { superseded_by: Some("a".into()), supersedes: Some("a".into()), ..note("a") };
        assert!(detect_candidates(&[a], &HashSet::new(), None).is_empty(), "a note can't supersede itself");
    }

    #[test]
    fn multiple_broken_anchors_still_skip() {
        // 3 broken anchors must NOT cross LOW on their own (saturated to one unit) —
        // the doctor owns re-anchoring; curation must not escalate this case.
        let a = NoteRecord { anchors: vec!["src/a.rs".into(), "src/b.rs".into(), "src/c.rs".into()], ..note("a") };
        assert!(detect_candidates(&[a], &HashSet::new(), None).is_empty(), "many broken anchors alone still skip");
    }

    #[test]
    fn superseded_present_escalates() {
        // b supersedes a → a is a curation candidate via superseded_by resolving.
        let a = NoteRecord { superseded_by: Some("b".into()), ..note("a") };
        let b = note("b");
        let cands = detect_candidates(&[a, b], &HashSet::new(), None);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].note_id, "a");
        assert!(cands[0].signals.contains(&SIG_SUPERSEDED_PRESENT));
        assert_eq!(cands[0].superseded_by.as_deref(), Some("b"));
        assert_eq!(cands[0].band, Band::Escalate, "a single strong signal (1.5 < HIGH) → escalate to the LLM");
    }

    #[test]
    fn lone_broken_anchor_skips_doctor_owns_it() {
        // one broken anchor (0.6) is below LOW → curation skips (the doctor proposes
        // re-anchoring); it only counts once stacked with another signal.
        let a = NoteRecord { anchors: vec!["src/gone.rs".into()], ..note("a") };
        assert!(detect_candidates(&[a], &HashSet::new(), None).is_empty());
    }

    #[test]
    fn dangling_supersession_is_not_a_candidate() {
        // a.superseded_by points to a MISSING note → that's a doctor data-error,
        // NOT a 'this note is superseded' archive signal.
        let a = NoteRecord { superseded_by: Some("ghost".into()), ..note("a") };
        assert!(detect_candidates(&[a], &HashSet::new(), None).is_empty());
    }

    #[test]
    fn stacked_signals_reach_act_band() {
        // passed_revalidate (1.0) + superseded_present (1.5) = 2.5 ≥ HIGH → act.
        let mut indexed = HashSet::new();
        indexed.insert("src/x.rs".to_string());
        let a = NoteRecord {
            revalidate_after: Some("2000-01-01".into()),
            superseded_by: Some("b".into()),
            ..note("a")
        };
        let b = note("b");
        let cands = detect_candidates(&[a, b], &indexed, Some("2026-01-01"));
        let a_c = cands.iter().find(|c| c.note_id == "a").unwrap();
        assert_eq!(a_c.band, Band::Act, "decisive signals act without the LLM: {}", a_c.score);
        assert!(a_c.signals.contains(&SIG_PASSED_REVALIDATE) && a_c.signals.contains(&SIG_SUPERSEDED_PRESENT));
    }

    #[test]
    fn clean_note_skips() {
        assert!(detect_candidates(&[note("a")], &HashSet::new(), None).is_empty());
    }
}
