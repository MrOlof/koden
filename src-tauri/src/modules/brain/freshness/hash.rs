//! blake3 per-file content hashing (CONCEPT [DP-13]): fast, non-crypto, the
//! primary freshness signal. Hex of the raw file bytes — any change flips it.

/// Hex blake3 of the given bytes.
pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_distinct() {
        assert_eq!(hash_bytes(b"hello"), hash_bytes(b"hello"));
        assert_ne!(hash_bytes(b"hello"), hash_bytes(b"hellp"));
    }
}
