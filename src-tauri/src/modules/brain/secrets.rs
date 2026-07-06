//! Secrets & sensitive-data gate (CONCEPT §7.1, BUILD-PROMPT §7/§13.9) — a HARD
//! safety gate. Nothing secret is ever indexed, embedded, or injected.
//!
//! Two barriers, both applied before any content is tokenized or stored. First, a
//! file denylist: whole files matching known credential patterns are skipped
//! entirely (never read into the index). Second, content redaction: per line,
//! four detectors replace the secret span with `REDACTED` — (a) known provider
//! token prefixes (sk-, ghp_, AKIA, JWT, PEM); (b) secret-named assignments
//! (`password = "..."`, `api_key: ...`), whose whole value is redacted regardless
//! of shape (catches short/low-entropy and punctuation-split secrets); (c)
//! high-entropy mixed-alphanumeric tokens (>=16 chars), excluding git-SHA/hex,
//! UUIDs, and path/URL/version shapes so legitimate searchable content survives;
//! and (d) PEM block bodies: after any line CONTAINING a `-----BEGIN ...-----`
//! marker (bare, or sharing the line with an assignment/quote — inline PEMs in
//! code are nearly always string literals), every body line — pure
//! base64-alphabet, or a QUOTED concat fragment (`"...\n" +` in Java/C#/JS,
//! adjacent C literals, Python implicit concat) — is redacted WHOLE until a
//! line containing `-----END` (itself re-checked for a same-line `-----BEGIN`
//! of a NEXT block, the concatenated-bundle shape) — `/`- and `=`-split
//! key-body shards would otherwise fragment below (c)'s length floor (the leak
//! the 2026-07-05 index-layer probe reproduced; its reviews confirmed the
//! assignment-line, quoted-concat, and bundle variants).
//!
//! `.gitignore`/`.kodenignore` are honored upstream by the `ignore` walker; this
//! is the hardcoded base denylist that holds even for un-ignored files. Policy is
//! conservative-by-design ("if uncertain, treat as secret"). Known, documented
//! residual gaps (the honesty rule, BUILD-PROMPT §13.30): a bare in-code secret
//! that is pure-hex, or split by `/` outside an open PEM block — since multi-line
//! literals now open the block even when the marker shares the assignment line,
//! that means a truly single-line `\n`-escaped PEM literal (BEGIN and END on one
//! physical line never open the block) — and is NOT assigned to a secret-named
//! key, may survive content redaction — the file denylist, `.gitignore`, and
//! (future) the visible "excluded N as secret-like" override are the backstops.
//!
//! ponytail: regex-free (no new dep) — char-class scanning is enough here.

/// Known secret/credential token prefixes (CONCEPT [DP-25]), matched as a literal
/// prefix on a candidate token.
const SECRET_PREFIXES: &[&str] = &[
    "sk-", "rk-", "sk_live_", "sk_test_", "pk_live_", "rk_live_",
    "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_",
    "glpat-",
    "xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-",
    "AKIA", "ASIA",
    "AIza", "ya29.",
    "eyJ",          // JWT / base64 `{"`
    "-----BEGIN",   // PEM block marker
    "SG.",          // SendGrid
    "shpat_", "shpss_", "shpca_", "shppa_", // Shopify
    // NOTE: Azure `AccountKey=...` is handled by detector (b) via SECRET_KEY_WORDS
    // ("accountkey") — a prefix here would be dead code ('=' is not a candidate char).
    "dop_v1_",      // DigitalOcean
    "npm_",         // npm automation token
];

/// Key-name fragments (after lowercasing + stripping `_`/`-`) that mark an
/// assignment value as a secret.
const SECRET_KEY_WORDS: &[&str] = &[
    "password", "passwd", "passphrase", "pwd",
    "secret", "apikey", "accesskey", "secretkey", "privatekey", "sessionkey",
    "accountkey", // Azure storage `AccountKey=...` (connection-string segment)
    "credential", "token", "bearer", "authtoken", "oauth",
    "connectionstring", "databaseurl",
];

/// Lower-cased basename patterns that denylist a whole file.
fn is_denylisted_basename(name_lower: &str) -> bool {
    // exact / prefix matches
    if name_lower == ".env"
        || name_lower.starts_with(".env.")
        || name_lower == ".npmrc"
        || name_lower == ".pypirc"
        || name_lower == ".netrc"
        || name_lower == ".pgpass"
        || name_lower == ".git-credentials"
        || name_lower == ".htpasswd"
        || name_lower == "credentials"
        || name_lower.starts_with("credentials.")
        || name_lower == "kubeconfig"
        || name_lower.starts_with("id_rsa")
        || name_lower.starts_with("id_dsa")
        || name_lower.starts_with("id_ecdsa")
        || name_lower.starts_with("id_ed25519")
    {
        return true;
    }
    // extension matches
    const DENY_EXT: &[&str] = &[
        ".pem", ".key", ".pfx", ".p12", ".p8", ".kdbx", ".tfstate", ".jks",
        ".keystore", ".asc", ".ppk", ".ovpn",
    ];
    if DENY_EXT.iter().any(|e| name_lower.ends_with(e)) {
        return true;
    }
    // composite name patterns
    if name_lower.ends_with(".json")
        && (name_lower.contains("service-account")
            || name_lower.contains("serviceaccount")
            || name_lower.contains("-credentials")
            || name_lower.contains("gcp-key"))
    {
        return true;
    }
    if name_lower.ends_with(".tfstate.backup") {
        return true;
    }
    false
}

/// True if this path must never be indexed/embedded/injected.
pub fn is_denylisted_path(path: &str) -> bool {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    is_denylisted_basename(&base)
}

fn shannon_entropy(s: &str) -> f64 {
    use std::collections::HashMap;
    let len = s.chars().count() as f64;
    if len == 0.0 {
        return 0.0;
    }
    let mut counts: HashMap<char, u32> = HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0) += 1;
    }
    let mut e = 0.0;
    for &n in counts.values() {
        let p = n as f64 / len;
        e -= p * p.log2();
    }
    e
}

/// Chars that belong to a contiguous "secret body" run. `/` is intentionally
/// excluded so file paths are not treated as one token (it would otherwise be
/// impossible to distinguish a path from a base64 blob).
fn is_candidate_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

/// Detector (d) entry: the line CONTAINS a full `-----BEGIN <LABEL>-----`
/// marker. Matching bare marker lines only (the pre-review behavior) missed the
/// most common real-world inline-PEM shape — a multi-line raw-string/template
/// literal whose marker shares the line with the assignment (`` const k =
/// `-----BEGIN ...` ``). Conservative-by-design: a quoted marker in parser code
/// also opens the block; the accepted cost is bounded to whole-line redaction of
/// FULL base64-alphabet lines until a `-----END` (normal code keeps flowing
/// through — see the quoted-marker test). A `-----END` after the BEGIN on the
/// SAME line (single-line `\n`-escaped literal — the documented residual gap)
/// must NOT open the block, else stale state would blank unrelated later lines.
fn opens_pem_block(line: &str) -> bool {
    match line.find("-----BEGIN") {
        Some(pos) => {
            let rest = &line[pos + "-----BEGIN".len()..];
            rest.contains("-----") && !rest.contains("-----END")
        }
        None => false,
    }
}

/// Detector (d) body: inside a PEM block, a line made solely of base64-alphabet
/// chars is key material. Redacted WHOLE — its `/`-split shards are each below
/// (c)'s 16-char floor, which is exactly how the old gap leaked.
fn is_pem_body_line(trimmed: &str) -> bool {
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
}

/// Detector (d) body, quoted-concat variant: string-CONCATENATION inline PEMs
/// (mandatory in Java/C/C++, common in C#/Python/older JS) wrap each body line
/// in quote + `\n` escape + concat punctuation — none of which are base64
/// chars — so `is_pem_body_line` misses them and the `/`-split shards fall
/// below (c)'s 16-char floor (the 2026-07-05 fix's review reproduced the
/// leak). Inside an open block, accept a line that is exactly ONE quoted
/// literal whose interior is base64-alphabet (plus `\` escapes), followed only
/// by concat/terminator punctuation (`+` `,` `;` `)`, line-continuation `\`).
/// Assignment/code lines don't START with a quote, so normal code inside an
/// over-opened block keeps flowing through (see the quoted-marker test).
fn is_quoted_pem_body_line(trimmed: &str) -> bool {
    let Some(quote @ ('"' | '\'')) = trimmed.chars().next() else {
        return false;
    };
    let rest = &trimmed[1..];
    let Some(close) = rest.rfind(quote) else {
        return false;
    };
    let (interior, suffix) = (&rest[..close], &rest[close + 1..]);
    interior.chars().any(|c| c.is_ascii_alphanumeric())
        && interior
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '\\'))
        && suffix
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '+' | ',' | ';' | ')' | '\\'))
}

fn is_uuid_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, &c) in b.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_hexdigit(),
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Detect a high-entropy, key-shaped token (detector c). FP-aware: excludes
/// git-SHA/hex, UUIDs, and path/URL/version shapes.
fn should_redact(candidate: &str) -> bool {
    if SECRET_PREFIXES.iter().any(|p| candidate.starts_with(p)) {
        return true;
    }
    let n = candidate.len();
    if n < 16 {
        return false; // too short → identifier false-positive risk
    }
    if candidate.matches('.').count() >= 2 {
        return false; // version / domain / dotted path
    }
    let has_alpha = candidate.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = candidate.chars().any(|c| c.is_ascii_digit());
    if !(has_alpha && has_digit) {
        return false; // protects plain identifiers and pure-numeric ids
    }
    if candidate.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return false; // git SHA / hash / hex id (a must-not-redact control)
    }
    if is_uuid_shaped(candidate) {
        return false;
    }
    shannon_entropy(candidate) >= 3.0
}

/// Trailing identifier in `prefix` (skipping trailing spaces/quotes) — the "key"
/// of a `key = value` assignment.
fn trailing_ident(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches([' ', '\t', '"', '\'']);
    let rev: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    rev.chars().rev().collect()
}

fn is_secret_keyword(key: &str) -> bool {
    let k: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    SECRET_KEY_WORDS.iter().any(|w| k.contains(w))
}

/// Detector (b): redact the whole value of a secret-named `key = value` /
/// `key: value` assignment, regardless of the value's shape.
fn redact_keyed_values(line: &str) -> (String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut count = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if (c == '=' || c == ':') && is_secret_keyword(&trailing_ident(&out)) {
            out.push(c);
            i += 1;
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                out.push(chars[i]);
                i += 1;
            }
            let quote = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                out.push(q);
                i += 1;
                Some(q)
            } else {
                None
            };
            let start = i;
            while i < chars.len() {
                let v = chars[i];
                match quote {
                    Some(q) if v == q => break,
                    Some(_) => i += 1,
                    None => {
                        if v.is_whitespace() || v == ';' || v == ',' {
                            break;
                        }
                        i += 1;
                    }
                }
            }
            let value: String = chars[start..i].iter().collect();
            if value.chars().filter(|c| !c.is_whitespace()).count() >= 4 {
                out.push_str("REDACTED");
                count += 1;
            } else {
                out.push_str(&value);
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, count)
}

/// Detectors (a) + (c): scan candidate runs for prefixes / high-entropy tokens.
fn redact_candidates(s: &str) -> (String, usize) {
    let mut out = String::with_capacity(s.len());
    let mut buf = String::new();
    let mut count = 0usize;
    let flush = |buf: &mut String, out: &mut String, count: &mut usize| {
        if !buf.is_empty() {
            if should_redact(buf) {
                out.push_str("REDACTED");
                *count += 1;
            } else {
                out.push_str(buf);
            }
            buf.clear();
        }
    };
    for c in s.chars() {
        if is_candidate_char(c) {
            buf.push(c);
        } else {
            flush(&mut buf, &mut out, &mut count);
            out.push(c);
        }
    }
    flush(&mut buf, &mut out, &mut count);
    (out, count)
}

/// Redact secret-shaped content. Returns `(redacted, count)`. Runs before
/// tokenization so secrets never reach the FTS index, AST graph, memory, or gist.
/// Deterministic (a pure function of `content`), so identical bytes still map to
/// identical redacted text and the gist byte-identity gate ([DP-12]) is unmoved.
pub fn redact(content: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut count = 0usize;
    // Detector (d) block state. On an unclosed BEGIN it persists to EOF, but only
    // full base64-alphabet lines are affected — normal prose/code after a stray
    // marker keeps flowing through the per-line detectors untouched.
    let mut in_pem = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if in_pem {
            // `contains`, not `starts_with`: the END of an inline literal often
            // carries trailing code (`-----END X-----`;`) or leading quotes.
            if trimmed.contains("-----END") {
                // Re-evaluate entry after exit: a concatenated bundle missing the
                // interior newline CLOSES one block and OPENS the next on the same
                // physical line (`-----END X----------BEGIN Y-----`); gating entry
                // behind `else` leaked the second block's body. The marker line
                // itself still falls through to the normal passes.
                in_pem = opens_pem_block(trimmed);
            } else if is_pem_body_line(trimmed) || is_quoted_pem_body_line(trimmed) {
                // Whole-line redaction, preserving surrounding whitespace/newline
                // (trim boundaries are char boundaries, so slicing is safe).
                out.push_str(&line[..line.len() - line.trim_start().len()]);
                out.push_str("REDACTED");
                out.push_str(&line[line.trim_end().len()..]);
                count += 1;
                continue;
            }
            // Non-body lines (e.g. `Proc-Type:` headers in encrypted PEM) fall
            // through to the normal passes; the block stays open until `-----END`.
        } else if opens_pem_block(trimmed) {
            in_pem = true; // the marker token itself is redacted by detector (a)
        }
        let (keyed, kn) = redact_keyed_values(line);
        let (scanned, cn) = redact_candidates(&keyed);
        count += kn + cn;
        out.push_str(&scanned);
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denylists_credential_files() {
        for p in [
            "project/.env",
            "project/.env.production",
            "C:\\repo\\config\\server.pem",
            "deploy/gcp-service-account.json",
            "/home/u/.ssh/id_rsa",
            "/home/u/.netrc",
            "app/.pgpass",
            "x/.git-credentials",
            "key.p8",
        ] {
            assert!(is_denylisted_path(p), "should denylist {p}");
        }
        for p in ["src/main.rs", "README.md", "env.ts", "config/app.toml"] {
            assert!(!is_denylisted_path(p), "should NOT denylist {p}");
        }
    }

    #[test]
    fn redacts_known_prefixes() {
        let (r, n) = redact("const k = \"sk-abc123DEF456ghi789jkl012\";");
        assert!(!r.contains("sk-abc123"), "leaked: {r}");
        assert!(r.contains("REDACTED") && n >= 1);
        let (r2, _) = redact("token: ghp_AAAA1111BBBB2222CCCC3333");
        assert!(r2.contains("REDACTED") && !r2.contains("ghp_AAAA"));
    }

    #[test]
    fn redacts_secret_named_assignments_any_shape() {
        // short, low-entropy, and punctuation-split secrets via the key-name path
        for line in [
            "password = \"hunter2pass\"",
            "let api_key = \"AKIAIOSFODNN7EXAMPLE\";",
            "DB_PASSWORD: Tr0ub4dor!3Kx9Lm2Qp",
            "client_secret='short99'",
        ] {
            let (r, n) = redact(line);
            assert!(n >= 1, "expected redaction in: {line} -> {r}");
            assert!(r.contains("REDACTED"), "no REDACTED in: {r}");
        }
        // whole value gone, including the punctuation-split tail
        let (r, _) = redact("DB_PASSWORD: Tr0ub4dor!3Kx9Lm2Qp");
        assert!(!r.contains("3Kx9Lm2Qp"), "split tail leaked: {r}");
    }

    /// ADR-010 cluster 5: the old `AccountKey=` SECRET_PREFIXES entry was
    /// unreachable ('=' is not a candidate char) — Azure storage keys must be
    /// redacted via the keyed-assignment detector instead.
    #[test]
    fn redacts_azure_account_key_connection_string() {
        let cs = "DefaultEndpointsProtocol=https;AccountName=mystore;AccountKey=wJalrXUtnFEMI+K7MDENGbPxRfiCY==;EndpointSuffix=core.windows.net";
        let (r, n) = redact(cs);
        assert!(n >= 1, "expected redaction in: {r}");
        assert!(!r.contains("wJalrXUtnFEMI"), "Azure key leaked: {r}");
        assert!(r.contains("AccountKey=REDACTED"), "whole value redacted: {r}");
        assert!(r.contains("AccountName=mystore"), "non-secret segment preserved: {r}");
    }

    /// Index-layer probe 2026-07-05: the old documented "split by `/`" gap. A
    /// realistic PEM body line fragments at every `/` into sub-16-char shards
    /// that duck detector (c)'s length floor. Detector (d) now redacts every
    /// base64-alphabet line inside a BEGIN/END block whole.
    #[test]
    fn redacts_slash_fragmented_pem_block_body() {
        let src = "fn material() {}\n\
                   -----BEGIN RSA PRIVATE KEY-----\n\
                   WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                   MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8c\n\
                   -----END RSA PRIVATE KEY-----\n\
                   fn after() {}\n";
        let (r, n) = redact(src);
        assert!(n >= 2, "expected both body lines redacted: {r}");
        for shard in [
            "WqZx83Ky", "VnPb27Jm", "HcQf94Dw", "LqNs61Bu", "KvYw52Ez", "PjLm73Nq", "XrWt84Uz",
        ] {
            assert!(!r.contains(shard), "PEM shard leaked: {shard} in {r}");
        }
        assert!(!r.contains("MIIEvQ"), "clean body line leaked: {r}");
        // Redaction stays surgical: code around the block survives, and the
        // block state exits at -----END (the trailing line is untouched).
        assert!(r.contains("fn material() {}"), "{r}");
        assert!(r.contains("fn after() {}"), "{r}");
    }

    /// Encrypted-PEM headers (`Proc-Type:`/`DEK-Info:`) between BEGIN and the
    /// body must not close the block — the body after them is still redacted.
    #[test]
    fn pem_block_survives_encryption_headers() {
        let src = "-----BEGIN RSA PRIVATE KEY-----\n\
                   Proc-Type: 4,ENCRYPTED\n\
                   DEK-Info: AES-128-CBC,ABCD\n\
                   \n\
                   WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                   -----END RSA PRIVATE KEY-----\n";
        let (r, _) = redact(src);
        assert!(!r.contains("WqZx83Ky"), "body after headers leaked: {r}");
        assert!(r.contains("Proc-Type"), "header line wrongly blanked: {r}");
    }

    /// A QUOTED marker now opens the block too (conservative-by-design — inline
    /// PEMs live inside string literals, so quote-stripping the entry check is
    /// exactly what leaked; see the assignment-line test). The accepted cost is
    /// bounded: while the block is open only FULL base64-alphabet lines are
    /// blanked, so normal code keeps flowing through, and a quoted `-----END`
    /// closes the block again.
    #[test]
    fn quoted_pem_marker_over_redacts_only_base64_lines() {
        let src = "const HEADER = \"-----BEGIN CERTIFICATE-----\";\n\
                   let route = \"api/v0cfg/get\";\n\
                   const FOOTER = \"-----END CERTIFICATE-----\";\n\
                   ok\n";
        let (r, _) = redact(src);
        assert!(r.contains("api/v0cfg/get"), "code inside open block lost: {r}");
        assert!(r.contains("ok\n"), "all-alnum line after quoted END lost: {r}");
    }

    /// Review of the 2026-07-05 fix: the most common real-world inline-PEM shape
    /// puts the BEGIN marker on the ASSIGNMENT line of a multi-line raw-string /
    /// template literal (JS/TS backtick, Go raw string). The block must open
    /// anyway so the `/`-fragmented body is redacted whole — detector (b) only
    /// covers the assignment line itself, and each `/`-split shard is under
    /// (c)'s 16-char floor. The Go case uses a non-secret-named variable, so it
    /// proves detector (d) alone protects the body.
    #[test]
    fn pem_block_opens_when_marker_shares_the_assignment_line() {
        let js = "const PRIVATE_KEY = `-----BEGIN RSA PRIVATE KEY-----\n\
                  WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                  -----END RSA PRIVATE KEY-----`;\n\
                  export function afterward() {}\n";
        let go = "var pemData = `-----BEGIN RSA PRIVATE KEY-----\n\
                  WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                  -----END RSA PRIVATE KEY-----`\n\
                  func afterward() {}\n";
        for src in [js, go] {
            let (r, n) = redact(src);
            assert!(n >= 1, "expected body redaction: {r}");
            for shard in [
                "WqZx83Ky", "VnPb27Jm", "HcQf94Dw", "LqNs61Bu", "KvYw52Ez", "PjLm73Nq",
                "XrWt84Uz",
            ] {
                assert!(!r.contains(shard), "PEM shard leaked: {shard} in {r}");
            }
            // the inline -----END (with trailing `;` / backtick) closes the
            // block: code after the literal survives
            assert!(r.contains("afterward"), "code after the literal lost: {r}");
        }
    }

    /// Review round 2 of the 2026-07-05 fix: the string-CONCATENATION inline-PEM
    /// encoding (mandatory in Java/C/C++, common in C#/Python/older JS) wraps
    /// every body line in quotes + `\n` escape + concat punctuation — none of
    /// which are base64-alphabet chars — so the pure-base64 body predicate
    /// missed them and each `/`-split shard fell below (c)'s 16-char floor.
    /// Inside an open block a quoted concat literal is a body line too. The
    /// assignment keys are deliberately NOT secret-named, so detector (d) alone
    /// must protect the body.
    #[test]
    fn pem_block_redacts_quoted_concat_body_lines() {
        // Java `+` concat (also C# / older JS string building).
        let java = "String pemMaterial = \"-----BEGIN RSA PRIVATE KEY-----\\n\" +\n\
                    \"WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\\n\" +\n\
                    \"-----END RSA PRIVATE KEY-----\\n\";\n\
                    void afterward() {}\n";
        // C adjacent string literals (no operator between fragments).
        let c = "const char *pem =\n\
                 \"-----BEGIN RSA PRIVATE KEY-----\\n\"\n\
                 \"WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\\n\"\n\
                 \"-----END RSA PRIVATE KEY-----\\n\";\n\
                 void afterward(void) {}\n";
        // Python parenthesized implicit concat.
        let py = "PEM_MATERIAL = (\n\
                  \"-----BEGIN RSA PRIVATE KEY-----\\n\"\n\
                  \"WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\\n\"\n\
                  \"-----END RSA PRIVATE KEY-----\\n\"\n\
                  )\n\
                  def afterward(): pass\n";
        for src in [java, c, py] {
            let (r, n) = redact(src);
            assert!(n >= 1, "expected quoted body redaction: {r}");
            for shard in [
                "WqZx83Ky", "VnPb27Jm", "HcQf94Dw", "LqNs61Bu", "KvYw52Ez", "PjLm73Nq",
                "XrWt84Uz",
            ] {
                assert!(!r.contains(shard), "PEM shard leaked: {shard} in {r}");
            }
            // the quoted -----END fragment closes the block: code after survives
            assert!(r.contains("afterward"), "code after the literal lost: {r}");
        }
    }

    /// Review round 2 of the 2026-07-05 fix: a concatenated bundle missing the
    /// interior newline CLOSES one block and OPENS the next on the SAME physical
    /// line (`cat a.pem b.pem` output pasted into a code file or heredoc). Entry
    /// must be re-evaluated after exit — gating it behind `else` left the second
    /// block closed and leaked its `/`-split body shards.
    #[test]
    fn end_and_begin_on_one_line_reopens_the_block() {
        let src = "-----BEGIN CERTIFICATE-----\n\
                   MIIBcertBody47AaZzQqRrTt\n\
                   -----END CERTIFICATE----------BEGIN RSA PRIVATE KEY-----\n\
                   WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                   -----END RSA PRIVATE KEY-----\n\
                   fn after() {}\n";
        let (r, _) = redact(src);
        assert!(!r.contains("MIIBcert"), "first-block body leaked: {r}");
        for shard in [
            "WqZx83Ky", "VnPb27Jm", "HcQf94Dw", "LqNs61Bu", "KvYw52Ez", "PjLm73Nq", "XrWt84Uz",
        ] {
            assert!(!r.contains(shard), "second-block shard leaked: {shard} in {r}");
        }
        // the second -----END still closes the block: trailing code survives
        assert!(r.contains("fn after() {}"), "{r}");
    }

    /// BEGIN and END on the SAME physical line (the single-line `\n`-escaped
    /// literal — still the documented residual gap for its own content) must NOT
    /// open the block: stale state would blank unrelated all-alnum lines to EOF.
    #[test]
    fn same_line_begin_end_does_not_poison_block_state() {
        let src = "const K = \"-----BEGIN X-----\\nzz\\n-----END X-----\";\n\
                   done\n";
        let (r, _) = redact(src);
        assert!(r.contains("done"), "block left open past same-line BEGIN/END: {r}");
    }

    #[test]
    fn redacts_high_entropy() {
        let (r, n) = redact("aws = wJalrXUtnFEMIxK7MDENGbPxRfiCYEXAMPLEKEY1");
        assert!(n >= 1 && r.contains("REDACTED"), "expected high-entropy redaction: {r}");
    }

    #[test]
    fn preserves_normal_code_and_controls() {
        // normal code must not be touched
        let src = "fn validateUserCredentials(payload: Request) -> Result<User, Error> { Ok(user) }";
        let (r, n) = redact(src);
        assert_eq!(n, 0, "false positive: {r}");
        assert_eq!(r, src);
        // must-not-redact controls: git SHA, UUID, path, dotted version
        for ctrl in [
            "commit a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0 fixed it",
            "id = 550e8400-e29b-41d4-a716-446655440000",
            "path src/modules/brain/secrets.rs here",
            "version 1.20.34 released",
            "let maxRetriesAllowed = 5;",
        ] {
            let (r, n) = redact(ctrl);
            assert_eq!(n, 0, "control wrongly redacted: {ctrl} -> {r}");
        }
    }
}
