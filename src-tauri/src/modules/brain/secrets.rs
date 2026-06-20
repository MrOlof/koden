//! Secrets & sensitive-data gate (CONCEPT §7.1, BUILD-PROMPT §7/§13.9) — a HARD
//! safety gate. Nothing secret is ever indexed, embedded, or injected.
//!
//! Two barriers, both applied **before any content is tokenized or stored**:
//!  1. **File denylist** — whole files matching known credential patterns are
//!     skipped entirely (never read into the index).
//!  2. **Content redaction** — within indexed files, secret-shaped tokens
//!     (known provider prefixes + high-entropy strings) are replaced with
//!     `REDACTED` before tokenization.
//!
//! `.gitignore`/`.kodenignore` are honored upstream by the `ignore` walker; this
//! is the hardcoded base denylist that holds even for un-ignored files. P0 scope
//! is conservative-by-design ("if uncertain, treat as secret"); the Auditor
//! hardens this against the planted-secrets fixture (BUILD-PROMPT §6.5).
//!
//! ponytail: regex-free (no new dep) — a char-class scanner is enough and
//! avoids pulling in `regex` for a handful of prefix/entropy checks.

/// Known secret/credential token prefixes (provider regexes from CONCEPT [DP-25],
/// matched as literal prefixes on a candidate token).
const SECRET_PREFIXES: &[&str] = &[
    "sk-",          // OpenAI / Stripe secret
    "rk-",          // Stripe restricted
    "pk_live_",     // Stripe live publishable (still sensitive in context)
    "sk_live_",     // Stripe live secret
    "ghp_", "gho_", "ghu_", "ghs_", "ghr_", // GitHub tokens
    "github_pat_",  // GitHub fine-grained PAT
    "glpat-",       // GitLab PAT
    "xoxb-", "xoxp-", "xoxa-", "xoxr-", // Slack
    "AKIA", "ASIA", // AWS access key id
    "AIza",         // Google API key
    "ya29.",        // Google OAuth token
    "eyJ",          // JWT / base64 `{"`
    "-----BEGIN",   // PEM block marker
    "AccountKey=",  // Azure storage
    "SG.",          // SendGrid
    "shpat_", "shpss_", // Shopify
];

/// Lower-cased basename patterns that denylist a whole file.
fn is_denylisted_basename(name_lower: &str) -> bool {
    // exact / prefix matches
    if name_lower == ".env"
        || name_lower.starts_with(".env.")
        || name_lower == ".npmrc"
        || name_lower == ".pypirc"
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
        ".pem", ".key", ".pfx", ".p12", ".kdbx", ".tfstate", ".jks", ".keystore",
        ".asc", ".ppk",
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

/// A token belongs to a candidate "secret body" run.
fn is_candidate_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+')
}

fn should_redact(candidate: &str) -> bool {
    if SECRET_PREFIXES.iter().any(|p| candidate.starts_with(p)) {
        return true;
    }
    // High-entropy heuristic: long, mixed letters+digits, dense entropy.
    if candidate.len() >= 20 {
        let has_letter = candidate.chars().any(|c| c.is_ascii_alphabetic());
        let has_digit = candidate.chars().any(|c| c.is_ascii_digit());
        if has_letter && has_digit && shannon_entropy(candidate) >= 3.5 {
            return true;
        }
    }
    false
}

/// Redact secret-shaped substrings from `content`. Returns `(redacted, count)`.
/// Runs before tokenization so secrets never reach the FTS index, the AST graph,
/// memory, or (downstream) a gist.
pub fn redact(content: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
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

    for c in content.chars() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denylists_credential_files() {
        assert!(is_denylisted_path("project/.env"));
        assert!(is_denylisted_path("project/.env.production"));
        assert!(is_denylisted_path("C:\\repo\\config\\server.pem"));
        assert!(is_denylisted_path("deploy/gcp-service-account.json"));
        assert!(is_denylisted_path("/home/u/.ssh/id_rsa"));
        assert!(!is_denylisted_path("src/main.rs"));
        assert!(!is_denylisted_path("README.md"));
        assert!(!is_denylisted_path("env.ts")); // not `.env`
    }

    #[test]
    fn redacts_known_prefixes() {
        let (r, n) = redact("const k = \"sk-abc123DEF456ghi789jkl012\";");
        assert!(!r.contains("sk-abc123"), "leaked: {r}");
        assert!(r.contains("REDACTED"));
        assert!(n >= 1);
        let (r2, _) = redact("token: ghp_AAAA1111BBBB2222CCCC3333");
        assert!(r2.contains("REDACTED") && !r2.contains("ghp_AAAA"));
    }

    #[test]
    fn redacts_high_entropy() {
        let (r, n) = redact("aws_secret = wJalrXUtnFEMI8K7MDENGbPxRfiCYEXAMPLEKEY1");
        assert!(n >= 1, "expected high-entropy redaction in {r}");
    }

    #[test]
    fn preserves_normal_code() {
        let src = "fn validateUserInput(payload: Request) -> Result<User, Error> { Ok(user) }";
        let (r, n) = redact(src);
        assert_eq!(n, 0, "false positive redaction: {r}");
        assert_eq!(r, src);
    }
}
