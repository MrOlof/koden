//! Koden Brain — relevance benchmark (CONCEPT §12.2, BUILD-PROMPT §13.12).
//!
//! Anti-gaming rules honored: a labeled ground-truth corpus AND a **negative
//! control** (queries whose right answer is "not here"); measured-only averages
//! reported; worst cases surfaced; and a deliberately-honest "semantic-intent"
//! band that P0's lexical-only search is *expected to miss* (semantic is P5) — so
//! the report is a discriminating measurement, never a vanity 1.0.
//!
//! Run `cargo test --test brain_bench -- --nocapture` to see the full report; the
//! committed numbers in docs/koden-brain-BENCH.md are pasted from a real run.

use std::path::Path;

use koden_lib::modules::brain::store::{SearchIndex, SqliteIndex};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "bench";
const TOPK: usize = 5;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A small but realistic mixed TS/Rust corpus.
fn build_corpus(root: &Path) -> usize {
    let files: &[(&str, &str)] = &[
        ("src/auth/login.ts", "export function loginHandler(req) { return validateSession(req.token); }"),
        ("src/auth/session.ts", "export function validateSession(token: string): boolean { return verifyToken(token); }"),
        ("src/payments/checkout.ts", "export function createStripeCheckout(cart: Cart) { return stripe.checkout(cart); }"),
        ("src/payments/refund.ts", "export function issueRefund(paymentId: string) { return stripe.refund(paymentId); }"),
        ("src/utils/currency.ts", "export function formatCurrency(amount: number, code: string): string { return intl(amount, code); }"),
        ("src/db/migrate.rs", "pub fn run_migrations(conn: &Connection) -> Result<()> { apply_pending(conn) }"),
        ("src/db/pool.rs", "pub fn build_connection_pool(url: &str) -> Pool { Pool::new(url) }"),
        ("src/api/router.ts", "export function registerRoutes(app: App) { app.use(authRouter); }"),
        ("src/search/indexer.rs", "pub fn build_search_index(docs: Vec<Doc>) -> Index { Index::from(docs) }"),
        ("src/notifications/email.ts", "export function sendEmailNotification(to: string, body: string) { smtp.send(to, body); }"),
    ];
    for (rel, body) in files {
        write(root, rel, body);
    }
    files.len()
}

#[test]
fn relevance_benchmark_with_negative_control() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    let corpus_files = build_corpus(root);

    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    let stats = index_dir(&idx, PID, root);

    // Labeled ground-truth: lexically-aligned positives (target must be top-5).
    let positives: &[(&str, &str)] = &[
        ("login handler", "src/auth/login.ts"),
        ("validate session", "src/auth/session.ts"),
        ("stripe checkout", "src/payments/checkout.ts"),
        ("issue refund", "src/payments/refund.ts"),
        ("format currency", "src/utils/currency.ts"),
        ("run migrations", "src/db/migrate.rs"),
        ("connection pool", "src/db/pool.rs"),
        ("register routes", "src/api/router.ts"),
        ("send email notification", "src/notifications/email.ts"),
    ];

    // Negative control: the right answer is "not here". HARD gate → must be empty.
    let negatives: &[&str] = &[
        "kubernetes deployment yaml",
        "graphql subscription resolver",
        "webassembly compiler backend",
    ];

    // Semantic-intent band: P0 lexical is EXPECTED to miss these (words don't
    // overlap the code). Reported, NOT asserted — the honest gap that semantic
    // search (P5) would close.
    let semantic: &[(&str, &str)] = &[
        ("money formatting", "src/utils/currency.ts"),
        ("user authentication flow", "src/auth/login.ts"),
    ];

    let mut pos_hit = 0usize;
    let mut pos_misses: Vec<&str> = Vec::new();
    for (q, expect) in positives {
        let res = idx.search(Some(PID), q, TOPK).unwrap();
        if res.iter().take(TOPK).any(|h| h.path == *expect) {
            pos_hit += 1;
        } else {
            pos_misses.push(q);
        }
    }
    let recall = pos_hit as f64 / positives.len() as f64;

    let mut neg_leaks: Vec<(&str, String)> = Vec::new();
    for q in negatives {
        let res = idx.search(Some(PID), q, TOPK).unwrap();
        if let Some(h) = res.first() {
            neg_leaks.push((q, h.path.clone()));
        }
    }

    let mut sem_hit = 0usize;
    let mut sem_lines: Vec<String> = Vec::new();
    for (q, ideal) in semantic {
        let res = idx.search(Some(PID), q, TOPK).unwrap();
        let hit = res.iter().take(TOPK).any(|h| h.path == *ideal);
        if hit {
            sem_hit += 1;
        }
        sem_lines.push(format!("    \"{q}\" -> ideal {ideal}: {}", if hit { "HIT" } else { "miss (lexical gap)" }));
    }

    // ---- report (paste into docs/koden-brain-BENCH.md) ----
    println!("\n===== KODEN BRAIN — RELEVANCE BENCHMARK =====");
    println!("corpus: {corpus_files} files, indexed {} (pruned {})", stats.indexed, stats.pruned);
    println!(
        "coverage: {} positive + {} negative-control + {} semantic-intent queries",
        positives.len(),
        negatives.len(),
        semantic.len()
    );
    println!("\n[positives] recall@{TOPK} = {pos_hit}/{} = {:.2} (measured)", positives.len(), recall);
    if pos_misses.is_empty() {
        println!("    worst cases: none");
    } else {
        println!("    worst cases (missed top-{TOPK}): {pos_misses:?}");
    }
    println!("\n[negative control] leaks = {}/{}", neg_leaks.len(), negatives.len());
    for (q, p) in &neg_leaks {
        println!("    LEAK \"{q}\" -> {p}");
    }
    println!("\n[semantic-intent] (P0 lexical expected to miss; semantic is P5) hit {sem_hit}/{}", semantic.len());
    for l in &sem_lines {
        println!("{l}");
    }
    println!("\nknown weakness: pure semantic/synonym queries are out of scope for P0");
    println!("================================================\n");

    // ---- gates ----
    assert!(neg_leaks.is_empty(), "negative-control queries must return nothing: {neg_leaks:?}");
    assert!(recall >= 0.8, "positive recall@{TOPK} {recall:.2} below 0.80 floor; misses: {pos_misses:?}");
}
