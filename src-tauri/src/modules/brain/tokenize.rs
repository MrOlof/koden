//! Lexical tokenizer — a faithful Rust port of Conductr's
//! `src/lib/search/lexical.ts` — `tokenize`/`pushToken`/`stemLight`/`splitCamel`
//! plus its stoplist. Applied identically to code and notes so identifier
//! retrieval holds. This is [DP-1] in CONCEPT §4.1a.
//!
//! Behaviour mirrored exactly:
//! - lowercase + split on non-alphanumerics (`[A-Za-z0-9]+`)
//! - camelCase / PascalCase / digit-boundary split, emitting BOTH the whole
//!   token AND the parts (`writeAiFiles` → `writeaifiles`,`write`,`ai`,`files`)
//! - additive light stemming, emitting BOTH forms (`validation` → `validate`)
//! - drop tokens shorter than 2 chars and the stoplist
//!
//! Tokens are pre-computed and stored as a space-joined stream into FTS5 (the
//! "pre-tokenization pass" of CONCEPT [DP-3]) because the synthetic stem/part
//! tokens are not substrings of the source and so cannot ride an FTS5 external
//! tokenizer.

/// Stoplist (verbatim from `lexical.ts:15-53`; matches Conductr's indexing set).
/// Kept short so we never drop identifiers. (A larger superset exists in
/// Conductr's query-expansion layer; the indexing tokenizer uses this set.)
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "is", "it", "for", "on",
    "with", "that", "this", "as", "be", "by", "at", "from", "are", "was",
    "were", "you", "your", "we", "our", "if", "so", "but", "not", "no", "do",
    "does", "can", "will", "into",
];

fn is_stopword(t: &str) -> bool {
    STOPWORDS.contains(&t)
}

/// Tokenize text for lexical indexing. See module docs for the contract.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in split_words(text) {
        let lower = word.to_ascii_lowercase();
        push_token(&mut out, &lower);
        for part in split_camel(&word) {
            let lower_part = part.to_ascii_lowercase();
            if lower_part != lower {
                push_token(&mut out, &lower_part);
            }
        }
    }
    out
}

/// `text.match(/[A-Za-z0-9]+/g)` — contiguous ASCII alphanumeric runs.
fn split_words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn push_token(out: &mut Vec<String>, token: &str) {
    // Tokens here are pure ASCII (from `split_words` + `to_ascii_lowercase`), so
    // byte length == char count == the JS UTF-16 `.length`.
    if token.len() < 2 {
        return;
    }
    if is_stopword(token) {
        return;
    }
    out.push(token.to_string());
    // Light morphological expansion: emit a stem alongside the original so
    // "validated" (query) matches "validate" (doc). Additive — both forms kept.
    let stem = stem_light(token);
    if stem != token && stem.len() >= 3 {
        out.push(stem);
    }
}

/// Minimal suffix-stripping for English verb/noun morphology — verbatim port of
/// `stemLight` (`lexical.ts:99-129`). Conservative; rule order is load-bearing.
fn stem_light(t: &str) -> String {
    let n = t.len();
    // "validation" → "validate", "annotation" → "annotate"
    if n > 7 && t.ends_with("ation") {
        return format!("{}ate", &t[..n - 5]);
    }
    // "validated" → "validate" (drop trailing "d")
    if n > 6 && t.ends_with("ated") {
        return t[..n - 1].to_string();
    }
    // "rejection" → "reject", "detection" → "detect"; consonant-preceded base ≥4.
    if n > 7 && t.ends_with("ion") && !t.ends_with("ation") {
        let base = &t[..n - 3];
        let last = base.as_bytes()[base.len() - 1] as char;
        if base.len() >= 4 && !"aeiou".contains(last) {
            return base.to_string();
        }
    }
    // "rejected" → "reject", "parsed" → "parse"
    if n > 4 && t.ends_with("ed") && !t.ends_with("eed") && !t.ends_with("ied") {
        return t[..n - 2].to_string();
    }
    // "applied" → "apply", "verified" → "verify"
    if n > 4 && t.ends_with("ied") {
        return format!("{}y", &t[..n - 3]);
    }
    t.to_string()
}

/// Split a camelCase / PascalCase / digit-bounded identifier into parts.
/// Equivalent to `splitCamel`'s four sequential `.replace()` boundary rules
/// (`lexical.ts:131-141`), implemented as a single boundary-insertion pass:
///   B1 `[a-z0-9]│[A-Z]`  B2 `[A-Z]│[A-Z][a-z]`  B3 `[A-Za-z]│[0-9]`  B4 `[0-9]│[A-Za-z]`
fn split_camel(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    for i in 1..n {
        let p = chars[i - 1];
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let b1 = (p.is_ascii_lowercase() || p.is_ascii_digit()) && c.is_ascii_uppercase();
        let b2 = p.is_ascii_uppercase()
            && c.is_ascii_uppercase()
            && next.is_some_and(|x| x.is_ascii_lowercase());
        let b3 = p.is_ascii_alphabetic() && c.is_ascii_digit();
        let b4 = p.is_ascii_digit() && c.is_ascii_alphabetic();
        if b1 || b2 || b3 || b4 {
            parts.push(chars[start..i].iter().collect());
            start = i;
        }
    }
    parts.push(chars[start..n].iter().collect());
    parts.into_iter().filter(|p: &String| !p.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_plus_parts_for_camel() {
        // CONCEPT example: writeAiFiles → writeaifiles, write, ai, files
        let t = tokenize("writeAiFiles");
        assert!(t.contains(&"writeaifiles".to_string()));
        assert!(t.contains(&"write".to_string()));
        assert!(t.contains(&"ai".to_string()));
        assert!(t.contains(&"files".to_string()));
    }

    #[test]
    fn acronym_then_word() {
        let t = tokenize("HTTPServer");
        assert!(t.contains(&"http".to_string()));
        assert!(t.contains(&"server".to_string()));
    }

    #[test]
    fn additive_stemming_both_forms() {
        let t = tokenize("validation");
        assert!(t.contains(&"validation".to_string()));
        assert!(t.contains(&"validate".to_string()));
        let t2 = tokenize("applied");
        assert!(t2.contains(&"applied".to_string()));
        assert!(t2.contains(&"apply".to_string()));
        let t3 = tokenize("rejected");
        assert!(t3.contains(&"reject".to_string()));
    }

    #[test]
    fn drops_short_and_stopwords() {
        let t = tokenize("a it the of x");
        assert!(t.is_empty(), "stopwords + single chars dropped, got {t:?}");
    }

    #[test]
    fn digit_boundaries() {
        let t = tokenize("parseURL2HTML");
        assert!(t.contains(&"parse".to_string()));
        assert!(t.contains(&"url".to_string()));
        assert!(t.contains(&"html".to_string()));
    }

    #[test]
    fn query_doc_symmetry() {
        // The same surface form in doc and query must produce overlapping tokens.
        let doc = tokenize("fn validateUserInput(payload)");
        let q = tokenize("validate input");
        assert!(q.iter().all(|qt| doc.contains(qt)), "doc={doc:?} q={q:?}");
    }
}
