# Morning report — hands-free voice, complete (overnight 2026-07-12 → 13)

**TLDR: the full voice flow you asked for is built, hardened, and PROVEN
end-to-end against a synthetic microphone — including with your own Ctrl+Space
rebind. Clean, headless, professional. Branch tip has it all; tests at
495/495.**

## The flow, as it works this morning

- **Tap `Ctrl+Space`** (your rebind — every tooltip tracked it automatically):
  a small floating pill appears bottom-center — spruce waveform, mono label,
  "tap Ctrl Space to send". **The chat window does not open.** Speak; pause as
  long as you like — silence never cuts a take. **Tap again** → pill flips to
  Transcribing → the Librarian works headless → substantive replies arrive as
  one compact toast (Open → chat); action-only turns stay quiet (the activity
  toasts already narrate). × on the pill discards.
- **Hold** the key = classic push-to-talk. **Header mic click** = the
  always-on session: listens immediately, re-arms after every turn, Esc tiers
  (discard take → end session → close window). Pulse states make a hot mic
  unambiguous.
- **Approvals can't deadlock**: the ONE case where voice opens the chat window
  is a pending tool-approval card — the moment that genuinely needs you.
- Voice drives everything from the week: terminals by name, layouts, tasks,
  notes, memory — with the tier system intact (hands-free arming in settings
  remains the only approval waiver, and the model still can't touch it).

## What the verifiers caught before you ever saw it

1. Cross-lane pref-key mismatch hidden behind a type cast — the hands-free
   voice loop would have shipped silently dead (fail-safe, but dead).
2. Session-end during the OS mic-permission prompt → hot mic recording with
   the toggle showing off. Fixed with unconditional cancel.
3. Rapid toggling → two concurrent getUserMedia, first stream's track leaked
   (stuck OS mic indicator until app exit). Fixed with a per-attempt epoch.
4. My own panel-close fix would have instantly killed headless takes had the
   HUD lane not made it transition-scoped (the flagged critical interaction —
   now regression-tested).
5. Session-switch mid-voice-turn flashing "done" + toasting a mid-stream
   partial. Settle guard added.
6. A double "Koden finished" toast on headless completions. Suppressed.

## The E2E proof (`.memory/svart-verification/voice-*.png`)

App relaunched with a fake media device; CDP drove the real pipeline: tap →
HUD listening with hint, mini window closed (headless ✓) → 4-second take held
with zero silence cutoff (Wispr semantics ✓) → tap → Transcribing pill (a real
Whisper-1 call on your key ✓) → clean settle. First run "failed" because it
tapped the OLD default hotkey — you had rebound to Ctrl+Space and the whole
stack (listener + tooltips) had followed: the rebind system proven by
accident.

## State

Commits tonight: `79ace56` (session + mic hardening) → `44f86a9` (Wispr take
semantics + panel-close delivery) → `f5b14eb` (headless HUD + reply routing +
settle guard). tsc clean · vitest **495/495** (44 voice tests) · fresh signed
installer building as this report is written. ADR-017 carries the full voice
contract (headless, tiers, orthogonality). Voice still requires your OpenAI
key (whisper-1); local STT remains the designed v2.

Reload the window (or restart dev) and tap Ctrl+Space. Godmorgon. 🌲
