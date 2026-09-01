---
title: terax-workspace — memory index
created: "2026-06-17T19:02:09.000Z"
updated: "2026-06-17T19:02:09.000Z"
---

# terax-workspace — memory index

Retrieval entry point for the Terax fork. Read this first; open linked files on demand.

## Purpose
A fork of `crynta/terax-ai` (a terminal-first, local-first AI-native dev workspace on
**Tauri 2 + Rust + React 19**) that evolves it into a **unified multi-agent workspace** —
adding planning boards, notes, tasks, and agent orchestration on top of the base terminal.

## Current status
In-progress fork (base Terax **v0.8.0**). Active branch
**`overnight/agents-tasks-persistence-2026-06-16`**; `main` left untouched. The four
ADR-001 threads are coded and type/test-verified but **not yet runtime-GUI-verified**;
recent commits pushed the agent topology graph (constellation/forest layout, pan/zoom) and
pane-split work further. On 2026-06-19 five more additions (ADR-002) were built on the same
branch (pane split-dropdown + 4-way splits, per-pane/per-type title colors, smart
clickable/copyable terminal output + selection-visibility fix, ported claude-auto-retry,
graph visual redesign) — static checks all green, **GUI verification still pending**. Nothing
on the branch is committed yet.

**2026-06-20 (overnight):** the fork is being rebranded **Terax → Koden** (KOsta
waDENfalk; "koden" = "the code"). This session shipped terminal/agent UX (history
search + "Find in terminal", scrollback Claude-turn capture, single-sidebar default,
grid launcher, tab/pane context menus, AGENTS group/filter, hover scrollbar), fixed
the GLM subagent-visibility bug (`AgentBusBridge` now recovers subagents from corrupt
`agent-bus.jsonl` by `tool_use_id`), cut the "Ask Terax" popup (gesture kept), and
executed the Koden **identity rename** (all user-facing strings → "Koden"; bundle id +
every runtime contract KEPT as the internal `terax` codename so user data is not
orphaned; crynta attribution preserved). The auto-updater is repointed at `MrOlof/koden`
behind a default-off `autoUpdateCheck` pref (no upstream footgun). All static-verified
(tsc + 360 vitest). HELD for Kosta: bundle-id change (resets appdata), minting the
minisign key + creating the repo + publishing, contract migration, cutting Whisper/Mod+J.

**2026-07-06 (overnight):** the **ADR-010 brain fix plan was fully EXECUTED** on
`feat/koden-brain` — 8 checkpoint commits `f989238..3036916` (28 files, +2608/−299),
each cluster adversarially verified (correctness + regression lenses) before commit.
Covers all 6 ordered clusters, the remaining HIGHs/cheap MEDs outside the order
(AST def-noise scope-anchoring, HNSW upsert-replace, Windows case-fold resolve,
add/remove_project hardening, BrainPane poll guard), and the new **ADR-011** gist
upgrades (known-unknowns section + per-claim freshness labels, cache-stability
preserved). `SCHEMA_VERSION` 9→11. Sweep green: cargo lib 342 passed, brain_sandbox
48/48, tsc clean, vitest 1090/1090 (only the two known pre-existing env failures).
Deferred: the perf pair (temporal-boost scan, rebuild_edges O(project)); GUI/real-run
validation for the whole branch is still pending. NOT pushed anywhere.

**2026-07-06 (later):** 9 behavioral simulations (ephemeral `sim_*.rs` fixtures driving
the REAL brain end-to-end, every scenario with a negative control) — **all 9 "works",
zero contract failures**; adjudicated *works-with-deviations*: mechanism proven, scale
unproven. Honest gaps to close before calling §12.1 fully satisfied: (1) header-destroyed
sqlite corruption LOSES canonical tables loudly (salvage only works on attachable
corruption — reframe the criterion as "loud failure, not silent loss" or add a
salvage-friendlier path); (2) inline high-entropy secrets verified absent from GIST
output but not asserted absent from the FTS index layer; (3) the Librarian fail-streak
cap has no public test seam (unit-tested only); (4) everything ran on tiny fixtures —
no thousands-of-files stress of the 5-15s SLA / never-freeze claim; (5) gitignore
honoring requires a `.git` marker (`require_git(true)`) — non-git dirs don't get
.gitignore semantics (base denylist still applies).

**2026-07-06 (evening) — hardening round DONE** (closes sim gaps 2/3/5 above; resumed
post-reboot from `RESUME-hardening-2026-07-06.md`, now deleted). Four commits
`a3ef629..1cf968f` (+1228/−40), each fix adversarially verified before commit:
- `a3ef629` + `0d9159e` — **inline-secret leak closed at the index layer**: redaction
  chokepoint before tokenize/FTS insert (worker.rs, shared by walk + watcher) + detector
  (d) rewritten to wholesale between-marker PEM redaction (`pem_state_after_line`
  left-to-right folding, `PEM_BLOCK_LINE_CAP=1024` restarted per block — the same-line
  `END----------BEGIN` junction no longer accumulates the run and leaks the second
  body). Permanent index-layer regression test `tests/secret_index.rs`. Residual gaps
  documented in the module doc (single-line `\n`-escaped literal, cap-exceeding armors,
  split BEGIN atom).
- `b114704` — **Librarian fail-streak seam**: round-decision core extracted to
  `librarian_round_step` (pure code motion, production delegates — no drift copy);
  `tests/librarian_rounds.rs` proves streak→cap→park, re-arm on new content (streak
  zeroed only by a successful round — actual ADR-010 cluster-5 semantics), transient vs
  persistent classification.
- `1cf968f` — **.gitignore honored in non-git roots**: `require_git(false)` +
  `parents(false)` (project-bounded; subdir-of-a-repo no longer inherits repo-root
  .gitignore — documented ceiling), `.kodenignore` was already wired; watcher per-file
  gate `walk::is_ignored_file` + dir-event `walk_files_under` now agree with the full
  walk (closes the watcher-index/full-pass-prune oscillation). Flagged out of scope:
  fs/tree.rs + fs/grep.rs + fs/search.rs (UI walkers) still `require_git(true)`.
Sweep green (cargo lib 362/363 — only the known symlink-privilege failure; brain_sandbox
48/48; new targets pass). **Release-build SLA measured: 29.1 s first-index / 1592 files
/ 21.4 MB db (debug was 61 s); searches 2–4 ms. OVER the §12.1 5–15 s target ~2×** —
open verdict settled as over-SLA; next lever is the deferred perf pair (temporal-boost
scan, rebuild_edges) + per-file hashing parallelism, not measurement. GUI/real-run
validation still HELD (Kosta's call). NOT pushed.

**2026-07-07 (overnight, Kosta proxying) — NorrGit-parity steal COMPLETE.** Compared the
brain against `Beefcapone/norrgit` (in-house Node symbol-graph MCP; README pulled via gh)
and adopted everything that fits the file-granular architecture, in 3 verified rounds,
9 commits `454fecc..6f7207f` (each fix→adversarial-verify→commit):
- `454fecc`+`0f95577` — **impact parity**: depth-annotated bidirectional BFS
  (upstream/downstream/both), max_results 200 w/ truncated/result_total, exclude_tests,
  deterministic incl. at the truncation boundary.
- `8268395` — **precision gate** `tests/brain_precision.rs`: 40-file hand-labeled corpus,
  23 queries w/ negative controls, measured floors (standing gate, runs in default sweep).
- `14bc48a` — **detect_changes**: git diff (staged/working/both) → affected indexed files
  + first-degree dependents; non-git roots soft-skip.
- `ad340f9` — **plan_context**: one-call bundle (search+changes+impact) w/ advisories.
- `c643600` — **token-coverage re-rank**: macro P@10 **0.53→0.96** (camel class 0.29→1.00),
  R@10 0.96 + pollution 0.05 unchanged; gate ratio 0.7 chosen over score-maximal 0.8 to
  keep the scoreboard discriminating. Gist key rotated via SCHEMA_VERSION 11→12.
- `0921062` — **perf pair closed**: temporal boost = bounded probes (bit-identical to old
  scan by property test); edge relink delta-proportional w/ full==incremental convergence test.
- `fa86298` — **parallel first-index**: bounded-channel fan-out, deterministic apply order,
  single-writer preserved. **Release SLA re-measured: 29.1 s → 2.8 s / 1598 files** —
  §12.1 (5–15 s) now comfortably met. Searches 3.5–6 ms.
- `6f7207f` — **git-backed hotspots + changed_between** (ADR-013's recommended first step):
  read-only git w/ flag-injection shape gate, `tests/brain_temporal.rs`.
Deliberately NOT adopted (await Kosta): symbol-granular graph (**ADR-012 Proposed**),
stored bitemporal history (**ADR-013 Proposed**), framework intelligence (route/ORM
extractors), embeddings-on (HNSW is default-off by design), cypher passthrough (secrets-gate
bypass risk). NorrGit's own gap noted for its repo: no secrets gate. All standing gates
green at `6f7207f`; GUI validation still HELD; nothing pushed.

**2026-07-07 (day) — app-integrated brain GAUNTLET run + fixed.** 12 live-context-engine
scenarios at the real runtime boundary (separate brain_cli process + real worker/watcher/
commands-layer fns; honesty-reported boundaries per scenario). **11/12 pass, 1 blocked**
(S11 real paid Librarian round: no key in OS keyring `koden-ai`/`anthropic-api-key` — the
harness `brain_cli reflect-live` is built and runs unchanged once a key exists; NoKey
pre-flight demonstrated live, $0 spent). Crash-recovery (S10) proven at the process level:
hard-kill mid-index → WAL survives, relaunch converges byte-idempotent, no ghosts; corrupt
header → ADR-010 rename-aside fires. 5 defects found → fixed → adversarially verified →
committed (`b7c93fa..ca4ce3a`): **secret-intent-echoed-to-gist** (MED — gist intent
excerpt bypassed redaction; the exact leak class the gauntlet targeted),
**claude-worktrees-indexed** (MED — 61% of the real-repo index was `.claude/worktrees`
duplicate scratch, silently inflating impact/gist evidence AND the historical 1592-file
SLA numbers → real main-tree corpus is ~630 files), stale-note-labeled-current,
no-test-exclusion-in-gist-search, rust-imports-no-ast-dependents (Rust `use` extraction
+ SCHEMA_VERSION bump; Rust symbols now get AST dependents). Sweep: lib 393/1-known,
all 8 brain suites green, sims deleted. **Loose end: `examples/brain_cli.rs` +376/-2
uncommitted** (G6 process-boundary subcommands incl. `reflect-live` + the
open_with_recovery fix) — Kosta to accept or drop.

**2026-07-10 (day) — BRAIN VALIDATED IN THE LIVE APP + MERGED TO MAIN.** First-ever GUI
validation of the brain inside the running Koden app (CDP-driven, isolated throwaway
bundle id `app.mrolof.kdnval`, real data dir verified byte-identical before/after,
evidence PNGs in `.memory/brain-verification/`): **9/9 checklist items pass** — boot,
add-project→Ready, live watcher in-app, UI search, gist write + traversal guard, real
paid Librarian round via GUI config ($0.0004 total), budget meter, hard-kill with 4.35MB
dirty WAL → full recovery, clean consoles. 3 defects found → fixed → verified → committed
→ **re-validated live (R1–R4 all pass)**: `d44e3ed` D1 mid-session rescan now restores
Ready; `566d0f8` D3 memory view polls proposals live; `7f557f7` **D2 Approve now APPLIES**
(product ruling per Conductr provenance: create materializes a `.koden-memory` note,
archive flips status file+table, supersede wires both sides, update appends; apply runs
on the writer thread via a reply channel — adversarially verified, no-hang proven).
Stale `.claude/worktrees` removed (were tripling vitest + polluting the index).
**MERGED: main fast-forwarded `f00a360 → 7f557f7`** (121 files, ~36k insertions).
Post-merge sweep green (lib 419/420 known-only, tsc clean, vitest 370/370).
Loose ends: validator observed ONE unexplained clean exit (exit 0, zero data loss,
unreproduced — dev-mode fluke, watch for it); `feat/koden-svart` now rebases onto main;
OpenAI key still in keyring (rotate); ADR-015 embeddings still Kosta's call.

**2026-07-10→11 (overnight, proxy) — LIBRARIAN GAUNTLET + fixes + ADR verdicts.** Full
detail in `MORNING-REPORT-2026-07-11-librarian.md` (delete after reading). 3 verified legs,
21 scenarios, ~$0.08 real spend: lifecycle accuracy (8 real rounds, judge: recall 1.00 /
precision 1.00 / zero fabrications), concurrent 5-terminal load (search never blocks,
0 event loss), 7-session kill matrix (ledger exact after every kill). 7 commits
`10ba031..83f4ce7`: digest-pin persistence, **ADR-016 implemented** (LLM call off the
worker thread — round staleness 20.1 s→0.79 s, offload races fixed), **sidecar journal**
(canonical tail survives header-destroying corruption, re-spend refused after recovery),
UI-walker gitignore parity, brain_cli harness, **proposal-stream dedup** (judge's 2.4×
redundancy finding: Jaccard gate + pending-aware digest + named findings; live-confirmed —
restatements silent, contradictions still fire). ADR verdicts as proxy: 012 Deferred
(demand-driven), 013 stored-rejected-for-now, 016 Accepted+done, 015 Proposed (embeddings —
Kosta's dependency call). OpenAI key STILL in keyring (Kosta to rotate). Remaining: GUI
validation → merge; Svart branch rebase.

**2026-07-10 (later) — S11 LIVE paid Librarian round COMPLETE, EXIT=0** (`423bd45`+`f6dc786`;
Kosta supplied a throwaway OpenAI key, since deleted from keyring + to be revoked
server-side). The first real round immediately exposed a **latent cross-provider defect**:
the reflect prompt never STATED the output schema (field names/enums) — faithfully ported
from Conductr's reflect-llm.ts, whose chatJson doesn't state it either — so any model
invents keys → InvalidOutput → fail-streak parks the Librarian everywhere. Undetectable by
fakes (they emit schema-perfect JSON by construction). Fix `423bd45`: exact schema stated
in system_prompt(), adversarially verified (prompt⇄parser field-exact; escaping proven by
compiled render). Full live suite then green vs gpt-5.4-mini (~$0.0025 total, ledger
reconciled): GATE1 disabled / ROUND1 Ok w/ ground-truth-correct proposal / GATE2 unchanged
$0 / GATE3 over-budget $0 / GATE4 reject-sig round-trip / ROUND2 new-note re-fire Ok.
Learned en route: the reflect delta digest = memory notes + doctor findings, NOT code
content (a code-only change is correctly Unchanged). Carry-forwards (observations, not
defects): prompt's "no other keys" suppresses the optional `evidence` enrichment
detail_for would render; a pre-fix parked digest re-fires only on content change or a
manual round. `brain_cli set-key <provider>` (key via STDIN) added for future key loads.

**2026-07-10 — cleanup pair.** `7ddde42` commits the brain_cli G6 extension (accepted by
Kosta). `cff7c8e` = the last NorrGit micro-steal: empty search results in plan_context are
now EXPLAINED via `advisories` (`explain_empty_search_readonly`, closed vocabulary:
no-searchable-tokens / index-empty / matches-other-projects / no-token-match — one class
per agent next-move). Balance review 2026-07-10: encryption-at-rest REJECTED (OS FDE is
the right layer; index = derived data next to plaintext source), OKF adapter PARKED
(v0.1, pre-1.0; brain already IS the OKF+RAG two-layer pattern — notes+gist = curated
layer, FTS = retrieval layer). Remaining queue by decision: embeddings prototype
(measured gap: hard-concept recall 0.25), ship-the-branch/GUI validation, S11 keyring
key, sidecar journal, UI-walker require_git. ADR-012 demand-driven; ADR-013 stored
upgrade rejected-for-now.

**2026-08-31 (new laptop, NO .git on this copy — commit from a git machine):
notification levels + coalescing.** `agentNotificationMode` pref ("all" |
"smart" | "important", default **smart**) + `lib/coalesce.ts` (4 s window
batches calm kinds → "N agents finished — Tab1, Tab2"; attention/error always
immediate; bell records everything in every mode; "important" = finished/memory
→ bell only). Settings → General → Agents gets a "Notification level" select
under the existing switch. Touched: settings/store.ts, agents/lib/route.ts,
agents/lib/coalesce.ts(+test), GeneralSection.tsx. tsc clean, vitest 12/12
(coalesce+store), biome lint clean on new files (format check unusable here —
CRLF noise from the git-less MEGA copy). GUI not runtime-verified.

**2026-08-31 (evening) — ssh-space freeze root-caused + PTY main-thread hardening.**
Symptom: after adding the ai-server remote Space, "can't type anywhere, mouse
works". Root cause: `pty_write`/`pty_resize` were SYNC Tauri commands (main
thread); a ConPTY that wedged during the ssh spawn blocked them → all IPC dead
while the WebView2 DOM stayed alive. Evidence: ai-server sshd showed the probes
succeeding then silence; no server-side drops; 3 force-killed Koden boots in
25 min. Fixes (all uncommitted — git-less MEGA copy, commit from a git machine):
per-session writer thread + bounded channel so `pty_write` stays sync but can
never block (session.rs; DA replies routed through it too); `pty_resize` +
foreground checks async on the blocking pool; kill moved onto the detached
drop thread in `pty_close`/`pty_close_all`; pty-bridge now reaps the backend
Session on natural exit (was leaking a conhost per exited shell);
`ServerAliveInterval=15`+`CountMax=2` (~30s dead-link detection, was 90s);
`handleLeafExit` holds a non-zero-exit ssh pane with an Enter-to-reconnect
banner (layout survives drops) and backs off instead of respawn-looping when
a shell dies <5s after spawn (`holdLeafForRetry`/`leafExitedQuickly`).
Sweep green: cargo lib 554/1-known, vitest 643/643, tsc + clippy clean.
Unrelated but real: HQ's Tailscale is wedged (expired key, NoState) — Kosta
re-authing it; the ssh path itself is LAN (`ai-server` = 192.168.1.240).

**2026-09-01 (dogfood night close-out).** After cross-device shipped, the
same night added: **F2 manifest push** (`ca5cdd3`+`97e591c` — active ssh
Space mirrors tab names to `~/.koden/spaces/<tmuxKey>.json`; host-side
views label windows; fresh-tab "shell" placeholder mirrors the displayed
cwd name), **remote image paste M2.9** (`977b89d` — Ctrl+V image in an ssh
pane: Tauri clipboard readImage → canvas PNG → staged raw-body command →
ssh stdin to `~/.koden/paste/` → quoted path typed; hooked in
rendererPool's key handler because WebView2 never fires a browser paste
event there; live-verified by Kosta), and **auto-updater ACTIVATED**
(v0.11.1: the minisign pair minted 2026-07-31 in `_ClaudeSetup/secrets/`
matches the baked-in pubkey, repo is public, endpoint was right all
along; `autoUpdateCheck` defaults ON; `scripts/release-koden.ps1` =
one-command signed release incl. `git push origin main` — releases are
the ONE sanctioned push path). Companion product on the box:
`ai-server:~/Snorlax/Products/ai-server-dashboard` (v2.6, own git) —
faceplate, tiles, workspace w/ live pulses + peek + kill/update buttons,
devices, VM detail, roster w/ skills. M2.8 (remote agent signal via
TMUX_PANE hook) speced in KODEN-REMOTE.md, not built.

**2026-08-31 (final) — CROSS-DEVICE WORKSPACES LIVE (`5159c59` + `e90fc8d`).**
Two closing moves on top of F2-lean: (1) explicit close = kill (`5159c59`):
closing a tab/pane kills its tmux window (`ssh_tmux_kill_window`, always
confirmed via forceTerminalConfirm — local foreground check can't see remote),
deleting a Space kills the base session; accidents still resurrect via
adoption. (2) **path-keyed sessions** (`e90fc8d`): tmux session =
`koden-p<fnv1a(path)>-<tail>` (`pathTmuxKey` in tmuxKey.ts, frontend-only),
so every device connecting to the same host+path shares ONE live session and
adoption merges tabs both ways — Kosta's "same spaces despite device" done.
Delete-space therefore kills the workspace for ALL devices, by design.
Kosta's live session was migrated by hand (`tmux rename-session` →
`koden-pcnh0sv-home-snorlax`); older `koden-sp-*` strays on ai-server are
invisible to the new naming — kill manually when spotted. Known feel-ceiling
accepted by Kosta: fast output over tmux is frame-batched, not stream-smooth
(tmux repaints, inherent). Laptop installer staged at
`ai-server:~/koden-dist/`. HQ runs this build (installed 22:55).

**2026-08-31 (later) — M2.5 F2 lean + launcher liveness SHIPPED (`6ff03c5`).**
tmux IS the manifest: `ssh_tmux_windows` (ssh.rs) lists a Space's base-session
windows (name+command+path, one bounded probe, `sh -c` wrapped). On first
activation of an ssh+tmux Space, App adopts live windows no local pane owns —
`adoptTerminalTab` (useTabs) creates a background tab whose leaf is seeded
with the window's restore key BEFORE mount, so the spawn reattaches. Pure
logic + parity vectors in `spaces/lib/remoteSessions.ts(.test)` —
`windowNameForKey` mirrors Rust `tmux_window_name`; keep them in lockstep.
Launcher RECENT rows show "● N live sessions" via the same probe (cached
30s, async fill). Deferred: json manifest (custom titles/order), switcher
badges, re-reconcile without app restart, brain crate-split + headless
indexer (M2.7 A — next big rock).

**2026-08-31 (late) — M2.5 Feature 1 SHIPPED (`ab1ad5a`): per-pane tmux
windows.** With the Space tmux flag on (connect form now defaults it ON),
every pane owns a tmux window named `w-<restore key>` (`leafRestoreKey` —
restart-stable) inside base session `koden-<spaceId>`; each client attaches
via a grouped session (`-t base`, destroy-unattached) pinned to its window —
no pane/device mirroring, restored panes reattach 1:1, closing a pane
detaches (window keeps running). Ceilings: orphan windows after a pane is
closed locally (F2 manifest reconciliation will re-surface them), viewport
name `$$`-based (pid-reuse collision = one Enter-retry). F2 (host-side Space
manifest, cross-device tab discovery) NOT built — spec in
`To-Do\Workbench-Setup\KODEN-REMOTE.md` §M2.5. Committed on restored `.git`
(main `3d18407 → 35159df → ab1ad5a`); NOT pushed.

## Key decisions (`decisions/`)
- **ADR-011 — Gist known-unknowns + per-claim freshness labels** *(Accepted + implemented
  2026-07-06, commit `3036916`)*. Two adoptions from an external design review: the gist
  now states verified-absence explicitly ("no code hits / no memory notes", gated so an
  unready index stays freshness-only) and labels injected notes
  `[current]`/`[possibly-stale]`/`[historical(superseded)]` from cache-key-covered state
  only (supersession edges + anchor touches; `revalidate_after` excluded to preserve gist
  byte-identity — ceiling + upgrade path in the ADR). Gist schema v11.
- **ADR-010 — Brain module correctness review: confirmed findings + fix plan** *(Accepted
  2026-07-03; **EXECUTED overnight 2026-07-06** — all clusters fixed + verified, commits
  `6b955f3..3036916`, execution record in the ADR; perf pair still deferred)*. Full
  adversarial review of `src-tauri/src/modules/brain/`
  at `f989238` (10 dimensions, 2 refuters/finding): 48 confirmed / 3 disputed / 2 refuted /
  13 unverified findings. Fix order: (1) reconcile-delete data-loss cluster (`worker.rs:491`
  wipes a project on unreadable root), (2) watcher armed after warm walk + ignored `Rescan`
  flag, (3) `brain_write_gist` path traversal + `tier2` injection one-liners, (4) corrupt-cache
  rebuild path, (5) Librarian unbounded paid retry loop, (6) main-thread commands + perf pair.
  Verified-solid: FTS5 injection-proof by construction, compile-time single-writer, atomic
  migrations, faithful Conductr tokenizer port. The "monthly budget never resets" claim was
  REFUTED (cumulative cap is documented design). All detail in the ADR — self-contained.
- **ADR-009 — Brain Map rebuilt as "Koden Brain 3D" (Three.js)** *(Accepted 2026-06-24; 3D is
  the sole Brain Map, no 2D toggle)*. `BrainMapPane.tsx` is now a faithful port of the
  `Koden Brain 3D` design-handoff (real Three.js WebGL): brain neuron core (ported verbatim) +
  real projects as fibonacci-sphere lobes + files in tangent-plane recency shells (mtime bands)
  + GPU point-cloud shader + orbit camera + live agents (orbit hottest files) + `fs:changed`
  recent-edit glow + search/Highlight-24h/detail/feed. **Adds `three`** (~150KB gz). Dropped vs
  the interim 2D Observatory: blast-radius/risk/timeline-scrub (not in the 3D design). 2D work
  preserved in git (`73754af`, `42d2aa0`); 3D = `da8b8cd`. Needs WebGL + a visual-tuning pass.
- **ADR-008 — Autonomous Librarian: event-driven trigger + delta-gated reflect** *(Accepted
  2026-06-22; supersedes ADR-006's "manual, never on a timer")*. Reflect is now autonomous and
  EVENT-driven (not a count, not a clock): `worker.rs` marks a project dirty on watcher changes,
  and the 60-s Tick fires one delta-gated reflect when it's past a 5-min min-gap AND **either**
  gone quiet 3 min (idle-settle) **or** an AI session just **exited** there (`handle_agent`
  returns the project → boundary). `reflect_auto` blake3-hashes the digest and **skips the paid
  call ($0, `Unchanged`)** when nothing material changed. Budget ceiling = throttle (0 = off =
  `Disabled`); reflect only PROPOSES (curate stays manual). `reflect_once` = thin `prev=None`
  wrapper. Evolved same-day: count(20) → 15-min clock → event-driven. 119 lib + 39 sandbox green.
  Live GUI loop still unverified. Answers "how/when does she run?".
- **ADR-007 — Agent turn capture via the Koden bus (Claude + Codex)** *(Accepted; live-confirmed
  2026-06-22)*. `UserPromptSubmit` hooks append `{"cmd":"user-turn","id":<KODEN_SESSION>,"data":…}`
  to `~/.koden/director-bus.jsonl`; `AgentBusBridge` routes by pty id to the Inputs list. Hooks
  now install on **app startup** (so manual `cm`/`codex` get them, not just Koden-launched agents)
  and **migrate stale pre-rename TERAX hooks**. **Codex** added via `agent_codex.rs` (append-only
  to `~/.codex/config.toml`, POSIX `command` + Windows `.ps1`, capture-only, same bus line — zero
  frontend change). Inputs fix: bus turns are **marker-free** (registerMarker lines go -1 in a
  repainting TUI → only the first survived); bridge **primes to bus end** on mount (no replay of
  old runs). Caveats: `id:"1"` per-pty collision for concurrent agent panes (unverified); Codex
  gist injection deferred. Env/paths are `KODEN_*`/`~/.koden` now (older `TERAX_SESSION` text below is stale).
- **ADR-006 — Koden Brain native in-process architecture (Code + Brain)** *(Accepted founding architecture, 2026-06-20; CANONICAL; supersedes ADR-005/004 + Conductr ADR-033)*. Build the brain **NATIVE in Rust, in-process** — no Node, no subprocess, no MCP; Conductr is the idea source, not a dependency. One `src-tauri/src/modules/brain/` tree + one GUI-resident worker (poll.rs template) listening to `koden:agent-signal` + a recursive watcher. **Stack:** SQLite/FTS5 (behind a `SearchIndex` trait), tree-sitter (TS/JS+Rust v1) for a real AST graph (the upgrade over Conductr's regex), ported BM25+RRF+identifier tokenizer, blake3 freshness, default-OFF budgeted LLM reflect, deferred semantic. **Storage:** git-committed portable memory source + local rebuildable SQLite cache. **Phases:** P0 warm lexical brain (zero-token search) → P1 freshness + memory + wizard → P2 tree-sitter AST graph (XL differentiator) → P3 cache-stable gist injection (the unification payoff) → P4 budgeted reflect + crash-resume → P5 deferred semantic. Top risk = prompt-cache-stable gist.
- **ADR-005 — Koden Brain via Conductr stdio MCP child** *(SUPERSEDED 2026-06-20 by ADR-006)*. Wrapped Conductr's engine as a managed Node child; reversed to native Rust. History only.
- **ADR-004 — Conductr as Koden's upkeep/memory daemon** *(SUPERSEDED 2026-06-20 by ADR-005→006)*. Original "tokio task + `externalBin` sidecar" framing; history only.
- **ADR-001 — Multi-agent workspace feature direction** *(Proposed → partially implemented)*.
  One cross-cutting root cause: terminal→agent registration is best-effort and late, there
  is no per-pane identity, and three separate stores track agent state without a single
  source of truth. Plan, in order: crash-safe docs persistence → agent-visibility foundation
  (pre-register a placeholder agent per terminal leaf, inject `TERAX_SESSION=<leafId>`,
  generalize the subagent bus) → notification roll-up + app-level signal → Tasks tab →
  topology graph last. **Shipped:** crash-safe docs, Tasks tab, agent pre-registration,
  worst-wins tab roll-up + taskbar flash.
- **ADR-003 — Usage guard, retry fix, command minimap + ADR-002 iterations** *(Accepted;
  implemented + statically verified 2026-06-19 overnight, GUI/real-run pending)*. Proactive
  usage guard (Rust OAuth-usage poller + time-fallback + soft spawn-gate), reactive auto-retry
  FIXED for Claude Code v2.1.168 (modern banner + Windows TZ + Esc menu-dismiss), command
  minimap (OSC-133 tick strip), OKLCH readable pane colors, configurable smart-link categories,
  graph focus/lock, visible scrollbar, + a fake-claude sandbox harness (`scripts/`). No new deps.
- **ADR-002 — Five workspace additions** *(Accepted; implemented + statically verified,
  GUI-verification pending)*. Pane split-dropdown (always-on header; type×direction;
  `sideToSplit`/before-insert), per-pane title colors + per-type default prefs (renamePane
  color-loss bug fixed; persisted via serialize reader/seeder), smart link providers
  (paths→reveal, secrets→copy; selection-alpha fix), claude-auto-retry **ported** off tmux
  to a Rust `retry_detect` per-session detector + JS `RetryBridge`/`retryStore` (per-tab,
  cap 3), and the topology graph restyle to the AgentDock idiom. No new deps.
- The **orchestration store is the authoritative model** (driven by real user actions +
  terminal-agent link, persisted to `terax-orchestration.json`).
- Status color convention: **amber** = needs-input/waiting, **blue** = working,
  **green** = done/idle, **red** = error.

**2026-08-30 (Kosta-directed, 5 parallel worktree agents): CATE PARITY MERGED ON MAIN** (`c52052b`..WP5 merge; all five packages): ssh WorkspaceEnv through the system OpenSSH client + host picker + local-fs gate (WP1), worktree Spaces with `git_worktree_*` (WP4), the launcher tab "What do you want to do?" incl. open-folder-as-Space and remote connect (WP2), resume cards for the dormant `brain/resume` journal + scrollback restore across launches in `koden-scrollback.json` (WP3), the `koden` CLI for agents inside Koden terminals over a per-instance named pipe (WP5). Research: `cate-deep-dive-2026-08-30.md`; decision + ceilings: `decisions/ADR-022-cate-parity-remote-launcher-resume-worktrees-cli.md`. Gates green on main (vitest 587, cargo lib 516, clippy 0). NOT pushed, NOT runtime-verified in the GUI.

## Important files / docs
- `TERAX.md` — base Terax architecture (two-process Tauri model, PTY, AI subsystem); read first. `CLAUDE.md` and `AGENTS.md` both just point here.
- `WORKSPACE.md` — the definitive spec of what the fork adds (orchestration spine, Agent Dock, Topology, Flow Inspector, Director, persistence).
- `decisions/ADR-001-multi-agent-workspace-feature-direction.md` — design + shipped/deferred table + open verification.
- `feature-backlog.md` — 12 proposed (unbuilt) features with effort sizes.
- **`koden-overhaul-plan-2026-06-20.md`** — Koden rebrand + soft-update-channel + bloat execution plan (decisions-needed box up top).
- **`audit-verification-2026-06-20.md`** — verification of the two 2026-06-19 research baselines against the current tree (done / stale / flipped).
- **`koden-update-channel-setup.md`** — actionable checklist to stand up the signed Koden update feed (mint key, CI secrets, test release).
- `feature-research-2026-06-19.md`, `fork-rebrand-and-onboarding-2026-06-19.md` — original research baselines (now partly superseded by the two dated docs above).
- `ROADMAP.md`, `README.md`, `package.json` (stack source of truth).

## Retrieval hints
- **Use this memory for:** the why / what / order of the multi-agent fork, the orchestration
  architecture and its three-store fragmentation, source-file pointers, what shipped
  overnight vs. deferred, and the proposed backlog.
- **Do not use it for:** runtime-verified behavior (ADR-001 explicitly says the GUI was
  never verified) or upstream base-Terax internals (those live in `TERAX.md`).

## Open questions / known gaps
- Live-verify agent registration + Tasks persistence in the GUI (not yet done).
- Tasks keybinding (the default `Ctrl+Shift+T` is taken).
- In-panel prompt answering — deferred to v2.
- Global-hooks phase 2: per-pane `TERAX_SESSION` is injected and the subagent bus is now WIRED + resilient (`AgentBusBridge` + `subagentBus.ts` recover subagents from corrupt `agent-bus.jsonl` by `tool_use_id`). Remaining: the subagent-start hook in `~/.claude/settings.json` is non-atomic (reader recovers; the writer still corrupts on parallel spawns), the dual-installer drift (`agent.rs` still writes the legacy OSC-777/director-bus path + stale `OWNED_MARKERS`), and store unification.
- ~~`getAgentCommand()` drops `@args`~~ **FIXED 2026-06-20**: `getAgentCommandWithArgs()` swaps `cm`→`claude` when flags are present so `--append-system-prompt`/`--agents` survive; the plain no-arg launch is unchanged.
- Known non-regression test failures: `eager-budget.test.ts` (env), Rust `authorize_spawn_cwd_blocks_symlink_escape` (Windows symlink privilege).

**2026-08-31 (new laptop, NO .git on this copy — commit from a git machine):
notification levels + coalescing.** `agentNotificationMode` pref ("all" |
"smart" | "important", default **smart**) + `lib/coalesce.ts` (4 s window
batches calm kinds → "N agents finished — Tab1, Tab2"; attention/error always
immediate; bell records everything in every mode; "important" = finished/memory
→ bell only). Settings → General → Agents gets a "Notification level" select
under the existing switch. Touched: settings/store.ts, agents/lib/route.ts,
agents/lib/coalesce.ts(+test), GeneralSection.tsx. tsc clean, vitest 12/12
(coalesce+store), biome lint clean on new files (format check unusable here —
CRLF noise from the git-less MEGA copy). GUI not runtime-verified.

**2026-09-01 — Koden Buddy: character APPROVED + implementation plan DRAFTED
(awaiting Kosta's review; nothing in-app yet).** Character: a living terminal
block cursor in Svart spruce with a spruce sprout on his head (Kosta's
favorite detail); status via underline caret in Svart ANSI colors;
hollow-cursor sleep; fully procedural SVG+CSS, zero deps. Overnight recon +
full plan (architecture, mood engine, 3-tier popover, 4 phases, open
decisions): headline = ~90% of machinery exists (AiMiniWindow chat, adapters,
signal spine); 7 brain commands incl. `brain_plan_context` are Rust-complete
with zero frontend callers (Phase 0 = bindings, valuable standalone). Spec,
recon findings, plan-artifact link, and the open-decision list all in
`buddy-character-design-2026-09-01.md`. Open-LLM-VTuber = idea source only
(param rig, emotion tags), rejected as dependency.

