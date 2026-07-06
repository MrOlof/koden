//! The semantic seams — the stable traits a future vector leg fuses into the RRF
//! path with (EXECUTION_PLAN §5.1). No production impl compiles in the v1 default
//! build; the reference impl behind `feature = "semantic"` exercises these.

/// A document id in the vector store. Same `"project\0path"` composite the lexical
/// legs key on, so a future vector leg fuses by the SAME id space (no remapping).
pub type DocId = String;

/// Produces embeddings. The enablement-time production impl is fastembed-rs (ONNX,
/// local, no key for embedding) behind `feature = "semantic"`.
pub trait Embedder: Send + Sync {
    /// Stable id written to the `embedderId` header (e.g. `bge-small-en-v1.5`), so a
    /// later build can detect a model/dimension mismatch and rebuild rather than
    /// serve stale embeddings.
    fn id(&self) -> &str;
    /// Embedding dimensionality (must match the persisted header's `dims`).
    fn dims(&self) -> usize;
    /// Embed a batch of texts → one vector each (caller-parallel-safe; `&self`).
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

/// Stores + queries embeddings. The enablement-time production impl is hnsw_rs
/// (persisted ANN under `app_local_data_dir()/koden/brain/`) behind the feature.
pub trait VectorStore: Send + Sync {
    /// The embedder id this store was built with (mismatch ⇒ rebuild).
    fn embedder_id(&self) -> &str;
    /// Insert-or-replace `(id → vector)` pairs. REPLACE is load-bearing: after a
    /// re-embed (file edit) the old vector must never surface from `query` again.
    fn upsert(&self, ids: &[DocId], vectors: &[Vec<f32>]) -> Result<(), String>;
    /// Remove ids (unknown ids are a no-op). A removed id must never surface from
    /// `query` again — the delete path for pruned files (ADR-010 cluster 7).
    fn remove(&self, ids: &[DocId]) -> Result<(), String>;
    /// Top-`k` nearest, best-first, as `(id, score)`. `score` is higher-is-better
    /// cosine similarity in `[-1, 1]` (NOT `[0, 1]` — orthogonal is 0, opposed is
    /// negative); both impls agree. The RRF fuser uses leg RANK, not magnitude, so the
    /// sign is irrelevant there — but anything reading the score must handle `[-1, 1]`.
    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String>;
}
