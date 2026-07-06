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
//! and (d) PEM blocks: each line's `-----BEGIN ...-----` / `-----END` markers
//! are folded LEFT-TO-RIGHT into a running block state, and every line touching
//! an open block is redacted WHOLE — no body-line classification. Between BEGIN
//! and END in real files there is nothing legitimate to preserve (base64 body,
//! quoted/concat fragments in any language's string syntax, encrypted-PEM
//! headers whose `DEK-Info` carries the IV), and per-shape body predicates kept
//! leaking variants (operator-first concat, PHP dot-concat — reviews of the
//! 2026-07-05 fix, whose index-layer probe first reproduced the `/`-split
//! key-body shards fragmenting below (c)'s length floor). Positional folding
//! handles the awkward marker orderings by construction: BEGIN+END on one line
//! ends closed, END-then-BEGIN (a concatenated bundle missing its interior
//! newline) ends open, and a complete single-line block followed by a fresh
//! BEGIN ends open. A stray unterminated BEGIN in prose/docs is bounded: the
//! block auto-closes after `PEM_BLOCK_LINE_CAP` consecutive block lines (the
//! run restarts whenever a line opens a block, so a concatenated bundle is
//! bounded per block, not cumulatively).
//!
//! `.gitignore`/`.kodenignore` are honored upstream by the `ignore` walker; this
//! is the hardcoded base denylist that holds even for un-ignored files. Policy is
//! conservative-by-design ("if uncertain, treat as secret"). Known, documented
//! residual gaps (the honesty rule, BUILD-PROMPT §13.30): a bare in-code secret
//! that is pure-hex, or split by `/` outside an open PEM block — a truly
//! single-line `\n`-escaped PEM literal (BEGIN and END on one physical line
//! fold back to the CLOSED state, so the line is never wholesale-blanked), or
//! body lines past `PEM_BLOCK_LINE_CAP` consecutive lines of ANY block,
//! TERMINATED or not (an armor longer than the cap auto-closes mid-body and
//! its tail leaks), or a block whose BEGIN marker atom is itself split across
//! physical concat lines (the same-line completeness check never opens it) —
//! and is NOT assigned to a secret-named key, may survive content redaction —
//! the file denylist,
//! `.gitignore`, and (future) the visible "excluded N as secret-like" override
//! are the backstops.
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

/// Detector (d) blast-radius ceiling: consecutive block lines redacted whole
/// before an open block is auto-closed. A stray marker in prose/docs would
/// otherwise blank the rest of the file. The run is PER BLOCK: it restarts
/// whenever a line opens a block, so a concatenated bundle is bounded per
/// block, never cumulatively.
// ponytail: 1024 is a deliberate ceiling — typical PEM bodies are ~50 lines
// (4096-bit RSA), and even the big genuine armors close under it (RSA-16384
// ~430 lines, a PGP key with a photo ID ~600+), while a stray BEGIN's
// false-positive cost stays bounded. A rarer, larger armor (a multi-MB PGP
// MESSAGE) auto-closes mid-body and its tail leaks — TERMINATED or not — a
// documented residual gap; the .asc/.pem/.key file denylist is the backstop
// for whole-file armors.
const PEM_BLOCK_LINE_CAP: usize = 1024;

/// Detector (d) markers: fold this line's `-----BEGIN ...-----` / `-----END`
/// markers LEFT-TO-RIGHT into the running block state, returning the state
/// AFTER the line. Positional folding handles the awkward orderings by
/// construction — no special cases: BEGIN+END on one line (single-line
/// `\n`-escaped literal, the documented residual gap) ends CLOSED so stale
/// state can't blank later lines; END-then-BEGIN (a concatenated bundle
/// missing its interior newline) ends OPEN; a complete single-line block
/// followed by a fresh BEGIN ends OPEN. A BEGIN only counts when the marker is
/// complete (`-----BEGIN <LABEL>-----` — trailing dashes somewhere after it);
/// a bare `-----BEGIN` fragment in prose opens nothing. The second return is
/// true when any marker on this line OPENED a block — the caller restarts the
/// `PEM_BLOCK_LINE_CAP` run on it, so the cap bounds each block, not the
/// concatenation (a same-line END-then-BEGIN junction keeps the final state
/// unchanged and would otherwise be invisible).
fn pem_state_after_line(line: &str, mut in_pem: bool) -> (bool, bool) {
    let mut rest = line;
    let mut opened = false;
    loop {
        // The earlier marker is folded first (they can never start at the same
        // byte); the loop re-finds the other one next pass.
        match (rest.find("-----BEGIN"), rest.find("-----END")) {
            (None, None) => return (in_pem, opened),
            (Some(b), Some(e)) if e < b => {
                in_pem = false;
                rest = &rest[e + "-----END".len()..];
            }
            (Some(b), _) => {
                let after = &rest[b + "-----BEGIN".len()..];
                if after.contains("-----") {
                    in_pem = true;
                    opened = true;
                }
                rest = after;
            }
            (None, Some(e)) => {
                in_pem = false;
                rest = &rest[e + "-----END".len()..];
            }
        }
    }
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
    // Detector (d) block state + the consecutive-block-line run that bounds an
    // unterminated BEGIN via PEM_BLOCK_LINE_CAP.
    let mut in_pem = false;
    let mut pem_line_run = 0usize;
    for line in content.split_inclusive('\n') {
        let was_in_pem = in_pem;
        let (now_in_pem, opened_block) = pem_state_after_line(line, in_pem);
        in_pem = now_in_pem;
        if was_in_pem || in_pem {
            // Detector (d): any line touching an open block — body, concat
            // fragments in any string syntax, encrypted-PEM headers, and the
            // marker lines themselves (they carry no secret, but blanking them
            // keeps the rule uniform) — is redacted WHOLE. Preserve the
            // surrounding whitespace/newline (trim boundaries are char
            // boundaries, so slicing is safe); whitespace-only lines carry
            // nothing to redact and pass through unchanged.
            let trimmed = line.trim();
            if trimmed.is_empty() {
                out.push_str(line);
            } else {
                out.push_str(&line[..line.len() - line.trim_start().len()]);
                out.push_str("REDACTED");
                out.push_str(&line[line.trim_end().len()..]);
                count += 1;
            }
            if opened_block {
                pem_line_run = 0; // fresh block: the cap run is per block, not cumulative
            }
            pem_line_run += 1;
            if in_pem && pem_line_run >= PEM_BLOCK_LINE_CAP {
                in_pem = false; // auto-close: a stray BEGIN, not a real body
            }
            if !in_pem {
                pem_line_run = 0; // block closed (END or cap): run over
            }
            continue;
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

    /// Encrypted-PEM headers (`Proc-Type:`/`DEK-Info:`) sit between BEGIN and
    /// the body; wholesale between-marker redaction blanks them along with the
    /// body (`DEK-Info` carries the IV — it IS key material), and the interior
    /// blank line must not close the block or grow the output.
    #[test]
    fn pem_block_redacts_encryption_headers_and_body() {
        let src = "-----BEGIN RSA PRIVATE KEY-----\n\
                   Proc-Type: 4,ENCRYPTED\n\
                   DEK-Info: AES-128-CBC,ABCD\n\
                   \n\
                   WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                   -----END RSA PRIVATE KEY-----\n\
                   fn after() {}\n";
        let (r, _) = redact(src);
        assert!(!r.contains("WqZx83Ky"), "body after headers leaked: {r}");
        assert!(!r.contains("DEK-Info"), "encrypted-PEM header leaked (carries the IV): {r}");
        assert!(r.contains("\n\n"), "interior blank line not preserved as-is: {r}");
        assert!(r.contains("fn after() {}"), "code after the block lost: {r}");
    }

    /// A QUOTED marker opens the block too (conservative-by-design — inline
    /// PEMs live inside string literals, so quote-stripping the entry check is
    /// exactly what leaked; see the assignment-line test). Under wholesale
    /// between-marker redaction the accepted over-redaction grew: EVERY line
    /// between a quoted BEGIN "header constant" and the matching END is
    /// blanked, code or not. The quoted `-----END` still closes the block
    /// (later code survives), and PEM_BLOCK_LINE_CAP bounds a header constant
    /// with no footer.
    #[test]
    fn quoted_pem_marker_pair_blanks_the_whole_interior() {
        let src = "const HEADER = \"-----BEGIN CERTIFICATE-----\";\n\
                   let route = \"api/v0cfg/get\";\n\
                   const FOOTER = \"-----END CERTIFICATE-----\";\n\
                   ok\n";
        let (r, _) = redact(src);
        assert!(!r.contains("api/v0cfg/get"), "interior of an open block must be blanked: {r}");
        assert!(r.contains("ok\n"), "line after quoted END lost: {r}");
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

    /// Review round 3 of the 2026-07-05 fix, confirmed defect: operator-FIRST
    /// concat puts the `+` (or PHP's `.`) where the old quoted-body predicate
    /// didn't allow it — a leading operator (the line no longer STARTS with a
    /// quote) or a trailing `.` (absent from the suffix allowlist) — so every
    /// `/`-split shard leaked. Wholesale between-marker redaction has no body
    /// predicate left to fool.
    #[test]
    fn pem_block_redacts_operator_first_and_dot_concat_body_lines() {
        // JS/TS operator-first `+` continuation style.
        let js = "const pem = \"-----BEGIN RSA PRIVATE KEY-----\\n\"\n\
                  + \"WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\\n\"\n\
                  + \"-----END RSA PRIVATE KEY-----\\n\";\n\
                  function afterward() {}\n";
        // PHP dot-concat (trailing `.` operator).
        let php = "$pem = \"-----BEGIN RSA PRIVATE KEY-----\\n\" .\n\
                   \"WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\\n\" .\n\
                   \"-----END RSA PRIVATE KEY-----\\n\";\n\
                   function afterward() {}\n";
        for src in [js, php] {
            let (r, n) = redact(src);
            assert!(n >= 1, "expected body redaction: {r}");
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

    /// Review round 3, confirmed defect: a complete single-line block followed
    /// by a FRESH `-----BEGIN` on the same physical line. The old entry check
    /// ("no `-----END` after the BEGIN") saw the FIRST block's END and refused
    /// to open, leaking the second block's `/`-split body. Left-to-right marker
    /// folding (BEGIN, END, BEGIN) ends the line in the OPEN state by
    /// construction.
    #[test]
    fn single_line_block_then_begin_opens_the_block() {
        let src = "const A = \"-----BEGIN X-----\\nzz\\n-----END X-----\"; const B = `-----BEGIN RSA PRIVATE KEY-----\n\
                   WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
                   -----END RSA PRIVATE KEY-----`;\n\
                   fn after() {}\n";
        let (r, _) = redact(src);
        for shard in [
            "WqZx83Ky", "VnPb27Jm", "HcQf94Dw", "LqNs61Bu", "KvYw52Ez", "PjLm73Nq", "XrWt84Uz",
        ] {
            assert!(!r.contains(shard), "second-block shard leaked: {shard} in {r}");
        }
        // the -----END still closes the block: trailing code survives
        assert!(r.contains("fn after() {}"), "{r}");
    }

    /// A stray unterminated `-----BEGIN ...-----` (prose/docs) must not blank
    /// the rest of the file: the block auto-closes after PEM_BLOCK_LINE_CAP
    /// consecutive block lines and everything past the cap flows through the
    /// normal detectors untouched.
    #[test]
    fn unterminated_begin_auto_closes_at_the_cap() {
        let mut src = String::from("-----BEGIN RSA PRIVATE KEY-----\n");
        for i in 0..PEM_BLOCK_LINE_CAP + 40 {
            src.push_str(&format!("prose line {i} kept flowing\n"));
        }
        src.push_str("fn survives() {}\n");
        let (r, _) = redact(&src);
        // Inside the cap: the stray marker DID open a block (conservative).
        assert!(!r.contains("prose line 0 "), "line inside cap leaked");
        // The marker line consumes run slot 1, so prose lines 0..=CAP-2 are the
        // remaining CAP-1 blanked lines; CAP-1 is the first survivor.
        let first_surviving = PEM_BLOCK_LINE_CAP - 1;
        assert!(
            !r.contains(&format!("prose line {} ", first_surviving - 1)),
            "line inside cap leaked"
        );
        assert!(
            r.contains(&format!("prose line {first_surviving} ")),
            "block did not auto-close at the cap"
        );
        assert!(r.contains("fn survives() {}"), "trailing code lost");
    }

    /// Re-verify defect (2026-07-06): the cap run must restart at a same-line
    /// END-then-BEGIN junction. With a cumulative run, a first block ending
    /// near the cap forces the auto-close a few lines into the SECOND block and
    /// leaks its `/`-split body tail.
    #[test]
    fn same_line_junction_restarts_the_cap_run_per_block() {
        let mut src = String::from("-----BEGIN CERTIFICATE-----\n");
        for i in 0..PEM_BLOCK_LINE_CAP - 2 {
            src.push_str(&format!("certbody{i}AAAA\n"));
        }
        src.push_str("-----END CERTIFICATE----------BEGIN RSA PRIVATE KEY-----\n");
        src.push_str("WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez\n");
        src.push_str("PjLm73Nq/XrWt84Uz/MnBv65Cx/QwEr21Ty/UiOp09As\n");
        src.push_str("-----END RSA PRIVATE KEY-----\n");
        src.push_str("fn after() {}\n");
        let (r, _) = redact(&src);
        for shard in ["WqZx83Ky", "KvYw52Ez", "PjLm73Nq", "UiOp09As"] {
            assert!(!r.contains(shard), "second-block shard leaked past a cumulative cap run: {shard} in {r}");
        }
        assert!(r.contains("fn after() {}"), "trailing code lost: {r}");
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
