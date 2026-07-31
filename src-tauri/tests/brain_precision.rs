//! Koden Brain — quantified search-quality regression gate (precision@10 /
//! recall@10 over a hand-labeled corpus).
//!
//! Complements `brain_bench.rs` (rank-1 / MRR / weight calibration) with SET
//! quality: how polluted is the top-10, and does everything relevant make it in?
//! Anti-vanity rules honored:
//!   - hand-labeled relevant sets chosen WITH the corpus (not fitted after);
//!   - DECOY files that share query tokens but are labeled NOT relevant, so
//!     precision can actually drop (a corpus where every hit is relevant would
//!     measure a vanity 1.0);
//!   - >=3 NEGATIVE CONTROLS (empty relevant sets): token-disjoint ones must
//!     > return ZERO hits, and the aggregate pollution ceiling makes a
//!     > return-everything ranker FAIL (it would score pollution ~1.0);
//!   - floors set from MEASURED values with headroom, never tuned upward to
//!     flatter the search; known-weak query classes are reported, not hidden.
//!
//! Metric definitions (documented so the numbers stay comparable over time):
//!   precision@10 = |relevant ∩ top10| / |top10 returned|   (0.0 if none returned)
//!   recall@10    = |relevant ∩ top10| / |relevant|
//! The denominator of precision is the RETURNED count (FTS returns only lexical
//! matches, usually <10 on this corpus), so it measures pollution of what the
//! user actually sees. Negative controls: pollution = |top10 returned| / 10.
//!
//! Run `cargo test --test brain_precision -- --nocapture` for the full table.

use std::path::Path;

use koden_lib::modules::brain::store::{SearchIndex, SearchWeights, SqliteIndex};
use koden_lib::modules::brain::worker::index_dir;

const PID: &str = "prec";
const TOPK: usize = 10;

// ---------------------------------------------------------------------------
// Floors — measured on this exact corpus:
//   2026-07-07 (introduction):        macro P@10 = 0.53 · macro R@10 = 0.96 ·
//                                     neg pollution = 0.05 · camel-token P@10 = 0.29
//   2026-07-07 (V3 coverage re-rank): macro P@10 = 0.96 · macro R@10 = 0.96 ·
//                                     neg pollution = 0.05 · camel-token P@10 = 1.00
// Floors raised DELIBERATELY with the multi-token coverage gate (blend + relative
// prune in search_with_weights) — same ~0.10 headroom policy as at introduction.
// Headroom is deliberate (regression gate, not a leaderboard). Raise/lower
// deliberately, never silently — re-measure with --nocapture and update the
// numbers in this comment alongside the consts.
// ---------------------------------------------------------------------------
const FLOOR_MACRO_PRECISION: f64 = 0.85;
const FLOOR_MACRO_RECALL: f64 = 0.85;
/// The camel-token class was the measured weak class (0.29 pre-coverage); the
/// coverage gate is what fixed it, so it gets its own floor (measured 1.00,
/// 2026-07-07) to stop a silent regression of exactly that fix.
const FLOOR_CAMEL_PRECISION: f64 = 0.85;
/// Ceiling on avg |hits|/10 over the negative controls. A return-everything
/// ranker scores 1.0 here and MUST fail; measured honest value is 0.05 (only
/// the deliberate hard negative "graphql subscription resolver" leaks via the
/// shared "subscription" token).
const CEILING_NEG_POLLUTION: f64 = 0.15;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Synthetic project: ~34 real files across auth/billing/search/config/notify/
/// storage/api/users domains + 6 DECOYS that share query tokens without being
/// relevant (analytics event names, icon maps, roadmap prose, templates,
/// fixtures, legacy shims). Deterministic content, no randomness.
const FILES: &[(&str, &str)] = &[
    // --- auth ---
    ("src/auth/login.ts", "export function loginHandler(req: Request) { return checkCredentials(req.body); }"),
    ("src/auth/logout.ts", "export function logoutHandler(req: Request) { destroyCookie(req); }"),
    ("src/auth/session.ts", "export class SessionStore { createSession(userId: string) { return this.persist(userId); } }"),
    ("src/auth/password_reset.ts", "export function sendPasswordResetEmail(address: string) { const resetToken = mintResetToken(); mailer.deliver(address, resetToken); }"),
    ("src/auth/mfa.ts", "// multi-factor authentication challenge\nexport function verifyTotpCode(code: string) { return totp.check(code); }"),
    ("src/auth/oauth.ts", "export function exchangeAuthorizationCode(code: string) { return github.exchange(code); }"),
    // --- billing ---
    ("src/billing/invoice.ts", "export function generateInvoice(order: Order) { return renderPdf(order.lines); }"),
    ("src/billing/subscription.ts", "export function cancelSubscription(id: string) { return billingApi.cancel(id); }\nexport function renewSubscription(id: string) { return billingApi.renew(id); }"),
    ("src/billing/payment_gateway.ts", "export function chargeCreditCard(card: Card, amountCents: number) { return gateway.charge(card, amountCents); }"),
    ("src/billing/tax.ts", "export function calculateSalesTax(amountCents: number, region: string) { return amountCents * rateFor(region); }"),
    ("src/billing/refunds.ts", "export function processRefund(paymentId: string) { return gateway.reverse(paymentId); }"),
    // --- search ---
    ("src/search/indexer.rs", "pub fn build_inverted_index(docs: &[Doc]) -> InvertedIndex { InvertedIndex::from_docs(docs) }"),
    ("src/search/query_parser.rs", "pub fn parse_query(raw: &str) -> Query { Query::from_terms(raw.split_whitespace()) }"),
    ("src/search/ranker.rs", "// bm25 ranking\npub fn score_documents(q: &Query, docs: &[Doc]) -> Vec<Scored> { bm25_rank(q, docs) }"),
    ("src/search/tokenizer.rs", "pub fn tokenize_text(raw: &str) -> Vec<Token> { raw.split(' ').map(Token::new).collect() }"),
    // --- config ---
    ("src/config/loader.ts", "export function loadConfigFile(path: string): AppConfig { return parseToml(readFile(path)); }"),
    ("src/config/env.ts", "export function readEnvOverrides(): Partial<AppConfig> { return pickPrefixed(process.env, 'APP_'); }"),
    ("src/config/schema.ts", "export function validateConfigSchema(candidate: unknown): AppConfig { return schema.parse(candidate); }"),
    // --- notifications ---
    ("src/notify/email.ts", "export function sendEmailNotification(to: string, subject: string) { return smtp.deliver(to, subject); }"),
    ("src/notify/sms.ts", "export function sendSmsNotification(phone: string, text: string) { return twilio.create(phone, text); }"),
    ("src/notify/push.ts", "export function sendPushNotification(deviceToken: string, payload: Payload) { return fcm.dispatch(deviceToken, payload); }"),
    // --- storage ---
    ("src/storage/s3.ts", "export function uploadToBucket(key: string, bytes: Buffer) { return s3client.put(key, bytes); }"),
    ("src/storage/local_disk.ts", "export function writeFileAtomic(path: string, bytes: Buffer) { tmp.write(bytes); fs.rename(tmp, path); }"),
    // --- api ---
    ("src/api/router.ts", "export function registerRoutes(app: App) { app.use('/v1', v1Routes); }"),
    ("src/api/middleware.ts", "export function rateLimiter(opts: LimiterOptions) { return tokenBucket(opts); }"),
    ("src/api/errors.ts", "export class ApiError extends Error { constructor(public status: number, message: string) { super(message); } }"),
    // --- users ---
    ("src/users/profile.ts", "export function updateUserProfile(userId: string, patch: ProfilePatch) { return repo.update(userId, patch); }"),
    ("src/users/avatar.ts", "export function storeAvatarImage(userId: string, img: Buffer) { return blobStore.put(userId, img); }"),
    // --- misc filler (realistic bulk; never labeled relevant) ---
    ("src/logging/logger.ts", "export function createLogger(scope: string) { return pino({ name: scope }); }"),
    ("src/queue/jobs.ts", "export function enqueueJob(job: Job) { return redis.lpush('jobs', job.id); }"),
    ("src/db/migrations.rs", "pub fn run_pending_migrations(conn: &mut Conn) -> Result<()> { apply_all(conn) }"),
    ("src/db/pool.rs", "pub fn create_pool(url: &str) -> Pool { Pool::builder().build(url) }"),
    ("src/i18n/translate.ts", "export function translateKey(locale: string, key: string) { return catalog[locale][key]; }"),
    ("src/metrics/histogram.ts", "export function recordLatency(millis: number) { histogram.observe(millis); }"),
    // --- DECOYS: share tokens with queries, labeled NOT relevant ---
    ("src/analytics/events.ts", "export const EVENTS = ['login_clicked', 'invoice_viewed', 'search_performed', 'notification_opened', 'subscription_upgraded'];"),
    ("src/ui/icons.ts", "export const ICONS = { billing: 'coins', search: 'magnifier', config: 'gear', notification: 'bell' };"),
    ("docs/roadmap.md", "# Roadmap\n\n- polish the billing pages\n- faster search results\n- simplify the config story\n"),
    ("src/emails/welcome_template.ts", "export const WELCOME_EMAIL = '<h1>Welcome aboard</h1>';"),
    ("tests/fixtures/password_fixtures.ts", "// fixture password list for the fuzzer\nexport const SAMPLE_PASSWORD_WORDS = ['correct', 'horse', 'battery', 'staple'];"),
    ("src/legacy/auth_shim.ts", "// kept only for the old login redirect quirk\nexport function legacyLoginShim() { return redirect('/login-old'); }"),
];

struct QueryCase {
    class: &'static str,
    query: &'static str,
    /// Hand-labeled relevant set (chosen while writing the corpus above).
    /// Empty = negative control: nothing in the corpus matches the concept.
    relevant: &'static [&'static str],
}

const fn q(class: &'static str, query: &'static str, relevant: &'static [&'static str]) -> QueryCase {
    QueryCase { class, query, relevant }
}

/// 19 labeled positives + 4 negative controls (3 token-disjoint + 1 hard).
const QUERIES: &[QueryCase] = &[
    // exact-name (identifier as typed)
    q("exact-name", "loginHandler", &["src/auth/login.ts"]),
    q("exact-name", "generateInvoice", &["src/billing/invoice.ts"]),
    q("exact-name", "parse_query", &["src/search/query_parser.rs"]),
    q("exact-name", "rateLimiter", &["src/api/middleware.ts"]),
    // camelCase-token (multi-part identifier — parts must all resolve)
    q("camel-token", "sendPasswordResetEmail", &["src/auth/password_reset.ts"]),
    q("camel-token", "validateConfigSchema", &["src/config/schema.ts"]),
    q("camel-token", "uploadToBucket", &["src/storage/s3.ts"]),
    // concept (multi-term, natural phrasing)
    q("concept", "password reset flow", &["src/auth/password_reset.ts"]),
    q("concept", "cancel subscription", &["src/billing/subscription.ts"]),
    q("concept", "charge credit card", &["src/billing/payment_gateway.ts"]),
    q("concept", "bm25 ranking", &["src/search/ranker.rs"]),
    q("concept", "load config file", &["src/config/loader.ts"]),
    q("concept", "send sms notification", &["src/notify/sms.ts"]),
    q("concept", "session store", &["src/auth/session.ts"]),
    q("concept", "multi factor authentication", &["src/auth/mfa.ts"]),
    q("concept", "calculate sales tax", &["src/billing/tax.ts"]),
    q("concept", "update user profile", &["src/users/profile.ts"]),
    // multi-relevant (recall across a whole domain)
    q("multi-relevant", "send notification", &["src/notify/email.ts", "src/notify/sms.ts", "src/notify/push.ts"]),
    // hard-concept: "authentication" (the word) only appears in mfa.ts; the
    // other auth-flow files are reachable only via the "auth" path abbreviation,
    // which lexical search cannot bridge. Expected LOW recall — reported honestly.
    q("hard-concept", "authentication flow", &["src/auth/login.ts", "src/auth/session.ts", "src/auth/mfa.ts", "src/auth/oauth.ts"]),
    // negative controls — nothing in the corpus is about these concepts.
    q("negative", "kubernetes ingress yaml", &[]),
    q("negative", "bluetooth pairing firmware", &[]),
    q("negative", "webassembly interpreter opcode", &[]),
    // hard negative: shares the "subscription" token with billing (a DIFFERENT
    // concept) — hits here are pure pollution and count against the ceiling.
    q("negative-hard", "graphql subscription resolver", &[]),
];

#[test]
fn precision_recall_regression_gate() {
    let work = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let root = work.path();
    for (rel, body) in FILES {
        write(root, rel, body);
    }

    let idx = SqliteIndex::open(&store.path().join("index.sqlite")).unwrap();
    let stats = index_dir(&idx, PID, root);
    assert_eq!(
        stats.indexed,
        FILES.len(),
        "fixture drift: every corpus file must index (labels assume all are searchable)"
    );

    let mut pos_p_sum = 0.0f64;
    let mut pos_r_sum = 0.0f64;
    let mut pos_n = 0usize;
    let mut neg_pollution_sum = 0.0f64;
    let mut neg_n = 0usize;
    let mut imperfect_precision = 0usize; // positives where a decoy survived the gate
    let mut ungated_collisions = 0usize; // positives where the UNGATED ranker retrieves a decoy
    // The coverage gate DISABLED — the anti-vanity seam: decoy collision is a
    // property of the corpus + lexical legs, measured BEFORE the gate prunes.
    let ungated = SearchWeights { coverage_w: 0.0, coverage_gate_ratio: 0.0, ..SearchWeights::default() };
    let mut per_class: std::collections::BTreeMap<&str, (f64, f64, usize)> =
        std::collections::BTreeMap::new();

    println!("\n===== KODEN BRAIN — PRECISION/RECALL@{TOPK} REGRESSION GATE =====");
    println!("corpus: {} files ({} indexed)", FILES.len(), stats.indexed);
    println!("{:<14} {:<34} {:>4} {:>6} {:>7} {:>7}", "class", "query", "ret", "rel", "P@10", "R@10");

    for case in QUERIES {
        let res = idx.search(Some(PID), case.query, TOPK).unwrap();
        let retrieved = res.len();
        let rel_found = res.iter().filter(|h| case.relevant.contains(&h.path.as_str())).count();

        if case.relevant.is_empty() {
            // negative control: every hit is pollution.
            let pollution = retrieved as f64 / TOPK as f64;
            neg_pollution_sum += pollution;
            neg_n += 1;
            println!(
                "{:<14} {:<34} {:>4} {:>6} {:>7} {:>7}",
                case.class, case.query, retrieved, "0/0", "-", "-"
            );
            if case.class == "negative" {
                // Token-disjoint negatives must return NOTHING (brain_bench idiom).
                assert!(
                    res.is_empty(),
                    "token-disjoint negative control \"{}\" leaked: {res:?}",
                    case.query
                );
            }
            continue;
        }

        let p = if retrieved == 0 { 0.0 } else { rel_found as f64 / retrieved as f64 };
        let r = rel_found as f64 / case.relevant.len() as f64;
        pos_p_sum += p;
        pos_r_sum += r;
        pos_n += 1;
        if p < 1.0 {
            imperfect_precision += 1;
        }
        let raw = idx.search_weighted(Some(PID), case.query, TOPK, &ungated).unwrap();
        if raw.iter().any(|h| !case.relevant.contains(&h.path.as_str())) {
            ungated_collisions += 1;
        }
        let e = per_class.entry(case.class).or_insert((0.0, 0.0, 0));
        e.0 += p;
        e.1 += r;
        e.2 += 1;
        println!(
            "{:<14} {:<34} {:>4} {:>6} {:>7.2} {:>7.2}",
            case.class,
            case.query,
            retrieved,
            format!("{rel_found}/{}", case.relevant.len()),
            p,
            r
        );
    }

    let macro_p = pos_p_sum / pos_n as f64;
    let macro_r = pos_r_sum / pos_n as f64;
    let neg_pollution = neg_pollution_sum / neg_n as f64;

    println!("\n[per-class macro averages]");
    for (class, (ps, rs, n)) in &per_class {
        println!("    {:<14} P@10 = {:.2} · R@10 = {:.2}  (n={n})", class, ps / *n as f64, rs / *n as f64);
    }
    println!(
        "\n[positives, n={pos_n}] macro precision@{TOPK} = {macro_p:.2} (floor {FLOOR_MACRO_PRECISION}) · macro recall@{TOPK} = {macro_r:.2} (floor {FLOOR_MACRO_RECALL})"
    );
    println!(
        "[negative controls, n={neg_n}] pollution = {neg_pollution:.2} (ceiling {CEILING_NEG_POLLUTION}; return-everything ranker would score 1.0)"
    );
    println!("known weakness: abbreviation gap (\"authentication\" cannot reach auth/* path tokens) and sibling pollution (send/notification/config tokens fan out) — see hard-concept and low-P concept rows above");
    println!("==================================================================\n");

    // ---- gates (floors from the measured values in the header comment) ----
    assert!(
        macro_p >= FLOOR_MACRO_PRECISION,
        "macro precision@{TOPK} {macro_p:.2} fell below floor {FLOOR_MACRO_PRECISION}"
    );
    assert!(
        macro_r >= FLOOR_MACRO_RECALL,
        "macro recall@{TOPK} {macro_r:.2} fell below floor {FLOOR_MACRO_RECALL}"
    );
    assert!(
        neg_pollution <= CEILING_NEG_POLLUTION,
        "negative-control pollution {neg_pollution:.2} above ceiling {CEILING_NEG_POLLUTION} — ranker is returning junk for concepts the corpus does not contain"
    );
    let (camel_p_sum, _, camel_n) = per_class["camel-token"];
    let camel_p = camel_p_sum / camel_n as f64;
    assert!(
        camel_p >= FLOOR_CAMEL_PRECISION,
        "camel-token P@10 {camel_p:.2} fell below floor {FLOOR_CAMEL_PRECISION} — the V3 coverage-gate fix regressed"
    );
    // Anti-vanity: the decoys must actually collide with queries, otherwise the
    // precision floor is vacuous (nothing could ever pollute the top-10). Since the
    // V3 coverage gate legitimately PRUNES colliders from production results, the
    // collision property is asserted on the UNGATED seam (coverage disabled) — the
    // corpus must still lexically collide, and the gated results must show the gate
    // actually earned its precision (gated ≥ measurably cleaner than ungated is
    // enforced by the floors above being far over the 0.53 pre-gate baseline).
    assert!(
        ungated_collisions >= 5,
        "only {ungated_collisions} positive queries retrieve any decoy UNGATED — corpus no longer discriminates; precision floor is vacuous"
    );
    println!("anti-vanity: {ungated_collisions} positives collide ungated; {imperfect_precision} still imperfect after the coverage gate");
}
