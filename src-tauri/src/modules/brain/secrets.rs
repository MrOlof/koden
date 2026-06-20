//! Secrets & sensitive-data gate (CONCEPT §7.1, BUILD-PROMPT §7/§13.9) — a HARD
//! safety gate. Nothing secret is ever indexed, embedded, or injected.
//!
//! Two barriers, both applied before any content is tokenized or stored. First, a
//! file denylist: whole files matching known credential patterns are skipped
//! entirely (never read into the index). Second, content redaction: per line,
//! three detectors replace the secret span with `REDACTED` — (a) known provider
//! token prefixes (sk-, ghp_, AKIA, JWT, PEM); (b) secret-named assignments
//! (`password = "..."`, `api_key: ...`), whose whole value is redacted regardless
//! of shape (catches short/low-entropy and punctuation-split secrets); and (c)
//! high-entropy mixed-alphanumeric tokens (>=16 chars), excluding git-SHA/hex,
//! UUIDs, and path/URL/version shapes so legitimate searchable content survives.
//!
//! `.gitignore`/`.kodenignore` are honored upstream by the `ignore` walker; this
//! is the hardcoded base denylist that holds even for un-ignored files. Policy is
//! conservative-by-design ("if uncertain, treat as secret"). Known, documented
//! residual gaps (the honesty rule, BUILD-PROMPT §13.30): a bare in-code secret
//! that is pure-hex, or split by `/`, and is NOT assigned to a secret-named key,
//! may survive content redaction — the file denylist, `.gitignore`, and (future)
//! the visible "excluded N as secret-like" override are the backstops for those.
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
    "AccountKey=",  // Azure storage connection string
    "dop_v1_",      // DigitalOcean
    "npm_",         // npm automation token
];

/// Key-name fragments (after lowercasing + stripping `_`/`-`) that mark an
/// assignment value as a secret.
const SECRET_KEY_WORDS: &[&str] = &[
    "password", "passwd", "passphrase", "pwd",
    "secret", "apikey", "accesskey", "secretkey", "privatekey", "sessionkey",
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
pub fn redact(content: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut count = 0usize;
    for line in content.split_inclusive('\n') {
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
