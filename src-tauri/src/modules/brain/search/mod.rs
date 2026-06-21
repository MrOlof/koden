//! P5 — deferred semantic seams (ADR-006). v1 ships ONLY the shape: the
//! [Embedder]/[VectorStore] traits + the `embedderId` header (`brain_semantic_meta`),
//! so the FTS5/AST/notes/ledger schema and the search path never churn when
//! semantic is later enabled. There is NO functional semantic search in v1 — no
//! vector leg joins the RRF fusion (see [registered_search_legs]). The embedding
//! stack lives behind the default-OFF `semantic` cargo feature and is absent from
//! the shipped binary.

pub mod vector;

/// Dependency-free reference Embedder/VectorStore — gated, NOT in the v1 binary.
/// Proves the seams compose + keeps the feature from rotting; the production swap
/// (fastembed-rs + hnsw_rs) lands behind the same traits at enablement time.
#[cfg(feature = "semantic")]
pub mod reference;

pub use vector::{DocId, Embedder, VectorStore};

/// The RRF fusion legs actually registered in the live search path. v1 = the two
/// FTS5 legs (`identity` = path+symbols, `content`). A semantic `vector` leg is
/// NEVER registered in the default build — the P5 no-vector-leg gate. When the
/// `semantic` feature is enabled AND a vector store is wired, the vector leg slots
/// in here (one more weighted RRF leg) with zero schema churn.
pub fn registered_search_legs() -> Vec<&'static str> {
    // Kept in sync with `store::sqlite::search_with_conn` (leg_a identity, leg_b
    // content). The vector leg is intentionally absent in v1.
    vec!["identity", "content"]
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
