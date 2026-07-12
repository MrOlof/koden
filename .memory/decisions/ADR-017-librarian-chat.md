# ADR-017 — The chat is the Librarian

- **Status:** Accepted (Kosta 2026-07-11: "the AI tab can be reworked as Librarian…
  I like the idea of being able to talk to ur librarian"); implemented same day,
  commit `5a960e0` on `feat/koden-svart`.
- **Partially superseded by ADR-018** (autonomous curation): the ENGINE now
  applies memory changes itself (snapshot-undo, revertible) instead of
  propose-only — but this ADR's CHAT constraint is unchanged and still enforced:
  the chat toolset stays read-only (no curation/revert/mode tools).
- **Supersedes:** the generic "Ask Koden anything" chat identity inherited from Terax
  (persona grid, snippets authoring, five built-in personas).

## Decision

Koden's built-in chat stops being a generic coding assistant — real Claude Code /
Codex agents in terminal panes own that job — and becomes **the Librarian**: the
same entity that curates the Brain's project memory, now conversational. You ask it
about your projects; it answers grounded in the brain index and memory notes.

## Shape

- **One built-in persona** `builtin:librarian` (`ai/lib/agents.ts`). Memory-grounded
  instructions: answer from tools, cite the note/file a claim came from, say plainly
  when memory has nothing, suggest — never write — memory updates. `loadAgents`
  falls back to the Librarian when a persisted `activeId` points at a removed
  builtin; custom agents in `koden-ai-agents.json` still load.
- **Brain tools** (`ai/tools/brain.ts`, main chat toolset only): read-only
  `brain_search` (≤15 hits) + `brain_notes`. Deliberately NOT given:
  `brain_curate` / `brainResolveProposal` / any write tool — propose-only philosophy
  holds; curation stays in the review inbox. Not exposed to subagents.
- **Model:** chat defaults to the Librarian engine's model only when the engine is
  on (reflect cap > 0) AND the model id maps to a known chat model
  (`isKnownModelId`) — `brain_librarian_status` returns a default even when
  unconfigured, so ungated adoption would hijack the user's model choice.
- **Settings:** tab id `agents` (frozen contract) relabeled **"Librarian"**:
  Librarian instructions (same `customInstructions` key) + the engine block
  (provider/model/key/cap/activity, moved verbatim from the Brain tab). **Brain tab
  = index only** (workspace, projects, health, rescan). Persona grid, "New agent"
  authoring, and the Snippets settings UI removed.
- **Copy:** every chat surface speaks Librarian (composer placeholder, AiChat empty
  state, AiMiniWindow header, Mod+J "Ask the Librarian about selection", Mod+I
  "Toggle Librarian chat"). Onboarding copy updated; its Librarian step unchanged
  functionally.

## Ceilings / follow-ups (known, deliberate)

1. **Chat spend is not metered**: the Rust ledger only meters reflect runs; the tab
   says so in one muted line. Wiring interactive chat through the budget ledger is
   a Rust-side follow-up if wanted.
2. The `#` picker machinery stays (shared with slash commands, relabeled
   "Commands"); previously-saved snippets still expand but can no longer be
   authored. Full snippet removal = cleanup pass.
3. `AgentSwitcher.tsx` was already unmounted dead code; still compiles. Delete in a
   cleanup pass.
4. Brain tools could be exposed to chat subagents later (registry + runSubagent)
   if the Librarian ever needs to delegate.

## Addendum (2026-07-12, Kosta-approved)

The read-only constraint is scoped to MEMORY, not to everything the chat can
touch: Brain memory stays read-only from chat (unchanged, still enforced), but
the workspace docs (Tasks / Notes / Board panes) now get approval-gated writes.
`ai/tools/workspace.ts` adds reads `workspace_tasks` / `workspace_notes` /
`workspace_boards` (auto-execute) and writes `workspace_task_add` /
`workspace_task_set_done` / `workspace_note_append` (append-only notes, no
delete tools), each write paused by `needsApproval` behind the same in-chat
approval card as `write_file` / `bash_run`. Writes go through the docs store
APIs only (primary + staggered-backup persistence and crash-guard flush), never
raw file IO, and are main-chat-only (not exposed to subagents).

**Layout tools (same day):** the chat can also BUILD workspace layouts —
`ai/tools/layout.ts` adds `workspace_open_tab` (terminal / notes / board /
tasks / editor / library / brain; singletons focus instead of duplicating),
`workspace_split_pane` (terminal | note | tasks — the full split-capable pane
set; four directions; the new pane takes focus so sequential calls compose
layouts), `workspace_focus_pane`, and the read `workspace_layout_state`.
These run WITHOUT approval: every action is immediately visible in the UI,
reversible with one click, and non-destructive. The lane is **create/arrange
only** — no close/delete tools exist in v1 (and no close callbacks are even
threaded through the Live bridge), so the chat can add to a layout but never
tear one down. Plumbing follows the existing pattern: App.tsx callbacks →
`useAiLiveBridge` → `Live` (chatStore) → `ToolContext` (chatRuntime).

## Addendum — terminal targeting + hands-free sends (2026-07-12)

`ai/tools/terminals.ts` gives the chat leaf-addressed reach into ANY terminal
pane in ANY space (the prior tools only saw the active terminal / the
session's one managed agent). Tiered by consequence:

| Tier | Tools | Gate |
|---|---|---|
| List | `workspace_list_terminals` — every pane, all spaces: id, title, space, cwd, agent, active | free |
| Read | `terminal_read` — redacted ~100-line tail of a named pane | free |
| Type | `terminal_send` submit:false — text lands at the prompt, NO Enter; the user's keypress is the execution gate | free |
| Submit | `terminal_send` submit:true — Enter included | approval card, OR free when hands-free is armed |

The submit gate is the SDK's **dynamic `needsApproval`** (a function, not a
boolean — first use in the repo): `submit === true && !ctx.isHandsFreeArmed()`,
evaluated per call so mid-session toggles apply to the next send.

**Resolution** (pure fn `resolveTerminalTarget`, unit-tested): pane id >
exact title > case-insensitive title > title substring > agent name > cwd
basename; titles match both the pane's own title and its tab label. Ambiguity
within ONE tab collapses to that tab's focused pane (naming a tab means its
focused pane — inject-into-active-pty semantics); any other ambiguity or no
match is an ERROR listing all candidates with pane ids — never a best-effort
pick into the wrong pty.

**Pty discipline** (recon'd rules, enforced in `shapeSendText` + the bridge's
`sendToTerminal`): type-only always flattens multiline to one line; shell
submits flatten + `checkShellCommand` (what the approval card shows is the one
logical line that runs); agent-pane submits keep newlines wrapped in bracketed
paste; Enter is always a separate chunk 120 ms later (Claude TUIs treat a
same-chunk CR as a literal newline); sends never focus panes or switch
tabs/spaces. Privacy tabs refuse read AND send (closes the old
`readLeafBuffer` gap for this lane); cold restored tabs resolve by name but
error with "activate it first".

**The hands-free contract** (`handsFreeMode` pref, default OFF, settings-store
pattern; ARMING lives only in the Librarian settings tab with an explicit
warning — deliberately not a one-click header switch, since arming waives
approval gates; the ARMED STATE is always visible in the Librarian window: dot
on the mic button, "Listening — hands-free…" row, title suffix):

- **User-armed only.** The model and its tools can read the pref, never write
  it. Arming is a deliberate act for voice-driving sessions; it persists until
  the user disarms (the settings copy says so honestly).
- **Visible.** Armed state shows in the header switch + settings status dot;
  every hands-free send raises a toast AND still lands in the transcript as a
  normal tool card (target pane, exact text, `hands_free: true`).
- **Scoped.** Hands-free submits reach known agent panes and bare shells only;
  a pane running an unrecognized foreground app (vim, a repl…) refuses armed
  sends — those still require the explicit approval path. Shell text passes
  `checkShellCommand` either way; payloads are capped (8k chars).

## Addendum — voice session ≠ hands-free approvals (2026-07-13)

The always-on VOICE SESSION (`voiceSessionActive`, composer state — session-
scoped, never persisted; the header mic toggle is the ONLY way it starts,
Esc tiering / toggle / window close end it) keeps the mic re-arming after
every assistant turn. It is ORTHOGONAL to the `handsFreeMode` pref:
listen-always ≠ approve-always. The session governs only when the mic
LISTENS; terminal-submit approvals continue to follow `handsFreeMode` alone
(arming stays the deliberate settings act above — the session toggle never
touches it). Either can be on without the other: session-on with the pref off
means every utterance still lands as an approval card; pref-on without a
session keeps the exact legacy re-arm (armed + window open + not suspended,
`shouldRearmVoice` in `ai/hooks/voiceSession.ts`).

**Hotkey tap = one take, Wispr Flow style (2026-07-13).** The Mod+Shift+M tap
no longer toggles the session — it starts ONE continuous MANUAL take
(`mode: "manual"` rides `VoiceCaptureMeta`, like `origin`): no silence
auto-stop, so the user can pause mid-thought indefinitely; the second tap
stops, transcribes, and auto-submits. The one guard a manual take keeps is a
60s never-spoke cancel (`MANUAL_NO_SPEECH_MS`) so a pocket-tap can't hold the
mic hot forever — once any speech registers, the take runs until stopped.
Hold stays classic push-to-talk (release sends). A tap while a session-loop
capture is live stops + submits that capture and the loop re-arms; a header
mic click while a take records stops + delivers that take instead of arming
the session (single-capture invariant). Session-loop and mic-click captures
keep the conversational `mode: "auto"` (silence auto-stop + 8s no-speech
cancel). Decision seams: `chordPressAction`/`chordReleaseAction` in
`ai/hooks/voiceChord.ts`, mode policy in `createSilenceDetector`
(`ai/hooks/useVoiceCapture.ts`).

## Addendum — voice is headless (2026-07-13)

Voice NEVER opens the Librarian window (Wispr-Flow feel: the window popping
open was blocking whatever lives bottom-right). Every voice path — hotkey
take, header-mic session, the voice auto-submit itself — runs with the window
closed; typed sends keep the open-on-send behavior. The ONE sanctioned
voice-path open is the approval auto-open in `AgentRunBridge` (a headless
turn hitting a tool approval genuinely needs the user). The voice surface is
the **VoiceHud** pill (`ai/components/VoiceHud.tsx`, mounted once at App-shell
level, bottom-center, pointer-events only on its buttons): listening
(RMS-driven mini-waveform off `useVoiceCapture.levelRef`, polled at 10Hz —
never a re-render pipe), transcribing, "Librarian is working…" while the
voice turn runs, a ~1.5s done flash, transient errors (~4s). **Reply
routing:** a voice-originated turn (`opts.voice` on `submit`, tracked as
`voiceTurnActive`) that settles cleanly with substantive assistant prose gets
ONE compact toast — "Librarian" + ~140-char preview + an Open action
(`voiceReplyPreview` in `ai/lib/voiceHud.ts`); action-only turns stay silent
(the activity/approval system already narrates). The session re-arm no longer
requires the window (`shouldRearmVoice` session lane; the legacy hands-free
lane keeps its window-open gate exactly). The window-close effect is
TRANSITION-ONLY (`miniCloseActionFor`, prev-ref guarded): only a genuine
open → close still ends a session / delivers a live take — reacting to
"closed" as a state would instantly self-stop every headless take. Esc
tiering, blur stop-and-deliver, and the single-capture invariant are
unchanged; the HUD derives from the same state, so it dismisses consistently.
