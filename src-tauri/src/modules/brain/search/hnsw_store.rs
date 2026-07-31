//! V2.5 — the REAL [VectorStore] backed by `hnsw_rs` (Hierarchical Navigable Small
//! World ANN), behind `feature = "semantic"`. Pure Rust (no ONNX/network), so it
//! builds + tests offline — unlike the embedder, which needs a model download.
//!
//! HNSW is append-only (no in-place delete), so REPLACE is implemented by
//! tombstone: every `upsert` appends, and a per-DocId `live` map points at the
//! NEWEST insertion — `query` filters superseded/removed insertions, so a stale
//! embedding can never outrank the current one after an edit (ADR-010 cluster 7).
//! `remove` tombstones the same way. Cosine DISTANCE is converted to a higher-is-better score
//! `1 - distance`; hnsw_rs DistCosine returns `1 - cosΘ ∈ [0, 2]`, so the score is
//! cosine similarity in `[-1, 1]` (same range as the BruteForceStore reference) — NOT
//! `[0, 1]`. The RRF fuser consumes leg RANK, not score magnitude, so the sign is
//! irrelevant there; anything reading the magnitude must handle `[-1, 1]`.
//!
//! NOT determinism-stable across runs: hnsw_rs seeds its layer RNG from OS entropy
//! (`from_os_rng()`, no seeding API), so the graph topology — and thus the ANN result
//! ordering — can differ run to run for identical input. This store therefore CANNOT
//! back a cache-stable gist key as-is; use the deterministic BruteForceStore (or pin a
//! seedable hnsw build) if/when the vector leg is folded into the gist hash.

use std::sync::Mutex;

use hnsw_rs::prelude::{DistCosine, Hnsw};

use super::vector::{DocId, VectorStore};

/// HNSW build params (modest). NB: hnsw_rs layer assignment is RNG-seeded from OS
/// entropy, so these params do NOT make the graph reproducible across runs.
const MAX_NB_CONN: usize = 16;
const MAX_LAYER: usize = 16;
const EF_CONSTRUCTION: usize = 200;
const EF_SEARCH: usize = 64;

struct Inner {
    hnsw: Hnsw<'static, f32, DistCosine>,
    /// data-id (insertion index) → DocId, so search results map back to DocIds.
    ids: Vec<DocId>,
    /// DocId → its CURRENT (live) data-id. hnsw_rs cannot delete in place, so
    /// replace/remove are tombstones: superseded insertions stay in the graph but
    /// only the live one may surface from `query`.
    live: std::collections::HashMap<DocId, usize>,
}

pub struct HnswStore {
    embedder_id: String,
    inner: Mutex<Inner>,
}

impl HnswStore {
    /// Build an empty store sized for up to `capacity` vectors of `embedder_id`.
    pub fn new(embedder_id: &str, capacity: usize) -> Self {
        let hnsw =
            Hnsw::<f32, DistCosine>::new(MAX_NB_CONN, capacity.max(1), MAX_LAYER, EF_CONSTRUCTION, DistCosine {});
        Self {
            embedder_id: embedder_id.to_string(),
            inner: Mutex::new(Inner {
                hnsw,
                ids: Vec::new(),
                live: std::collections::HashMap::new(),
            }),
        }
    }
}

impl VectorStore for HnswStore {
    fn embedder_id(&self) -> &str {
        &self.embedder_id
    }

    fn upsert(&self, ids: &[DocId], vectors: &[Vec<f32>]) -> Result<(), String> {
        if ids.len() != vectors.len() {
            return Err("ids/vectors length mismatch".into());
        }
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        for (id, vec) in ids.iter().zip(vectors) {
            let data_id = inner.ids.len();
            inner.hnsw.insert((vec, data_id));
            inner.ids.push(id.clone());
            // REPLACE semantics: the newest insertion becomes the live one; any
            // prior insertion of this id is tombstoned (filtered at query time).
            inner.live.insert(id.clone(), data_id);
        }
        Ok(())
    }

    fn remove(&self, ids: &[DocId]) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        for id in ids {
            inner.live.remove(id); // tombstone — the insertion stays but never surfaces
        }
        Ok(())
    }

    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String> {
        let inner = self.inner.lock().map_err(|e| e.to_string())?;
        if inner.live.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        // Over-fetch by the tombstone count: among any (k + stale) nearest
        // insertions at most `stale` are dead, so k live docs stay reachable (each
        // live DocId has exactly ONE live insertion — no same-doc duplicates).
        // ponytail: the window grows with churn, degrading toward a full scan for a
        // long-lived store under heavy re-upserts; the intended flow (rebuild from
        // persisted embeddings at boot) resets it. Upgrade path = periodically
        // rebuild the graph from the live set once live churn is real.
        let stale = inner.ids.len() - inner.live.len();
        let knbn = (k + stale).min(inner.ids.len());
        let neighbours = inner.hnsw.search(vector, knbn, EF_SEARCH);
        let mut out = Vec::with_capacity(k);
        for n in neighbours {
            let Some(doc) = inner.ids.get(n.d_id) else { continue };
            if inner.live.get(doc) != Some(&n.d_id) {
                continue; // superseded (re-upserted) or removed insertion
            }
            // cosine DISTANCE → higher-is-better similarity score.
            out.push((doc.clone(), 1.0 - n.distance));
            if out.len() == k {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(dims: usize, axis: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; dims];
        v[axis] = 1.0;
        v
    }

    #[test]
    fn hnsw_returns_nearest_first() {
        let store = HnswStore::new("reference-hash-v1", 16);
        let dims = 8;
        // three orthogonal unit vectors on distinct axes.
        store
            .upsert(
                &["p\u{0}a".into(), "p\u{0}b".into(), "p\u{0}c".into()],
                &[unit(dims, 0), unit(dims, 1), unit(dims, 2)],
            )
            .unwrap();
        // query closest to axis-1 → "b" must be rank-1.
        let mut q = unit(dims, 1);
        q[0] = 0.1; // slight lean toward a, but b still nearest
        let hits = store.query(&q, 3).unwrap();
        assert_eq!(hits[0].0, "p\u{0}b", "nearest by cosine ranks first: {hits:?}");
        assert!(hits[0].1 >= hits[1].1, "scores are higher-is-better (1 - distance)");
        assert_eq!(store.embedder_id(), "reference-hash-v1");
    }

    #[test]
    fn query_dedups_reinserted_id() {
        let store = HnswStore::new("e", 16);
        let v = unit(4, 0);
        store.upsert(&["x".into()], std::slice::from_ref(&v)).unwrap();
        store.upsert(&["x".into()], std::slice::from_ref(&v)).unwrap(); // re-insert same id
        let hits = store.query(&v, 10).unwrap();
        assert_eq!(hits.iter().filter(|(d, _)| d == "x").count(), 1, "re-inserted id returns once");
    }

    /// ADR-010 cluster 7: upsert REPLACES — after a re-embed the stale vector must
    /// never win. Pre-fix, both insertions stayed live and a query near the OLD
    /// vector returned the stale (nearest) one with similarity ~1.0.
    ///
    /// hnsw_rs ANN on a tiny RNG-layered graph (OS-seeded, see module docs) may
    /// legitimately return FEWER neighbours than requested — e.g. only the
    /// tombstoned insertion, which the live-map filter drops — so this asserts the
    /// deterministic REPLACE invariant (the stale vector can never surface), not
    /// exact ANN hit counts. The pre-fix bug still fails deterministically: both
    /// insertions live → two "x" hits and/or a ~1.0 score at the OLD embedding.
    #[test]
    fn upsert_replaces_stale_embedding() {
        let store = HnswStore::new("e", 16);
        let old = unit(4, 0);
        let new = unit(4, 1); // orthogonal: cosine(old, new) = 0
        store.upsert(&["x".into()], std::slice::from_ref(&old)).unwrap();
        store.upsert(&["x".into()], std::slice::from_ref(&new)).unwrap(); // the edit / re-embed
        // Query at the OLD embedding: the only admissible hit is the live (new)
        // vector at ~0 similarity — the stale one would score ~1.0 here.
        let hits = store.query(&old, 5).unwrap();
        assert!(hits.len() <= 1, "one live doc → at most one hit: {hits:?}");
        if let Some((doc, score)) = hits.first() {
            assert_eq!(doc, "x");
            assert!(*score < 0.5, "stale vector must not win: score {score}");
        }
        // Query at the NEW embedding: if the live doc surfaces, it scores as itself.
        let hits = store.query(&new, 5).unwrap();
        assert!(hits.len() <= 1, "one live doc → at most one hit: {hits:?}");
        if let Some((doc, score)) = hits.first() {
            assert_eq!(doc, "x");
            assert!(*score > 0.9, "live vector scores as itself: {score}");
        }
    }

    /// ADR-010 cluster 7: a removed id never surfaces again (unknown ids no-op).
    /// Same nondeterminism caveat as [upsert_replaces_stale_embedding]: assert the
    /// tombstone invariant, not exact ANN hit counts.
    #[test]
    fn remove_tombstones_the_id() {
        let store = HnswStore::new("e", 16);
        store.upsert(&["x".into(), "y".into()], &[unit(4, 0), unit(4, 1)]).unwrap();
        store.remove(&["x".into(), "never-existed".into()]).unwrap();
        let hits = store.query(&unit(4, 0), 10).unwrap();
        assert!(hits.iter().all(|(d, _)| d != "x"), "removed id must not surface: {hits:?}");
        assert!(hits.len() <= 1, "only 'y' is live: {hits:?}");
        if let Some((doc, _)) = hits.first() {
            assert_eq!(doc, "y");
        }
        // Removing everything → empty results (deterministic short-circuit), no panic.
        store.remove(&["y".into()]).unwrap();
        assert!(store.query(&unit(4, 0), 3).unwrap().is_empty());
    }

    #[test]
    fn empty_store_and_length_mismatch() {
        let store = HnswStore::new("e", 4);
        assert!(store.query(&[1.0, 0.0], 5).unwrap().is_empty());
        assert!(store.upsert(&["a".into(), "b".into()], &[vec![1.0]]).is_err());
    }
}
