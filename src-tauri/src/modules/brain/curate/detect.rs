//! Flow G stale-ADR detection + the two-stage significance gate (CONCEPT §5.4/§6).
//! Stage 1 is a free, transparent heuristic over per-note signals; it decides
//! SKIP / ESCALATE (let the Tier-2 LLM judge) / ACT (propose without paying the LLM).
//! Detection REUSES the P1 doctor's `check()` (broken_anchor, stale_revalidate) and
//! adds `superseded_present` (a newer note resolves this one's `superseded_by`).

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
/// Tuned so a LONE `broken_anchor` (0.6) SKIPs — the P1 doctor already proposes
/// re-anchoring it, so curation must not double-propose; broken_anchor only
/// contributes once stacked with another signal. A single strong signal
/// (passed_revalidate 1.0 or superseded_present 1.5) ESCALATEs (the LLM earns its
/// keep on the keep-as-history vs obsolete call); two stacked signals ACT ($0).
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

    for f in check(records, indexed_paths, now_date) {
        let Some(id) = f.note_id.as_deref() else { continue };
        match f.check {
            "stale_revalidate" => add(id, SIG_PASSED_REVALIDATE, W_REVALIDATE, &mut score, &mut signals),
            "broken_anchor" => add(id, SIG_BROKEN_ANCHOR, W_BROKEN_ANCHOR, &mut score, &mut signals),
            _ => {} // missing_type / broken_supersession aren't curation (archive) signals
        }
    }

    // superseded_present: a note whose `superseded_by` resolves to an existing note.
    let ids: HashSet<&str> = records.iter().map(|r| r.id.as_str()).collect();
    let mut superseded_by: BTreeMap<String, String> = BTreeMap::new();
    for r in records {
        if let Some(sb) = &r.superseded_by {
            if ids.contains(sb.as_str()) {
                add(&r.id, SIG_SUPERSEDED_PRESENT, W_SUPERSEDED, &mut score, &mut signals);
                superseded_by.insert(r.id.clone(), sb.clone());
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
        NoteRecord { id: id.into(), note_type: Some("decision".into()), revalidate_after: None, superseded_by: None, anchors: vec![] }
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
