//! V2.5 — the REAL [VectorStore] backed by `hnsw_rs` (Hierarchical Navigable Small
//! World ANN), behind `feature = "semantic"`. Pure Rust (no ONNX/network), so it
//! builds + tests offline — unlike the embedder, which needs a model download.
//!
//! HNSW is append-only (no in-place delete), which matches the brain's model: the
//! vector index is rebuilt from the persisted embeddings, not mutated in place. So
//! `upsert` APPENDS; `query` de-dups by DocId (keeps the nearest) so a re-inserted
//! id never returns twice. Cosine DISTANCE is converted to a higher-is-better score
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
            inner: Mutex::new(Inner { hnsw, ids: Vec::new() }),
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
        }
        Ok(())
    }

    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String> {
        let inner = self.inner.lock().map_err(|e| e.to_string())?;
        if inner.ids.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        // Over-fetch so de-dup by DocId can still return k distinct docs.
        // ponytail: k*2 assumes ≤1 duplicate per slot — true for the intended flow
        // (rebuild-from-persisted → each id inserted once). If a single id is
        // re-upserted enough to crowd the 2k window it can under-return; grow the
        // fetch (re-search up to ids.len()) only if live repeated upserts become real.
        let knbn = (k * 2).min(inner.ids.len());
        let neighbours = inner.hnsw.search(vector, knbn, EF_SEARCH);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(k);
        for n in neighbours {
            let Some(doc) = inner.ids.get(n.d_id) else { continue };
            if seen.insert(doc.clone()) {
                // cosine DISTANCE → higher-is-better similarity score.
                out.push((doc.clone(), 1.0 - n.distance));
                if out.len() == k {
                    break;
                }
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
        store.upsert(&["x".into()], &[v.clone()]).unwrap();
        store.upsert(&["x".into()], &[v.clone()]).unwrap(); // re-insert same id
        let hits = store.query(&v, 10).unwrap();
        assert_eq!(hits.iter().filter(|(d, _)| d == "x").count(), 1, "re-inserted id returns once");
    }

    #[test]
    fn empty_store_and_length_mismatch() {
        let store = HnswStore::new("e", 4);
        assert!(store.query(&[1.0, 0.0], 5).unwrap().is_empty());
        assert!(store.upsert(&["a".into(), "b".into()], &[vec![1.0]]).is_err());
    }
}
