//! Koden Brain — relevance benchmark + offline weight calibration (CONCEPT §12.2,
//! BUILD-PROMPT §13.12).
//!
//! Anti-gaming rules honored: a labeled ground-truth corpus AND a **negative
//! control** (queries whose right answer is "not here"); measured-only graded
//! metrics (recall@5 floor gate + MRR + precision@1); CONFUSER fixtures so the
//! corpus can actually DISCRIMINATE weight settings (without them every reasonable
//! weighting scores ~1.0 — a vanity number); worst cases surfaced; and a
//! deliberately-honest "semantic-intent" band that P0's lexical-only search is
//! *expected to miss* (semantic is P5), reported-not-asserted.
//!
//! Run `cargo test --test brain_bench -- --nocapture` for the report.
//! Run the (ignored) calibration sweep with:
//!   `cargo test --test brain_bench -- --ignored --nocapture`

use std::path::Path;

use koden_lib::modules::brain::store::{SearchIndex, SearchWeights, SqliteIndex};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "bench";
const TOPK: usize = 5;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A small but realistic mixed TS/Rust corpus, PLUS confuser files that mention a
/// query's term ONLY in their body while a different file is the labeled rank-1
/// answer (in its path) — so W_IDENTITY/RRF_W_IDENTITY actually decides rank-1.
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
        // --- CONFUSERS: the query term is ONLY in the TARGET's path (its body avoids
        // it) and ONLY in the DISTRACTOR's body (its path avoids it). So the target
        // appears solely in the identity leg, the distractor solely in the content
        // leg → which one is rank-1 is decided entirely by identity-vs-content weight.
        ("src/webhook/handler.ts", "export function handleHook(req) { return process(req.body); }"),
        ("src/api/dispatch.ts", "export function dispatch(evt) { /* delivers each webhook payload; webhook retry */ return fanout(evt); }"),
        ("src/telemetry/collector.ts", "export function collect(sample) { return buffer.push(sample); }"),
        ("src/metrics/sink.ts", "export function flush() { /* drains telemetry; telemetry batch */ return io.write(); }"),
        ("src/scheduler/cron.ts", "export function runCron(job) { return ticker.add(job); }"),
        ("src/jobs/queue.ts", "export function enqueue(j) { /* the scheduler drains this; scheduler ordering */ return q.push(j); }"),
    ];
    for (rel, body) in files {
        write(root, rel, body);
    }
    files.len()
}

/// Labeled positives: the query's term is in the target PATH; with confusers
/// present, a body-only weighting would rank the distractor first.
fn positives() -> Vec<(&'static str, &'static str)> {
    vec![
        ("login handler", "src/auth/login.ts"),
        ("validate session", "src/auth/session.ts"),
        ("stripe checkout", "src/payments/checkout.ts"),
        ("issue refund", "src/payments/refund.ts"),
        ("format currency", "src/utils/currency.ts"),
        ("run migrations", "src/db/migrate.rs"),
        ("connection pool", "src/db/pool.rs"),
        ("register routes", "src/api/router.ts"),
        ("send email notification", "src/notifications/email.ts"),
        // confuser-discriminated: term only in the target PATH vs only in a
        // distractor BODY — identity-vs-content weighting decides rank-1.
        ("webhook", "src/webhook/handler.ts"),
        ("telemetry", "src/telemetry/collector.ts"),
        ("scheduler", "src/scheduler/cron.ts"),
    ]
}

fn negatives() -> Vec<&'static str> {
    vec!["kubernetes deployment yaml", "graphql subscription resolver", "webassembly compiler backend"]
}

/// `(reciprocal_rank, precision_at_1, hit_at_k)` of `expect` in a result list.
fn score_query(res: &[koden_lib::modules::brain::Hit], expect: &str) -> (f64, bool, bool) {
    let rank = res.iter().position(|h| h.path == expect);
    match rank {
        Some(i) => (1.0 / (i as f64 + 1.0), i == 0, i < TOPK),
        None => (0.0, false, false),
    }
}

#[test]
fn relevance_benchmark_with_negative_control() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    let corpus_files = build_corpus(root);

    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    let stats = index_dir(&idx, PID, root);

    let positives = positives();
    let negatives = negatives();
    let semantic: &[(&str, &str)] = &[
        ("money formatting", "src/utils/currency.ts"),
        ("user authentication flow", "src/auth/login.ts"),
    ];

    let mut pos_hit = 0usize;
    let mut mrr_sum = 0.0f64;
    let mut p1_hit = 0usize;
    let mut pos_misses: Vec<&str> = Vec::new();
    for (q, expect) in &positives {
        let res = idx.search(Some(PID), q, TOPK).unwrap();
        let (rr, p1, hit) = score_query(&res, expect);
        mrr_sum += rr;
        if p1 {
            p1_hit += 1;
        }
        if hit {
            pos_hit += 1;
        } else {
            pos_misses.push(q);
        }
    }
    let n = positives.len() as f64;
    let recall = pos_hit as f64 / n;
    let mrr = mrr_sum / n;
    let precision_at_1 = p1_hit as f64 / n;

    let mut neg_leaks: Vec<(&str, String)> = Vec::new();
    for q in &negatives {
        if let Some(h) = idx.search(Some(PID), q, TOPK).unwrap().first() {
            neg_leaks.push((q, h.path.clone()));
        }
    }

    let mut sem_hit = 0usize;
    let mut sem_lines: Vec<String> = Vec::new();
    for (q, ideal) in semantic {
        let hit = idx.search(Some(PID), q, TOPK).unwrap().iter().take(TOPK).any(|h| h.path == *ideal);
        if hit {
            sem_hit += 1;
        }
        sem_lines.push(format!("    \"{q}\" -> ideal {ideal}: {}", if hit { "HIT" } else { "miss (lexical gap)" }));
    }

    println!("\n===== KODEN BRAIN — RELEVANCE BENCHMARK =====");
    println!("corpus: {corpus_files} files, indexed {} (pruned {})", stats.indexed, stats.pruned);
    println!(
        "coverage: {} positive (incl. confusers) + {} negative-control + {} semantic-intent queries",
        positives.len(),
        negatives.len(),
        semantic.len()
    );
    println!("\n[positives] recall@{TOPK} = {pos_hit}/{} = {recall:.2} · MRR = {mrr:.3} · precision@1 = {precision_at_1:.2} (measured)", positives.len());
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
    // The 9 base positives are rank-1 under any non-degenerate weighting, so a 0.75
    // floor (9/12) would pass even if ALL 3 confusers regressed. Floor at 0.90 (≥11/12)
    // so a confuser regression is actually caught here too (the hard confuser
    // invariant also lives in `path_identity_outranks_body_confuser`).
    assert!(precision_at_1 >= 0.90, "precision@1 {precision_at_1:.2} below 0.90 — confusers losing rank-1");
}

/// The confuser invariant at bench scale: a body-only mention must NOT outrank the
/// path/identity match — i.e. the identity weights actually decide rank-1.
#[test]
fn path_identity_outranks_body_confuser() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    build_corpus(work.path());
    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    index_dir(&idx, PID, work.path());
    for (q, target) in [("webhook", "src/webhook/handler.ts"), ("telemetry", "src/telemetry/collector.ts"), ("scheduler", "src/scheduler/cron.ts")] {
        let res = idx.search(Some(PID), q, TOPK).unwrap();
        assert_eq!(res.first().map(|h| h.path.as_str()), Some(target), "'{q}': path match must be rank-1, got {res:?}");
    }
}

/// Anti-vanity guard that RUNS in CI: prove the corpus genuinely discriminates by
/// showing the production weights beat a content-dominant weighting on the confuser
/// MRR (a flat-1.0 corpus would make these equal). This is what stops the benchmark
/// from being a vanity 1.0.
#[test]
fn production_weights_beat_content_dominant() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    build_corpus(work.path());
    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    index_dir(&idx, PID, work.path());

    let positives = positives();
    let mrr = |w: &SearchWeights| {
        positives.iter().map(|(q, e)| score_query(&idx.search_weighted(Some(PID), q, TOPK, w).unwrap(), e).0).sum::<f64>()
            / positives.len() as f64
    };
    let prod = SearchWeights::default();
    let content_dominant = SearchWeights { rrf_identity: 0.25, ..SearchWeights::default() };
    let (p, c) = (mrr(&prod), mrr(&content_dominant));
    assert!(p > c, "production weights ({p:.3}) must beat content-dominant ({c:.3}) — corpus must discriminate");

    // Defend the ACTUAL decision boundary: rrf_identity == rrf_content (1.0) is in the
    // LOSING band (ties break to the distractor), so production must beat it too —
    // this is what makes the "STRICT rrf_identity > rrf_content" claim CI-enforced and
    // stops a future tuner from lowering the default to 1.0.
    let boundary = SearchWeights { rrf_identity: prod.rrf_content, ..SearchWeights::default() };
    assert!(p > mrr(&boundary), "production must beat the equal-weight boundary (1.0) — strict > is load-bearing");
    assert!(prod.rrf_identity > prod.rrf_content, "production invariant: rrf_identity > rrf_content (strict)");
}

/// V2.3 boundedness guard: the temporal boost must NUDGE, never BURY. Even when a
/// body-only (content-leg) distractor is made fresh AND very frequent while the
/// path-match target is stale, the path-match must stay rank-1 — the bounded boost
/// (max < cross-leg RRF margin) guarantees it. This is the ONLY condition that
/// exercises differential recency (index_dir stamps uniformly), so without it CI
/// could never catch a recency-buries-path-match regression.
#[test]
fn temporal_boost_cannot_bury_path_match() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    build_corpus(work.path());
    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    index_dir(&idx, PID, work.path());
    let day = 86_400_000i64;
    let now = 1_000 * day;
    // body-only distractor: fresh + very frequent (max boost).
    for _ in 0..40 {
        idx.record_access(PID, "src/api/dispatch.ts", now).unwrap();
    }
    // path-match target: stale (300d old).
    idx.record_access(PID, "src/webhook/handler.ts", now - 300 * day).unwrap();
    let res = idx.search(Some(PID), "webhook", TOPK).unwrap();
    assert_eq!(
        res.first().map(|h| h.path.as_str()),
        Some("src/webhook/handler.ts"),
        "bounded boost: a fresh+frequent body-only hit must NOT bury the stale path-match: {res:?}"
    );
}

/// Offline weight CALIBRATION sweep (BUILD-PROMPT §13.12). Grid-sweeps a fixed
/// (no-RNG) set of weights via `search_weighted`, maximizing MRR over the labeled
/// positives SUBJECT TO the negative control staying empty (the regularizer that
/// stops a "rank everything high" weighting from winning). Prints a sorted table.
/// #[ignore]d — a tuning artifact, never a CI gate.
#[test]
#[ignore]
fn weight_sweep_reports_mrr_subject_to_zero_leaks() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    build_corpus(work.path());
    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    index_dir(&idx, PID, work.path());

    let positives = positives();
    let negatives = negatives();
    let mut rows: Vec<(String, f64, usize)> = Vec::new();
    // rrf_identity spans content-DOMINANT (0.25/0.5 < rrf_content 1.0, where the
    // body-only confuser wins → MRR drops) through identity-dominant — so the sweep
    // genuinely discriminates instead of reporting a flat 1.0.
    for path_w in [2.0f64, 3.0, 4.0] {
        for rrf_id in [0.25f64, 0.5, 1.0, 1.5, 3.0] {
            let w = SearchWeights {
                identity_bm25: (path_w, 1.5, 0.0),
                content_bm25: (0.0, 0.0, 1.0),
                rrf_identity: rrf_id,
                rrf_content: 1.0,
            };
            let mut mrr = 0.0;
            for (q, expect) in &positives {
                let res = idx.search_weighted(Some(PID), q, TOPK, &w).unwrap();
                mrr += score_query(&res, expect).0;
            }
            mrr /= positives.len() as f64;
            let leaks = negatives
                .iter()
                .filter(|q| !idx.search_weighted(Some(PID), q, TOPK, &w).unwrap().is_empty())
                .count();
            rows.push((format!("path_bm25={path_w} rrf_identity={rrf_id}"), mrr, leaks));
        }
    }
    // Objective: max MRR among leak-free settings.
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    println!("\n===== WEIGHT CALIBRATION SWEEP (max MRR s.t. leaks==0) =====");
    println!("{:<40} {:>8} {:>6}", "weights", "MRR", "leaks");
    for (label, mrr, leaks) in &rows {
        let flag = if *leaks == 0 { "" } else { "  <- DISQUALIFIED (neg leak)" };
        println!("{label:<40} {mrr:>8.3} {leaks:>6}{flag}");
    }
    let best = rows.iter().filter(|(_, _, l)| *l == 0).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    match best {
        Some((label, mrr, _)) => println!("\nBEST (leak-free): {label} -> MRR {mrr:.3}"),
        None => println!("\nno leak-free weighting found"),
    }
    println!("default (production) for comparison:");
    let d = SearchWeights::default();
    let mut dmrr = 0.0;
    for (q, expect) in &positives {
        dmrr += score_query(&idx.search_weighted(Some(PID), q, TOPK, &d).unwrap(), expect).0;
    }
    println!("  {d:?} -> MRR {:.3}", dmrr / positives.len() as f64);
    println!("============================================================\n");
    assert!(best.is_some(), "expected at least one leak-free weighting in the grid");
}
