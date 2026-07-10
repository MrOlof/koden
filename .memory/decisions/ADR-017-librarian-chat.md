# ADR-017 — The chat is the Librarian

- **Status:** Accepted (Kosta 2026-07-11: "the AI tab can be reworked as Librarian…
  I like the idea of being able to talk to ur librarian"); implemented same day,
  commit `5a960e0` on `feat/koden-svart`.
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
