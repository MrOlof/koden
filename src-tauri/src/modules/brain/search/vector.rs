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
    /// Insert-or-replace `(id → vector)` pairs.
    fn upsert(&self, ids: &[DocId], vectors: &[Vec<f32>]) -> Result<(), String>;
    /// Top-`k` nearest by similarity, best-first, as `(id, score)`.
    fn query(&self, vector: &[f32], k: usize) -> Result<Vec<(DocId, f32)>, String>;
}
