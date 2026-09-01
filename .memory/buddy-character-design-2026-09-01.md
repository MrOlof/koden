---
title: Koden Buddy — character design (APPROVED)
created: "2026-09-01"
status: design approved by Kosta 2026-09-01; functionality plan = next step, nothing implemented in-app yet
---

# Koden Buddy — character design

An animated workspace companion (Clippy done right) living in a corner of the
Koden workspace. **Design locked 2026-09-01**; functionality intentionally
deferred until after the character work (Kosta's call).

Interactive design lab (living spec — all states clickable, corner-size preview):
https://claude.ai/code/artifact/d6bbe569-92b6-4053-9abb-176c12aa9a28
Local source copy of the lab page can be regenerated from the artifact.

## Concept

**He is the terminal block cursor, alive.** Not a mascot dropped into Koden —
grown from it. Body is Svart spruce (`#5b8a6f`, same as `--terminal-cursor` in
`koden-default.ts`), silhouette is a hand-drawn asymmetric cursor-blob (never a
hard rectangle: uneven shoulders, slightly melty spread base). On his head: a
small **spruce sprout** (two needle leaves on a bent stem, `#6b9a7d`/`#7fae8f`)
— Svart's "single muted spruce wire" made literal. Kosta specifically likes the
sprout — it stays.

An early iteration copied a generic cute yellow blob from a reference
screenshot; **rejected** — must match Koden's style, not generic mascot style.

## Visual rules

- Body **never changes color** — always spruce, recognizable in peripheral vision.
- **Status lives in an underline caret** beneath him (cursor underline-style),
  in Svart ANSI colors: `#cfa964` needs-input · `#7d9fc7` working · `#9ac4a8`
  done · `#d97a72` error. Face = mood, underline = state.
- Eyes: vertical dark pills (`#0a0b0b`) with small light glints. Cheek shading
  is dark spruce at 12% opacity — no candy pink, Svart restraint throughout.
- Faint body glow = whisper of the K logo's edge light.
- Fully procedural: one SVG + CSS springs, **zero dependencies, no sprite
  assets, no Live2D/Rive**.

## States (mirror Koden's status convention: amber/blue/green/red)

| State | Behavior |
|---|---|
| Idle | Gentle sway loop, breathing, random blinks, eyes follow cursor; every 8–16 s a soft caret-blink (twice, `steps(1)`, then still); no underline |
| Working | Typing-rhythm bob, eyes darting across the work, focused brows, blue underline sweeps like a scanner |
| Needs you | Arm pops out + waves, hops, raised brows, fast-blinking amber underline — **the only state allowed to loop for attention** |
| Done | Quick wave, two bouncy hops, grin, solid green underline; auto-settles to idle ~3 s |
| Error | Wide eyes, one short shiver, red underline flashes ×6, then holds still (alarm, not panic; holds until acknowledged) |
| Sleeping | **Hollow cursor** (unfocused-terminal outline render), closed eyes, slow breathing, slight lean, mono-font z's |

## Motion rules

- Never fully still: every state has a low-amplitude ambient loop (sway /
  typing bob / breath) under one-shot spring squashes anchored at the baseline.
- Sprout sways on its own spring, tempo tracks mood (lazy idle → fast busy →
  near-still asleep). Follow-through is what sells him as alive.
- Terminal blinks are `steps(1)`, never fades. Transitions ≤ ~420 ms, spring
  easing. Click = jelly wobble (damped multi-oscillation). State switches get a
  quick anticipation squash.
- Respects `prefers-reduced-motion` (loops off, status colors remain).
- Reads at 56 px (verified in the lab's corner-size preview) — that's roughly
  his in-app size.

## Implementation plan (2026-09-01, overnight — AWAITING KOSTA'S REVIEW)

Full visual blueprint (architecture diagram, mood state machine, popover
wireframe, phased build, open decisions):
https://claude.ai/code/artifact/bb0b9fc6-4d4e-4f11-890b-6058e788a01d

Grounded in a 3-agent code recon (orchestration/notifications, brain/CLI
surface, UI shell). Headline findings the plan is built on:

- **~90% of the machinery exists.** `AiMiniWindow` (Librarian chat, Mod+I,
  ADR-017) already floats with tools for tabs/panes/notes/tasks + approval
  gates. Buddy = presence layer, NOT a new agent stack; tier-3 chat just
  opens the existing chat (one spend surface).
- **7 brain commands are Rust-complete but have zero frontend callers**:
  `brain_plan_context` (the "what am I working on" bundle), `brain_code_impact`,
  `brain_detect_changes`, `brain_hotspots`, `brain_changed_between`,
  `brain_get_symbol`, `brain_write_gist`. Phase 0 = wrap them in
  `brain/lib/bindings.ts` — valuable standalone.
- Mood engine: new `moodReducer.ts` — first app-wide worst-wins over the full
  9-value `AgentStatus` (only tab-level 4-value `worseTabStatus` exists).
  Gotchas: orchestration store is session-scoped (empty at boot → buddy wakes
  from live signals); `setStatus` no-ops on unchanged status (twitches
  subscribe to `koden:agent-signal` directly); finished maps to `idle` in
  OrchestrationActivityBridge but `ready` in AgentBusBridge (reducer treats
  both as calm rather than touching the bridges).
- Mount: `App.tsx` overlay block (~2806) next to VoiceHud/Toaster —
  VoiceHud is the pattern to clone (fixed z-50, pointer-events dance,
  usePresence; Framer Motion is GONE despite KODEN.md claiming otherwise).
  Bottom-LEFT (sonner toasts own bottom-right). CSS keyframes only, zero
  deps (540KB-gzip eager budget). Sprout doubles as Librarian activity
  indicator (same signal as statusbar BrainActivitySegment).
- Buddy volume obeys `agentNotificationMode`; reuse `createCoalescer`
  (4s CALM_WINDOW_MS) for batched celebrations; needs-you clears on
  focusing the waiting agent's pane (leafId), like tab pills.
- koden CLI deliberately NOT in the loop (no notes/tasks/settings verbs,
  WSL dead zone) — frontend stores are the path.
- Phases: 0 bindings (S) → 1 character+mood (M) → 2 popover tiers 1-2 (M)
  → 3 Librarian handoff (S). Parked: embedded chat, spend metering
  (pre-existing ADR-017 gap), drag position, emotion tags, voice.
- Open decisions for Kosta (in the artifact §06): name (rec Klippan),
  theme-follow via `--terminal-cursor` (rec yes), default-on (rec yes),
  corner (rec bottom-left), important-mode celebrations (rec quiet flash),
  ship Phase 0 standalone (rec yes).

## Open items / carry-forwards for the functionality plan

- **Name undecided.** "Klippan" (Clippy pun + Swedish "the rock") proposed, not confirmed.
- Functionality sketch from the ideation session (to be planned properly):
  buddy as the embodiment of the notification/status roll-up (worst-wins,
  respects `agentNotificationMode`); click → popover with (1) deterministic
  quick actions via the commands layer, (2) free local brain Q&A
  (search/plan_context/gist), (3) paid conversational tier that can act, with
  propose-then-act discipline. Placement: corner above status bar, hideable pref.
- **Reference, not dependency:** Open-LLM-VTuber (github.com/Open-LLM-VTuber/Open-LLM-VTuber)
  judged excessive to adopt (Python sidecar + Live2D — contradicts ADR-006
  native-in-process + bloat discipline) but worth stealing ideas from:
  continuous **parameter rig** (eye-open/gaze/lean params with layered idle
  noise) instead of discrete CSS states; **emotion tags in LLM output → buddy
  expressions** for the chat tier; their push-to-talk/interruption UX if voice
  is ever wanted (park for v1).
- Theming question for implementation: body color could read
  `--terminal-cursor` so the buddy re-skins with non-Svart themes, or stay
  spruce as brand. Decide at build time.
