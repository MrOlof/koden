//! Secrets gate — index-layer leak regression (CONCEPT §7.1 [DP-24]/[DP-25];
//! probe 2026-07-05). Drives the REAL pipeline (`worker::index_dir` → walk →
//! binary-sniff → blake3 → secrets redact → FTS index) over fixture files that
//! each carry one inline secret shape, then proves the secrets are unrecoverable
//! two independent ways: (a) raw SQL over `code_fts` via the read-only snapshot,
//! bypassing the search API entirely, and (b) the public `search` API. A benign
//! high-entropy pure-hex control (git SHA) MUST index and hit both ways, so the
//! proof cannot pass vacuously (a silently-empty index would fail the control).

use std::path::Path;

use koden_lib::modules::brain::store::{open_readonly_snapshot, SearchIndex, SqliteIndex};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "leak";

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

/// Raw storage inspection: TRUE if `needle` (the lowercase form the tokenizer
/// would store) appears ANYWHERE in the FTS streams — content, symbols, or path
/// — independent of the search API's query tokenization.
fn raw_hit(db: &Path, needle: &str) -> bool {
    let conn = open_readonly_snapshot(db).expect("readonly snapshot");
    let pat = format!("%{needle}%");
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM code_fts
             WHERE content LIKE ?1 OR symbols LIKE ?1 OR path LIKE ?1",
            [&pat],
            |r| r.get(0),
        )
        .expect("raw scan");
    n > 0
}

/// The probe's inline secret shapes (sk-, AKIA, ghp_, JWT, Azure `AccountKey=`,
/// and THREE PEM-block variants of the `/`-fragmented-body leak: bare marker
/// line inside the literal; the marker sharing the ASSIGNMENT line of a
/// raw-string literal; and — review round 2 — per-line QUOTED string
/// concatenation, whose quote/escape/`+` chars evade the pure-base64 body
/// predicate), each planted in an otherwise-normal source file. Fixture
/// identifiers are digit-free and chosen so no query sub-token of any secret
/// collides with a benign stored token (else the OR-match FTS query would
/// return relevance-artifact hits that are not leaks — probe's ghp_/"12" note).
#[test]
fn planted_inline_secrets_unrecoverable_from_storage_and_search() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();

    write(
        root,
        "src/openai.ts",
        b"const openAiSample = \"sk-ABCD1234efgh5678IJKL9012mnopQRSTUV\";\n\
          export function loadOpenAiConfig() {}\n",
    );
    write(
        root,
        "src/aws.ts",
        b"const awsAccessSample = \"AKIAIOSFODNN7EXAMPLE\";\n\
          export function validateAwsCredentials() {}\n",
    );
    write(
        root,
        "src/github.ts",
        b"const githubPatSample = \"ghp_1234567890abcdefghijklmnopqrstuvwxyz12\";\n\
          export function fetchGithubRepos() {}\n",
    );
    write(
        root,
        "src/jwt.ts",
        b"const sessionJwtSample = \"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJUp1rwW1gFWFOEjXk\";\n\
          export function decodeSessionClaims() {}\n",
    );
    // The regression shape: multi-line PEM whose body fragments at `/` into
    // sub-16-char shards (each below the entropy detector's length floor).
    write(
        root,
        "src/signing.rs",
        b"pub fn renewSigningMaterial() {}\n\
          pub const DEV_SIGNING_STUB: &str = \"\n\
          -----BEGIN RSA PRIVATE KEY-----\n\
          WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz\n\
          MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8c\n\
          -----END RSA PRIVATE KEY-----\n\
          \";\n",
    );
    // The review-confirmed variant of the same regression: the BEGIN marker
    // SHARES the assignment line of a multi-line raw-string literal (the most
    // common inline-PEM shape in real code), and the variable is NOT
    // secret-named — only detector (d)'s block state protects the body.
    write(
        root,
        "src/tls.go",
        b"package tls\n\
          var inlineCertBundle = `-----BEGIN RSA PRIVATE KEY-----\n\
          TgKr48Vd/BmWs92Hp/NcXz57Jq/FdYt63Lw/QhZv81Mx/RjAu74Ns/KpEw96Tb\n\
          -----END RSA PRIVATE KEY-----`\n\
          func rotateBundledSigner() {}\n",
    );
    // Review round 2 of the same regression: string-CONCATENATION inline PEM
    // (mandatory in Java/C/C++) — every body line is a QUOTED literal
    // (`"...\n" +`) whose quote/escape/concat chars evade the pure-base64 body
    // predicate. The variable is NOT secret-named, so only detector (d)'s
    // quoted-body branch protects the shards.
    write(
        root,
        "src/legacy.java",
        b"class LegacyVault {\n\
          String bundledMaterial = \"-----BEGIN RSA PRIVATE KEY-----\\n\" +\n\
          \"NvBq37Kd/WmZt58Rx/PcYf46Hs/JgLu92Dw/QzEk75Mv/XbAn64Tc/RfHp83Sy\\n\" +\n\
          \"-----END RSA PRIVATE KEY-----\\n\";\n\
          void warmLegacyVault() {}\n\
          }\n",
    );
    write(
        root,
        "src/azure.ts",
        b"const blobConn = \"DefaultEndpointsProtocol=https;AccountName=devstore;AccountKey=wJalrXUtnFEMIK7MDENGqPxRfiCYSAMPLE;EndpointSuffix=core.windows.net\";\n\
          export function connectAzureStorage() {}\n",
    );
    // Positive control: benign high-entropy pure-hex (git SHA shape) is a
    // must-not-redact control — it MUST be stored and retrievable, proving the
    // two inspection methods surface content when it exists.
    write(
        root,
        "src/control.ts",
        b"export const releaseCommitSha = \"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0\";\n\
          export function printReleaseBanner() {}\n",
    );

    let db = store.path().join("index.sqlite");
    let idx = SqliteIndex::open(&db).unwrap();
    index_dir(&idx, PID, root);
    assert_eq!(idx.file_count(PID).unwrap(), 9, "all fixture files indexed");

    // (a) Raw SQL over the real storage: no secret substring survives in any
    // stream. Needles are the lowercase alphanumeric runs the tokenizer would
    // have stored had redaction missed them.
    let secret_needles: &[(&str, &str)] = &[
        ("sk- body", "abcd1234efgh5678ijkl9012mnopqrstuv"),
        ("AKIA", "akiaiosfodnn7example"),
        ("ghp_ body", "1234567890abcdefghijklmnopqrstuvwxyz12"),
        ("JWT header", "eyjhbgcioijiuzi1niisinr5cci6ikpxvcj9"),
        ("PEM shard 1", "wqzx83ky"),
        ("PEM shard 2", "vnpb27jm"),
        ("PEM shard 3", "hcqf94dw"),
        ("PEM shard 4", "lqns61bu"),
        ("PEM shard 5", "kvyw52ez"),
        ("PEM shard 6", "pjlm73nq"),
        ("PEM shard 7", "xrwt84uz"),
        ("PEM clean line", "miievqibadanbgkqhkig9w0baqefaascbkcwggsjageaaoibaqc7vjtut9us8c"),
        ("assignment-line PEM shard 1", "tgkr48vd"),
        ("assignment-line PEM shard 2", "bmws92hp"),
        ("assignment-line PEM shard 3", "ncxz57jq"),
        ("assignment-line PEM shard 4", "fdyt63lw"),
        ("assignment-line PEM shard 5", "qhzv81mx"),
        ("assignment-line PEM shard 6", "rjau74ns"),
        ("assignment-line PEM shard 7", "kpew96tb"),
        ("quoted-concat PEM shard 1", "nvbq37kd"),
        ("quoted-concat PEM shard 2", "wmzt58rx"),
        ("quoted-concat PEM shard 3", "pcyf46hs"),
        ("quoted-concat PEM shard 4", "jglu92dw"),
        ("quoted-concat PEM shard 5", "qzek75mv"),
        ("quoted-concat PEM shard 6", "xban64tc"),
        ("quoted-concat PEM shard 7", "rfhp83sy"),
        ("AccountKey", "wjalrxutnfemik7mdengqpxrficysample"),
    ];
    for (label, needle) in secret_needles {
        assert!(!raw_hit(&db, needle), "secret in raw FTS storage ({label}): {needle}");
    }

    // (b) Public search API: querying each planted secret returns ZERO hits.
    let secret_queries = [
        "sk-ABCD1234efgh5678IJKL9012mnopQRSTUV",
        "AKIAIOSFODNN7EXAMPLE",
        "ghp_1234567890abcdefghijklmnopqrstuvwxyz12",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJUp1rwW1gFWFOEjXk",
        "WqZx83Ky/VnPb27Jm/HcQf94Dw/LqNs61Bu/KvYw52Ez/PjLm73Nq/XrWt84Uz",
        "TgKr48Vd/BmWs92Hp/NcXz57Jq/FdYt63Lw/QhZv81Mx/RjAu74Ns/KpEw96Tb",
        "NvBq37Kd/WmZt58Rx/PcYf46Hs/JgLu92Dw/QzEk75Mv/XbAn64Tc/RfHp83Sy",
        "wJalrXUtnFEMIK7MDENGqPxRfiCYSAMPLE",
    ];
    for q in secret_queries {
        let hits = idx.search(Some(PID), q, 20).expect("search");
        assert!(hits.is_empty(), "search-api hit for planted secret {q}: {hits:?}");
    }

    // Positive control — the proof is not vacuous: the benign hex sha IS stored
    // (raw) and retrievable (search), and redaction stayed surgical (the code
    // surrounding every secret is still searchable).
    let sha = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
    assert!(raw_hit(&db, sha), "control sha missing from raw storage (vacuous test)");
    assert!(
        !idx.search(Some(PID), sha, 20).unwrap().is_empty(),
        "control sha not searchable (vacuous test)"
    );
    for benign in [
        "loadOpenAiConfig",
        "validateAwsCredentials",
        "fetchGithubRepos",
        "decodeSessionClaims",
        "renewSigningMaterial",
        "rotateBundledSigner",
        "warmLegacyVault",
        "connectAzureStorage",
        "printReleaseBanner",
    ] {
        assert!(
            !idx.search(Some(PID), benign, 20).unwrap().is_empty(),
            "non-secret neighbor no longer searchable: {benign}"
        );
    }
}
