//! V2.5 — the REAL [Embedder] backed by fastembed-rs (ONNX, local inference) behind
//! `feature = "semantic-embed"`. Default model BAAI/bge-small-en-v1.5 (384-dim).
//!
//! Why its own feature (vs the pure-Rust `semantic`/HNSW): fastembed pulls the ONNX
//! runtime (a network download at BUILD via `ort`) and downloads the model on first
//! use (network at RUNTIME). So this builds + bit-rot-checks in CI under
//! `--features semantic-embed`, but its real EMBED run is an online/GUI step — there
//! is no offline/$0 test of the model itself (the `#[ignore]`d smoke below needs the
//! network). The pure-Rust HNSW store + the seams are the offline-tested half.
//!
//! `TextEmbedding::embed` takes `&mut self`, so the model is wrapped in a `Mutex` to
//! satisfy the `Embedder: Send + Sync` contract (single-flight embedding — fine for
//! the worker's batched index-time use).

use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::vector::Embedder;

/// bge-small-en-v1.5 output dimensionality (must match the persisted embedderId dims).
const DIMS: usize = 384;
const EMBEDDER_ID: &str = "bge-small-en-v1.5";

pub struct FastembedEmbedder {
    model: Mutex<TextEmbedding>,
}

impl FastembedEmbedder {
    /// Load (downloading on first use) the default bge-small model. Network at first
    /// call; cached thereafter under the fastembed cache dir.
    pub fn new() -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(false),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { model: Mutex::new(model) })
    }
}

impl Embedder for FastembedEmbedder {
    fn id(&self) -> &str {
        EMBEDDER_ID
    }
    fn dims(&self) -> usize {
        DIMS
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let mut model = self.model.lock().map_err(|e| e.to_string())?;
        model.embed(texts, None).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ONLINE smoke (downloads the model on first run) — `#[ignore]`d so it never
    /// runs in the default/offline suite. Proves the real embedder + HNSW give
    /// SEMANTIC ranking (synonyms cluster), which the lexical index cannot:
    ///   `cargo test --features semantic-embed -- --ignored fastembed_real_embed`
    #[test]
    #[ignore]
    fn fastembed_real_embed_is_semantic() {
        use crate::modules::brain::search::hnsw_store::HnswStore;
        use crate::modules::brain::search::vector::VectorStore;

        let emb = FastembedEmbedder::new().expect("load model (needs network on first run)");
        assert_eq!(emb.dims(), DIMS);
        let docs: Vec<String> = vec![
            "format a monetary amount into a currency string".into(),
            "render the gpu shader pipeline for the scene".into(),
        ];
        let vecs = emb.embed(&docs).expect("embed");
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), DIMS);

        let store = HnswStore::new(emb.id(), 16);
        store.upsert(&["p\u{0}currency.ts".into(), "p\u{0}shader.ts".into()], &vecs).unwrap();
        // a SYNONYM query ("money formatting") — no lexical overlap with "currency" —
        // must still rank the currency doc first. This is the P5/P0 lexical gap closed.
        let q = emb.embed(&["money formatting helper".to_string()]).unwrap();
        let hits = store.query(&q[0], 2).unwrap();
        assert_eq!(hits[0].0, "p\u{0}currency.ts", "semantic recall: synonym finds currency doc: {hits:?}");
    }
}
