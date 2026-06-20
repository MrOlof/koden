//! Reciprocal Rank Fusion with **first-class per-leg weights** (k=60) — the
//! Koden improvement over Conductr's "duplicate the list N times" weighting hack
//! (`rrf.ts` + `hybrid-search.ts:193/213/229/263-266`). CONCEPT [DP-9].
//!
//! `score(d) = Σ_legs w_leg · 1/(k + rank_d_in_leg)`
//!
//! Deterministic tie-break: score descending, then id ascending — mirrors
//! `rrf.ts:25-27`/`lexical.ts:217` so results are stable across runs (required
//! by the cache-stable-gist guarantee downstream).
//!
//! BM25 itself is delegated to SQLite FTS5's built-in `bm25()` (which hardcodes
//! k1=1.2, b=0.75 — identical to Conductr's `K1`/`B`, `lexical.ts:11-12`) with
//! first-class per-column weights, so the per-field "path ~3×" weight is also a
//! real parameter rather than Conductr's string-repetition hack (`code/search.ts:9-24`).

/// Default RRF constant (Conductr `rrf.ts:9`).
pub const RRF_K: f64 = 60.0;

/// One ranked leg of the fusion, with its weight.
pub struct Leg<'a> {
    pub weight: f64,
    /// Ids ranked best-first (rank = index + 1).
    pub ranked: &'a [String],
}

/// Fuse weighted ranked legs into a single ranking. Returns `(id, score)` pairs,
/// best-first, with the deterministic tie-break.
pub fn weighted_rrf(legs: &[Leg]) -> Vec<(String, f64)> {
    use std::collections::HashMap;
    let mut fused: HashMap<String, f64> = HashMap::new();
    for leg in legs {
        for (idx, id) in leg.ranked.iter().enumerate() {
            let rank = (idx + 1) as f64;
            *fused.entry(id.clone()).or_insert(0.0) += leg.weight * (1.0 / (RRF_K + rank));
        }
    }
    let mut v: Vec<(String, f64)> = fused.into_iter().collect();
    v.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fuses_two_legs_with_equal_weight() {
        let a = ids(&["x", "y", "z"]);
        let b = ids(&["y", "x", "w"]);
        let out = weighted_rrf(&[
            Leg { weight: 1.0, ranked: &a },
            Leg { weight: 1.0, ranked: &b },
        ]);
        // y appears at ranks 2 and 1; x at 1 and 2 — equal score; tie-break → x.
        assert_eq!(out[0].0, "x");
        assert_eq!(out[1].0, "y");
    }

    #[test]
    fn weight_shifts_ranking() {
        let a = ids(&["a"]); // only in leg A
        let b = ids(&["b"]); // only in leg B
        // Heavier weight on B should rank b above a.
        let out = weighted_rrf(&[
            Leg { weight: 1.0, ranked: &a },
            Leg { weight: 5.0, ranked: &b },
        ]);
        assert_eq!(out[0].0, "b");
    }

    #[test]
    fn deterministic_tiebreak() {
        let a = ids(&["m", "n"]);
        let out1 = weighted_rrf(&[Leg { weight: 1.0, ranked: &a }]);
        let out2 = weighted_rrf(&[Leg { weight: 1.0, ranked: &a }]);
        assert_eq!(out1, out2);
    }
}
