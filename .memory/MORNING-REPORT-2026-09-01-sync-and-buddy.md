# Morning report — 2026-09-01 overnight (delete after reading)

Two deliverables tonight. Coffee first.

## 1. Buddy implementation plan (review, don't merge — it's a document)

https://claude.ai/code/artifact/bb0b9fc6-4d4e-4f11-890b-6058e788a01d

Full blueprint grounded in a 3-agent code recon. Headline: ~90% of the
machinery exists (AiMiniWindow chat + tools, signal spine, adapters); the
buddy is a presence layer, ~4-6 days, zero deps, zero Rust. Six decisions
wait for you in section 06 (name, theme-follow, default-on, corner,
important-mode celebrations, ship Phase 0 standalone). Character lab:
https://claude.ai/code/artifact/d6bbe569-92b6-4053-9abb-176c12aa9a28

## 2. Cross-machine workspace sync — BUILT (branch `feat/workspace-sync-2026-09-01`)

Your "notes gone on the laptop" gap, fixed per your call: everything syncs
(docs + spaces + tab layout). ADR-023 has the full design; two commits:

- `sync: cross-machine workspace sync` — the feature (12 new files in
  `src/modules/sync/`, seams into spaces/docs/settings/statusbar/App).
- `sync: fix 10 confirmed review findings` — an adversarial /code-review at
  high found 10 real defects (worst: layout merge degrading to "last boot
  wins"; mid-session pushes marking remote layout consumed so boot never
  adopted it; a pre-hydration pull that could roll back newer notes). All
  fixed + tested; revised invariants recorded in the ADR.

How it works in one breath: ai-server holds the canonical state under
`~/.koden/spaces/sync-*` manifests (rides the existing atomic ssh commands,
zero new Rust); notes/tasks/boards merge live per-entry LWW; spaces merge on
a new contentUpdatedAt (a visit can't beat a rename); tab layouts adopt at
boot only, per-space LWW on new stateMeta stamps; deletes tombstone; worktree
Spaces stay machine-local; paths rewrite through a wire token.

**Sweep**: tsc clean · vitest 685/685 (32 new) · biome clean · no Rust
touched (WinHands VM was running all night — no cargo, none needed).

### To turn it on (per machine, it ships OFF)

1. Settings → General → Sync → enable. Host: `ai-server` (default).
2. Set "Tree root on this machine": HQ `C:/Users/Snorlax/Snorlax`,
   laptop/ai-server `/home/snorlax/Snorlax`.
3. Needs key-auth ssh to ai-server in known_hosts (HQ and laptop both have it).

### Held for you (the honest list)

- **GUI verification** per house convention — everything is static-verified
  only. Suggested live test: enable on two machines, write a note on A,
  focus B (adopts within ~30 s); restructure splits on A, restart B (adopts
  at boot).
- Branch NOT merged to main, nothing pushed anywhere (house rule).
- Ceilings (in ADR-023): layout adopts at boot only; space reorder and
  activeId don't sync; scrollback doesn't sync; clock-skew LWW assumption;
  tombstone TTL 90 days; `sync-*` rides the spaces-manifest namespace until
  a small Rust follow-up gives it `~/.koden/sync/`.

### Your recovered HQ notes (from the incident that started this)

Pulled off SNORLAX-HQ via ssh 2026-09-01; they live in HQ's
`%APPDATA%\app.mrolof.koden\koden-workspace-docs.json` and will flow to every
machine once sync is on there. Content of the two that matter:

**Note 1 (Koden ideas):** Clippy-vibe animated character helper in Koden ·
Skills/MCP to add to ai-server · workspaces in left pane can't be reordered
by drag-n-drop · sub-tabs inside a workspace (e.g. Nordomatic Group →
Intune / Identity) because one tab can only fit so many splits · WinHands
VMs: want to SEE the desktop sometimes for manual printscr/tests · get
github.com/chaseai-yt/claudex-loop onto ai-server.

**Note 2 (Voice):** own Android app to talk to agents remotely — ongoing
conversations with notifications + read-aloud summaries, beyond Claude's
text-only app.

(Also on HQ: a July work-tickets note, two test notes, empty task lists.)
