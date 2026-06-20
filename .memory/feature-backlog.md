---
created: "2026-06-16T02:30:00+02:00"
updated: "2026-06-16T02:30:00+02:00"
temporalSource: git
---

# Terax Workspace — feature backlog (proposals)

Proposed 2026-06-16 (overnight). **Nothing here is built** — these are for your
approval. Grouped by "natural extensions of what shipped tonight" (low risk,
high coherence) and "new directions." Effort is rough: S = a sitting, M = a day,
L = multi-day. Design bias matches your taste: Lucide/Hugeicons line icons only,
no AI-startup iconography, keyboard-first, crash-safe by default.

## A. Natural extensions of tonight's work

1. **Agents panel: "active only" toggle + group by space/tab.** (S–M)
   Pre-registration now shows *every* terminal, which is what you asked for but
   can get busy with many shells. A one-click "active only" filter (hide idle
   shells) and a group-by-space header keeps it scannable. This is the natural
   companion to the pre-registration and the first thing I'd add if the panel
   feels noisy.

2. **Live token + cost meter per agent.** (M)
   The dock already has the fields (`tokens`, `cost`) — they just read 0. Parse
   claude's usage (or carry it on the bus/OSC) and show live per-agent + a
   workspace total. You care about token budgets; this makes a Director run's
   spend visible as it happens.

3. **Tasks ↔ terminals: "run this task" + assignment.** (M)
   Right-click a task → "send to terminal" (pick a running terminal, inject the
   text as a prompt/command). Optionally tag a task with the agent working it, so
   the Tasks tab and the Agents panel reference each other. Bridges planning and
   doing without copy-paste.

4. **Desktop notification on needs-input / finished (gated).** (M)
   The taskbar flash shipped tonight; the OS-notification plumbing already exists
   in the agents module. Wire it to the orchestration foundation so you get a
   real notification (with the prompt text) when an agent needs you — even with
   Terax in the background. Needs the deferred OSC-prompt-text capture to show
   *what* it's asking. Respect the existing "Coding agent notifications" toggle.

5. **Activity log / "while you were away" timeline.** (M)
   A scrollback of agent events across all terminals (started · needed input ·
   finished · exited, with timestamps and one-click jump). Complements the flash:
   the flash says "come back," the log says "here's everything that happened."

6. **Inline markdown checkboxes in Notes.** (S–M)
   `- [ ]` / `- [x]` rendered clickable inside a note (Streamdown is already in
   the tree). Throwaway capture mid-note, distinct from the durable Tasks tab.
   Caveat: turns the raw textarea into a rendered surface — a real UX change to
   Notes, so it's a deliberate choice, not a freebie.

## B. New directions

7. **Named workspace snapshots.** (M–L)
   Save the whole layout (tabs + panes + notes + tasks) under a name —
   "Microsoft work", "IoT", "NorrShift" — and restore it in one action. You
   juggle many projects; this is faster than rebuilding a space by hand. Builds
   directly on the crash-safe persistence work.

8. **Global quick-capture hotkey → Notes/Tasks.** (S–M)
   A single shortcut opens a tiny capture box (command-palette style) to jot a
   note or task from anywhere, no tab switch. This is the Notepad-replacement
   instinct behind idea #3, made instant.

9. **Per-space orchestration scoping.** (M)
   Scope the Agents panel / topology to the active space, with a per-space
   rollup. Keeps the Director + team for one project from bleeding into another.
   (On the original roadmap.)

10. **Answer a prompt from the dock (v2 of notifications).** (L)
    When an agent is waiting, show its question in the dock and let you pick an
    option / type a reply that's injected into the PTY — without switching to the
    terminal. The big one; deferred because injecting keystrokes safely is real
    work, but it's the logical endpoint of the notification thread.

11. **Topology graph, for real.** (L)
    Once agents-everywhere is solid (it now is), make the graph live: terminals
    as nodes, Director→subagent edges from the bus, message-flow edges, status
    color, click-to-jump. You flagged this as "after detection works" — detection
    now works.

12. **Director cost/turn guardrails.** (M)
    Set a token or USD ceiling for a Director run; warn or pause when it's hit.
    Pairs with #2.

## Recommended first pick

If you want momentum, **A1 (active-only filter)** is the smallest thing that
makes tonight's agent-visibility change feel finished, and **A4 (desktop
notifications)** is the highest-value next step on the notification thread. Both
are low-risk and build straight on what shipped.
