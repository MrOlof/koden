//! P5 — deferred semantic seams (ADR-006). v1 ships ONLY the shape: the
//! [Embedder]/[VectorStore] traits + the `embedderId` header (`brain_semantic_meta`),
//! so the FTS5/AST/notes/ledger schema and the search path never churn when
//! semantic is later enabled. There is NO functional semantic search in v1 — no
//! vector leg joins the RRF fusion (see [registered_search_legs]). The embedding
//! stack lives behind the default-OFF `semantic` cargo feature and is absent from
//! the shipped binary.

pub mod vector;

/// Dependency-free reference Embedder/VectorStore — gated, NOT in the v1 binary.
/// Proves the seams compose + keeps the feature from rotting.
#[cfg(feature = "semantic")]
pub mod reference;

/// V2.5 — the REAL hnsw_rs-backed VectorStore (pure Rust, offline-testable).
#[cfg(feature = "semantic")]
pub mod hnsw_store;

/// V2.5 — the REAL fastembed-rs ONNX Embedder (needs network at build + runtime).
#[cfg(feature = "semantic-embed")]
pub mod fastembed_embedder;

pub use vector::{DocId, Embedder, VectorStore};

/// The RRF fusion legs the live search path actually fuses — the SAME
/// [`SEARCH_LEG_LABELS`](crate::modules::brain::store::SEARCH_LEG_LABELS) that
/// `store::sqlite::search_with_conn` builds, so this can't drift from reality. v1 =
/// the two FTS5 legs (`identity` = path+symbols, `content`); no semantic `vector`
/// leg is registered (the P5 no-vector-leg gate).
///
/// NOTE on the slot-in seam: enabling semantic later needs ZERO *schema* churn
/// (the only new object is the lazy `brain_vectors` table). It is NOT zero *code*
/// churn — there is no runtime leg registry; `weighted_rrf` is already N-leg, but
/// adding the vector leg is a localized edit to `search_with_conn` (build a third
/// `Leg`) plus appending its label to `SEARCH_LEG_LABELS`.
pub fn registered_search_legs() -> Vec<&'static str> {
    crate::modules::brain::store::SEARCH_LEG_LABELS.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P5 gate 1: the default build links no embedding/vector code. Compiled only
    /// in the default build (the `--features semantic` CI job deliberately turns the
    /// feature ON, so this assertion belongs to the default job).
    #[cfg(not(feature = "semantic"))]
    #[test]
    fn semantic_feature_absent_from_default_build() {
        let semantic_on = cfg!(feature = "semantic");
        assert!(!semantic_on, "the semantic feature must be OFF by default");
    }

    /// P5 gate: the live search path registers exactly the two FTS5 legs — no
    /// vector leg in v1.
    #[test]
    fn search_index_has_no_vector_leg_in_v1() {
        let legs = registered_search_legs();
        assert_eq!(legs, vec!["identity", "content"]);
        assert!(!legs.contains(&"vector"), "no semantic vector leg registered in v1");
    }
}
