//! Dependency-free REFERENCE impls of the semantic seams, behind
//! `feature = "semantic"`. They exist to (a) prove the [Embedder]/[VectorStore]
//! traits compose end-to-end and (b) keep the gated code from bit-rotting — NOT to
//! be the production stack. The production swap (fastembed-rs ONNX embedder +
//! hnsw_rs persisted ANN) lands behind the same traits at enablement time. No heavy
//! deps (no ONNX, no HNSW) enter the tree for the reference impl.

use std::sync::Mutex;

use super::vector::{DocId, Embedder, VectorStore};

const DIMS: usize = 64;

/// Deterministic hashed bag-of-tokens embedding, L2-normalized. Real vectors (not a
/// stub), but only lexical-grade quality — the production embedder replaces it.
pub struct HashEmbedder;

impl Embedder for HashEmbedder {
    fn id(&self) -> &str {
        "reference-hash-v1"
    }
    fn dims(&self) -> usize {
        DIMS
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|t| embed_one(t)).collect())
    }
}

fn embed_one(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; DIMS];
    for tok in text.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
        v[(djb2(&tok.to_lowercase()) as usize) % DIMS] += 1.0;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn djb2(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(b as u64);
    }
    h
}

/// Brute-force in-memory cosine kNN. A real query (not a stub); the production
/// VectorStore (hnsw_rs ANN, persisted) replaces it.
pub struct BruteForceStore {
    embedder_id: String,
    rows: Mutex<Vec<(DocId, Vec<f32>)>>,
}

impl BruteForceStore {
    pub fn new(embedder_id: &str) -> Self {
        Self { embedder_id: embedder_id.to_string(), rows: Mutex::new(Vec::new()) }
    }
}

impl VectorStore for BruteForceStore {
    fn embedder_id(&self) -> &str {
        &self.embedder_id
    }
    fn upsert(&self, ids: &[DocId], vectors: &[Vec<f32>]) -> Result<(), String> {
        if ids.len() != vectors.len() {
            return Err("ids/vectors length mismatch".into());
        }
        let mut rows = self.rows.lock().map_err(|e| e.to_string())?;
        for (id, vec) in ids.iter().zip(vectors) {
            rows.retain(|(d, _)| d != id); // upsert = replace
            rows.push((id.clone(), vec.clone()));
        }
        Ok(())
    }
    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String> {
        let rows = self.rows.lock().map_err(|e| e.to_string())?;
        let mut scored: Vec<(DocId, f32)> =
            rows.iter().map(|(id, v)| (id.clone(), cosine(vector, v))).collect();
        // best-first; id as a deterministic tie-break.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0))
        });
        scored.truncate(k);
        Ok(scored)
    }
}

/// Cosine similarity. Vectors are pre-normalized, so the dot product IS the cosine.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seams compose end-to-end: embed → upsert → query ranks the
    /// semantically-closer doc first.
    #[test]
    fn seams_roundtrip_ranks_similar_first() {
        let emb = HashEmbedder;
        assert_eq!(emb.dims(), DIMS);
        let store = BruteForceStore::new(emb.id());
        let docs = vec![
            "fn authenticate user login password".to_string(),
            "struct RenderPipeline gpu shader".to_string(),
        ];
        let vecs = emb.embed(&docs).unwrap();
        store.upsert(&["p\u{0}auth.rs".into(), "p\u{0}render.rs".into()], &vecs).unwrap();
        let q = emb.embed(&["user login session".to_string()]).unwrap();
        let hits = store.query(&q[0], 2).unwrap();
        assert_eq!(hits[0].0, "p\u{0}auth.rs", "closer doc ranks first: {hits:?}");
        assert_eq!(store.embedder_id(), "reference-hash-v1");
    }

    #[test]
    fn upsert_replaces_not_duplicates() {
        let store = BruteForceStore::new("reference-hash-v1");
        store.upsert(&["a".into()], &[vec![1.0; DIMS]]).unwrap();
        store.upsert(&["a".into()], &[vec![1.0; DIMS]]).unwrap();
        assert_eq!(store.query(&vec![1.0; DIMS], 10).unwrap().len(), 1, "upsert replaces");
    }
}
