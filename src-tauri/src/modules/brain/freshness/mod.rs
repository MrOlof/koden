//! Freshness — blake3 per-file content hashing + gitignore-aware population.
//! The PRIMARY change signal for ALL projects (git and non-git uniformly),
//! collapsing Conductr's git/no-git branch (ADR-006, CONCEPT §4.3 [DP-13]).
//! The recursive `notify` watcher that drives incremental updates lands in P1.

pub mod hash;
pub mod walk;
pub mod watch;

/// Aggregate workspace fingerprint: blake3 over the **sorted** `(path, file-hash)`
/// list — order-independent, changes iff any file changes (a Merkle-style digest).
/// This is the basis of P3's cache-stable gist key. CONCEPT [DP-13].
pub fn aggregate_fingerprint(entries: &mut [(String, String)]) -> String {
    entries.sort();
    let mut hasher = blake3::Hasher::new();
    for (path, file_hash) in entries.iter() {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_is_order_independent() {
        let mut a = vec![
            ("a.rs".to_string(), "h1".to_string()),
            ("b.rs".to_string(), "h2".to_string()),
        ];
        let mut b = vec![
            ("b.rs".to_string(), "h2".to_string()),
            ("a.rs".to_string(), "h1".to_string()),
        ];
        assert_eq!(aggregate_fingerprint(&mut a), aggregate_fingerprint(&mut b));
    }

    #[test]
    fn aggregate_changes_on_any_file_change() {
        let mut a = vec![("a.rs".to_string(), "h1".to_string())];
        let mut b = vec![("a.rs".to_string(), "h2".to_string())];
        assert_ne!(aggregate_fingerprint(&mut a), aggregate_fingerprint(&mut b));
    }
}
