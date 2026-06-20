# ADR-002: Five workspace additions (pane dropdown, pane colors, smart links, auto-retry, graph polish)

Status: Accepted (implemented, statically verified — pending GUI relaunch verification) — 2026-06-19

Built on branch `overnight/agents-tasks-persistence-2026-06-16`. No new npm/cargo dependencies. `pnpm check-types` / `pnpm lint` / `pnpm test` (285 real + 29 new) / `cargo check` + `cargo test retry_detect` (14 new) all green. Behavior still needs a Tauri relaunch to confirm.

## Context

Kosta requested five independent additions to the Terax fork. They span four subsystems, so the work was partitioned by file ownership and built by parallel agents + one integration pass, so no two agents touched the same file (no worktrees, no merge conflicts). Reserved shared files (`settings/store.ts`, `App.tsx`, `WorkspaceSurface.tsx`, `TerminalStack.tsx`, settings UI, spaces boot/persist hooks) were edited only in the integration pass.

## Decision

**1. Per-pane split dropdown + 4-direction splits.** The pane header (`PaneTreeView.tsx` `PaneHeader`) now **always renders** (a lone terminal previously had no header), and has a far-right `MoreHorizontal` dropdown: pane type {Terminal, Note, Task} × direction {Left, Right, Top, Bottom}. Keybindings (Mod+D etc.) are untouched — the dropdown sits on top. `insertBeside` (`panes.ts`) gained a before-insert path so Left/Top work; `SplitSide` + pure `sideToSplit(side) → {dir, before}` map Right=row/after, Bottom=col/after, Left=row/before, Top=col/before. Threaded `onSplit(leafId, type, side)` App → WorkspaceSurface → TerminalStack → PaneTreeView; `App.handlePaneSplit` focuses the clicked leaf first, then routes to `splitActivePane`/`addNotePane`/`addTasksPane`.

**2. Per-pane title colors + per-type defaults.** Fixed the bug where `renamePane` dropped `color` (now preserved). Added `setPaneColor` (no-op on locked) + a color picker in the dropdown. Three settings prefs `paneColorTerminal` `#9aa5b1` / `paneColorNotes` `#d8a657` (moved off `#f5b042` which collided with waiting-orange `#f97316`) / `paneColorTask` `#5fb8a8`, each a native `<input type=color>` in GeneralSection. Per-pane name+color now persist across restart via `serialize.ts` (optional `PaneTitleReader`/`PaneTitleSeeder`, locked panes excluded). The title-color resolution lives in `PaneHeader`: `titleColor = entry.color ?? defaultByType`; the **dot** keeps `entry.color` only, so its focus cue (primary when focused) survives. Empty-string labels fall back to the cwd basename (`||` not `??`) so a color-only terminal entry doesn't blank the header.

**3. Smart clickable/copyable output + selection visibility.** Selection was near-invisible (theme `selectionBackground` at 0.16–0.25 alpha) — bumped to ~0.40–0.50 across all 10 presets + added `selectionForeground`/`selectionInactiveBackground` (terminalTheme/tokens/applyTheme/types/validateTheme). New pure `linkDetect.ts` (+ tests) feeds a `registerLinkProvider` wired in `useTerminalSession`'s `registerOsc` (per-leaf disposal, sees `lastCwd`). **Paths** → `revealItemInDir` (reveal, never exec); **secrets** (tight allowlist: JWT, long hex, UUID, `ghp_`/`github_pat_`/`sk-`/`AKIA`/`xox*`/`glpat-`) → clipboard + Sonner toast. Ctrl/Cmd+click activation (consistent with URLs), distinct hover color copy-vs-open, hovered-line-only for perf. Existing WebLinksAddon (http/https) untouched. Gated by `smartLinksEnabled` pref (default on).

**4. claude-auto-retry, ported (not wrapped).** The upstream project is a tmux wrapper (capture-pane every 5s → regex limit message → parse reset time → send-keys "continue"). tmux doesn't fit Terax (Windows; Terax owns its PTYs), so the **logic** was ported and the transport dropped. Rust `retry_detect.rs` (pure, ANSI-strip, latched one-shot, scoped to armed-claude sessions, `limit`/`usage` early-out, 14 unit tests) rides the existing per-session reader loop in `session.rs` and emits `terax:retry-signal {id, resetEpochMs}`. JS `RetryBridge.tsx` (mirrors `AgentNotificationsBridge`) resolves pty→leaf, and per-leaf `retryStore.ts` schedules `setTimeout` to resetEpoch +60s then `submitToLeaf(leafId, "Continue where you left off. The previous attempt was rate limited.")`. **Per-tab is automatic** (detection is per-session); independent state+timer per leaf → 3 concurrent rate-limited terminals retry independently. Cap 3 retries, cancel on `exited`. Global default `autoRetryEnabled` (off) + per-tab toggle in AgentDock. Pending retry is in-memory only (lost on app restart — acceptable v1).

**5. Graph (agent topology) visual redesign.** `layout()`/pan/zoom/fitView untouched. Nodes went from saturated candy-discs + white glyphs to the `AgentDock` idiom: monochrome `var(--card)` chip, status color in a thin 1px rim, role line-icon at foreground, soft glow only on active states, bottom-right pulsing pip; dropped the redundant uppercase status line (moved to tooltip; `label` footprint 22→16 for fitView). Edges: hard-coded slate `<line>` → themed quadratic `<path>` with marching dashes (`terax-flow`, previously dead CSS) on active edges only. Background: primary radial gradient → faint graph-paper dots that pan with content. Director is the filled-`primary` hub anchor. Amber/blue/green/red semantics kept; no new deps/keyframes; `globals.css` untouched by this feature.

## Consequences

- 36 files changed (~970 insertions) + 6 new files: `retry_detect.rs`, `RetryBridge.tsx`, `retryStore.ts`, `linkDetect.ts`(+test), `panes.test.ts`.
- One real type fix during verify: `Reload01Icon` → `ReloadIcon` (the former isn't exported by the installed hugeicons).
- `main` still untouched; everything sits on the overnight branch alongside the ADR-001 work, none of which is committed yet.
- **Needs GUI relaunch to verify:** dropdown split behavior + correct leaf/direction; note/task tinting + live settings updates; smart-link click-to-open/copy; auto-retry detection/timing against a real Claude rate-limit banner; persistence round-trip across restart; graph visuals.
- Pre-existing non-regressions (do not chase): `eager-budget.test.ts` (env), the Windows symlink Rust test, `src/lib.rs:84` unused-var warning.
</content>
</invoke>
