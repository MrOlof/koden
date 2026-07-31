---
title: Koden v0.9.0 — first release checklist
created: 2026-07-30
status: DONE — v0.9.0 PUBLISHED 2026-07-31T06:36Z, repo public, updater live
---

## LIBRARIAN ROUND (2026-07-31, after v0.10.0)

Kosta's framing, which reframed the whole feature set: the Librarian runs on a
deliberately CHEAP key and must NOT code — that is the real agents' job. Its job
is to keep everything indexed so those expensive agents get answers fast instead
of reading whole project folders. A cheap model cannot reason its way out of a
bad index, so retrieval quality is the entire product.

1. **`934d9b3` — the activity trail was lying.** `files_activity_payload` had
   RE-IMPLEMENTED the indexer's gate set off the raw watcher batch and drifted:
   3 of 5 gates, missing `rel_under_skip_dir` (holds `.git`) and
   `is_ignored_file`. A `git commit` fanned `.git/index.lock` into the injected
   gist. Fixed structurally — `index_changed_accepted` returns the accepted rels
   (already computed for the edge relink, then discarded) and the trail consumes
   those, so it cannot disagree with the index by construction. SECOND defect:
   render was `take(6)` over a BTreeSet, so lexicographic order gave dot-prefixed
   paths every slot and source could never appear. Now ranked by a PURE function
   of the path string. Ranking by recency would rotate the gist key every
   debounce tick — that is why the intuitive answer is wrong. Negative control
   run: with the old render restored the test reproduces the live symptom.
   Side effect: `reflect::append_recent_activity` renders the same rows into
   every PAID Librarian round, so this was also burning tokens on git plumbing.
2. **`b89259d` — `koden-brain`, a read-only MCP server** (`src-tauri/src/bin/`).
   The index existed but was unreachable from outside the app. Tools:
   brain_search (omit project = search ALL indexed projects), brain_symbol,
   brain_impact, brain_recent_activity, brain_projects. No new deps.
   Unblocked because the brain was ALREADY headless (`examples/brain_cli.rs`:
   "no GUI, no Tauri app") and the store is WAL with `open_readonly` —
   cross-process reads are an explicit design goal (CONCEPT §8).
   Usability pivot: the store keys on an opaque 16-hex id but an agent only
   knows a directory, so tools take `path` (default cwd) and walk UP ancestors
   to the deepest indexed match, via new `registry::project_id_for_root`.
3. **`f056fc6` — the gist advertises retrieval.** Without it the server was
   discoverable only by luck. Worded conditionally; the gist cannot know whether
   the reader registered it.
4. **`2e86554` — the narrative layer.** Even with clean paths, a path list is
   not an answer to "what did we last work on?". The worker now records the git
   HEAD subject as a `commit` activity row when HEAD MOVES, and the gist renders
   commits FIRST in each day line. Chosen over LLM summarization deliberately:
   deterministic, free, cache-stable, cannot hallucinate a history.
   Fail-open off-git and pre-first-commit.
5. **`bf20fc2` — `rust-toolchain.toml`** pinning 1.97.1. `@stable` cost two CI
   cycles when it resolved ahead of the local toolchain.
6. **`0997d8e` — checklist item B closed.** The 15 dirty `.memory` paths
   (ADRs 007-016, brain-verification screenshots, morning reports) committed.

**Signing key backed up.** `_ClaudeSetup/secrets/` now holds the encrypted key
plus a README. Verified scrypt-encrypted (`kdf_alg`=`Sc`) — an earlier memory
note wrongly recorded it as passwordless. The PASSWORD is deliberately not
stored with it and cannot be recovered from GitHub (Actions secrets are
write-only): it must live in a password manager.

**MCP server installed at a stable path**
(`%LOCALAPPDATA%\app.mrolof.koden\bin\koden-brain.exe`, release build) and
registered user-level, so it survives `cargo clean`. New-machine steps are in
`_ClaudeSetup\NEW-MACHINE-HANDOVER.md`.

**svart worktree folded** — merged, shipped, deregistered.

## SHIPPED (2026-07-31)

`v0.9.0` is public at https://github.com/MrOlof/koden/releases/tag/v0.9.0,
built from `main` @ `735755d`. Repo flipped **public**. Verified live and
unauthenticated: `latest.json` → HTTP 200 (version 0.9.0, 7 platforms, every
signature non-empty), `Koden_0.9.0_x64-setup.exe` → HTTP 200, 5,620,559 bytes.

**Signing key verified end to end.** Key id in `tauri.conf.json` and the key id
embedded in the CI-produced `.sig` both `18EB07913027CD66` — the GitHub secret
is the private half of the key the shipped app trusts. This was the failure
that would otherwise have surfaced only after users were on v0.9.0.

Artifacts: `.msi`, NSIS `.exe`, `.deb`, `.rpm`, `.AppImage`, each `.sig`-signed.
macOS deliberately dropped (no Apple credentials; Kosta: Windows + Linux only).

### Six blockers CI had never exercised

1. **Shebang in `scripts/eager-graph.mjs`** — Vite hoists SSR import shims above
   line 1, pushing `#!` mid-module where `#` is an invalid token. Whole vitest
   suite died with a bare *SyntaxError* while plain node loaded the file fine.
   Fix = delete the shebang (always invoked as `node <file>`). **Non-obvious:
   the file is valid JS, `node --check` passes, esbuild transforms it fine.**
2. **Size cap** — total client JS 1.62 MB vs 1500 KB. All overage in lazy
   chunks; startup budget still green at 520/540 KB. Cap → 1700 KB.
3. **40 clippy lints** under 1.94 (`doc_lazy_continuation` ×33 allowed at
   package level, `cloned_ref_to_slice_refs` ×6, `type_complexity` ×1).
4. **2 more clippy lints** under 1.97 — CI's `dtolnay/rust-toolchain@stable`
   was two minors ahead of local. `collapsible_match` deliberately NOT taken
   (clippy wanted a side-effecting `insert` inside a match guard).
5. **Runner disk exhaustion** — the rust job builds the tree 4× (check, clippy,
   nextest, then clippy+nextest under `semantic`). Fixed with
   `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, and
   dropping preinstalled SDK trees. Not reproducible locally — runner property.
6. **Flaky `gist_cache_key_stable_under_concurrent_writes`** — the *correctness*
   assertion never fired; the anti-vacuousness guard (`seen.len() >= 2`) assumed
   the scheduler would interleave within a fixed 400 iterations. Now samples
   until it observes the flip, with a 30s deadline.

### NEXT: svart

`feat/koden-svart` (worktree `Products\koden-svart-wt`, `4e62ce6`) is **55
commits / 267 files / +14,814 −2,349 ahead of main and NOT in v0.9.0**: dark
theme (`globals.css`, `tokens.ts`), Librarian top-right (`Header.tsx`), voice
(push-to-talk, hands-free, headless HUD), the Library, Spaces, Brain Map
animation, Librarian layout/task/note tools.

Kosta's call 2026-07-31: **leave v0.9.0, do svart properly next.** Rebase onto
main FIRST — svart branched before the six fixes above and will otherwise
re-hit them. Expect its own CI round; it has never been run against CI either.
Shipping it as v0.10.0 also gives the real updater test v0.9.0 cannot.

### Still open

- **Signing key has ONE retrievable copy**: `C:\Users\Snorlax\.koden-updater.key`
  (encrypted, `rsign encrypted secret key`) + its password. It lives in
  `%USERPROFILE%`, **outside** the MegaSync tree, so it is NOT backed up and
  will NOT appear on the new laptop. GitHub secrets are write-only and cannot
  be read back. Lose it and every existing install rejects all future updates
  permanently — the trusted key is compiled into shipped binaries.
- Rotate the OpenAI key in the `koden-ai` keyring (outstanding since 2026-07-10).
- Bundle icons (cyan K) — Kosta dislikes them; cosmetic, deferred by his call.
  Note `koden-icon.png` at repo root is the OLD upstream **Terax** logo under a
  Koden filename; unreferenced by the build, but a trap.
- Consider `rust-toolchain.toml` to pin Rust — `@stable` means a new release
  can redden CI on lints unrelated to any change. Cost two cycles tonight.
- Local `LNK1104 msvcrt.lib` is **intermittent and unexplained**. Toolchain
  audited clean (SDK, libs, registry all intact); recurred after a clean build.
  Suspect AV or concurrent cargo processes locking `target\debug\deps`.
- First-run on a clean machine still unproven: fresh appdata under
  `app.mrolof.koden`, no orphaned `app.crynta.terax`, graceful degradation
  without Claude Code CLI / node / git.

## Progress 2026-07-30 (later session)

Local `main` = `3cd18d1`, **140 commits** ahead of `origin/main`. Two new commits:

- `6691155` **fix(ci)** — the two things that made CI red:
  - `scripts/eager-graph.mjs` had a shebang. Vite hoists its SSR import shims
    above line 1, pushing `#!` into the middle of the module where `#` is an
    invalid token, so `eager-budget.test.ts` threw *SyntaxError: Invalid or
    unexpected token* before any test ran and `pnpm test` exited 1. Plain node
    was fine, which is why it only ever failed under vitest. Shebang deleted
    (the script is always run as `node <file>`); comment left so it stays gone.
  - Size gate: total client JS had reached 1.62 MB against a 1500 KB cap.
    Overage is entirely lazy chunks; startup budget is still 520/540 KB. Cap
    raised to 1700 KB (Kosta's call).
- `3cd18d1` **release prep** — 0.9.0 in `package.json`, `Cargo.toml` (+lock),
  `tauri.conf.json`. macOS dropped from `release.yml` + `ci.yml` (Kosta: Windows
  and Linux are what matter, no Apple credentials exist).
  `update-nix-sources.yml` is manual-only now — it hashes the two darwin
  `.app.tar.gz` assets that no longer get built, so `release: published` would
  have failed every release. `nix/package.nix` keeps its darwin surface.

**Verified green locally:** install --frozen-lockfile, lint (0; 112 advisory
warnings), check-types, vitest **38 files / 372 tests**, build, size (both budgets).

**Not verified:** the Rust gates. Local MSVC is broken — `LNK1104: cannot open
file 'msvcrt.lib'` — so `cargo test` cannot link on this machine. Unrelated to
the code; CI runners will be the first real signal. Fixing the local toolchain
is also needed before Koden can be built locally at all.

**Secret scan done:** tracked tree + full history swept for live key shapes.
Every hit is a deliberate fixture (`secrets.rs`, `brain_sandbox.rs`,
`linkDetect.test.ts`, plus AWS's public `AKIAIOSFODNN7EXAMPLE`). Clean.
Still outstanding from B: rotate the OpenAI key in the `koden-ai` keyring.

**D1/D4 resolved:** D4 = macOS dropped. D1 (public) still blocking auto-update.
D3 (svart in or out) still open. Nothing pushed — awaiting Kosta's go.

# Koden v0.9.0 — first release checklist

State at time of writing: `origin/main` = `f00a360` (initial import only).
Local `feat/koden-brain` = `08eb847`, **138 commits ahead**, 16 dirty paths.
`gh release list` empty, latest tag `v0.8.0` (local only). Repo is **private**.

Already done (don't redo): minisign keypair minted (`pubkey` = `66CD2730…`,
not crynta's), `TAURI_SIGNING_PRIVATE_KEY` + password secrets set 2026-06-20,
bundle id = `app.mrolof.koden`, updater endpoint = `MrOlof/koden`,
`release.yml` builds+signs+emits `latest.json` on `v*` tags (draft).

---

## A. Decisions (Kosta — blocking)

- [ ] **D1 — private vs public repo.** The Tauri updater fetches
      `github.com/MrOlof/koden/releases/latest/download/latest.json` unauthenticated.
      Private repo ⇒ 404 ⇒ **auto-update cannot work**. Choose: flip public at
      release (Apache-2.0 + crynta attribution already in place), or stay private
      and install manually on each machine (updater stays off).
- [ ] **D2 — release version.** `package.json` + `src-tauri/tauri.conf.json` are
      both `0.8.0`. Bump both to `0.9.0` (must match the tag; the updater compares
      `latest.json.version` against `getVersion()`).
- [ ] **D3 — what ships.** Merge `feat/koden-brain` → `main` first. `feat/koden-svart`
      (GUI revamp, unmerged, needs rebase onto main) — in or out of v0.9.0?
- [ ] **D4 — macOS.** `release.yml` references `APPLE_CERTIFICATE`, `APPLE_API_KEY`,
      `APPLE_TEAM_ID`, … — none are set. Either drop the two `macos-latest` matrix
      rows for v0.9.0 or confirm the job degrades to unsigned instead of failing.
- [ ] **D5 — Windows signing.** No cert ⇒ SmartScreen warning on first run.
      `signpath-test.yml` exists but is unused. Accept the warning for v0.9.0?

## B. Pre-push hygiene

- [ ] Commit or drop the 16 dirty paths — 8 untracked ADRs (007–016), the two
      `MORNING-REPORT-*.md` (memory says delete after reading), `.memory/INDEX.md`,
      `ADR-004`, `src-tauri/Cargo.toml`.
- [ ] Decide `.koden-memory/` — track (portable memory, that's the design) or ignore.
- [ ] **Secret scan the tree + history before the first push** (`gitleaks` or
      `git log -p | rg -i 'sk-|api[_-]key'`). Repo is private now but may go public (D1).
- [ ] **Rotate the OpenAI key still sitting in the `koden-ai` keyring** — outstanding
      since 2026-07-10.
- [ ] Confirm `LICENSE`/`NOTICE` still carry the crynta Apache-2.0 attribution.

## C. Verify before tagging

- [ ] Full sweep on the merge commit: `cargo test` (expect only
      `authorize_spawn_cwd_blocks_symlink_escape` — Windows symlink privilege),
      `tsc`, `vitest` (expect only `eager-budget.test.ts` — env), 8 brain suites.
- [ ] **CI green on `main`** — `ci.yml` runs on push to main and has *never* run
      against this code.
- [ ] **`workflow_dispatch` the release workflow once before tagging** — catches
      build breaks (node 24 / pnpm / Rust target setup) without burning a tag.
- [ ] Confirm the draft release carries: 4 platform installers + `latest.json`
      with **non-empty `signature` fields**.

## D. Smoke on the new laptop (the actual goal)

- [ ] Install the NSIS `.exe`, first run: fresh appdata under `app.mrolof.koden`,
      no orphaned `app.crynta.terax` state.
- [ ] Terminal + agent launch; `KODEN_SESSION` injection; hooks install into
      `~/.claude/settings.json` on a machine that already has Claude Code
      (and the stale-TERAX-hook migration path is a no-op there).
- [ ] Brain: add a real project → Ready, watcher live, search, gist injection.
- [ ] Note what the laptop needs pre-installed (Claude Code CLI, node, git) and
      whether the app degrades gracefully without them — no fresh-machine test
      has ever been run.
- [ ] Watch for the **one unexplained clean exit** (exit 0, no data loss) seen once
      during 2026-07-10 GUI validation and never reproduced.
- [ ] Only after D1=public: install an older build, cut a `v0.9.1` test tag,
      confirm the updater verifies the new pubkey end-to-end.

## E. Explicitly out of scope for v0.9.0

Code-signing cert (SignPath), beta/stable dual channel, ADR-015 embeddings,
ADR-012 symbol graph, ADR-013 stored history.
