import { tool } from "ai";
import { z } from "zod";
import { checkShellCommand } from "../lib/security";
import type { TerminalTargetInfo, ToolContext } from "./context";

// Terminal targeting (ADR-017 addendum): the Librarian can list every
// terminal pane across all spaces, read a named pane's tail, and type into a
// named pane. Tiers: list/read are free; typing WITHOUT submit is free (the
// text lands at the prompt, the user presses Enter); submitting pauses for
// the in-chat approval card UNLESS the user armed hands-free mode. Sends
// never focus panes or switch tabs/spaces — the user's typing is never
// hijacked, and Privacy tabs refuse both reads and sends.

const MAX_SEND_CHARS = 8_000;
const READ_LINES = 100;
const READ_MAX_CHARS = 24_000;

/** Same collapse as flattenPrompt / send_to_agent: multiline → one line. */
export function flattenToLine(s: string): string {
  return s.replace(/\s*\r?\n\s*/g, " ").trim();
}

/** C0 controls (except \n when allowed), DEL, or Unicode bidi overrides. */
export function hasDisallowedControls(
  s: string,
  allowNewline: boolean,
): boolean {
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c === 0x0a && allowNewline) continue;
    if (c < 0x20 || c === 0x7f) return true;
  }
  return /[\u202A-\u202E\u2066-\u2069\u200E\u200F\u061C]/.test(s);
}

function paneBasename(cwd: string | null): string {
  if (!cwd) return "";
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return (parts[parts.length - 1] ?? "").toLowerCase();
}

function describePane(p: TerminalTargetInfo): string {
  const agent = p.agent ? `, agent ${p.agent.name}` : "";
  const cwd = p.cwd ? `, ${p.cwd}` : "";
  return `#${p.paneId} '${p.title}' (space ${p.space}${cwd}${agent})`;
}

function candidateList(panes: TerminalTargetInfo[]): string {
  return panes.map(describePane).join("; ");
}

export type ResolvedTarget =
  | { ok: true; pane: TerminalTargetInfo; via: string }
  | { ok: false; error: string };

/** 0 or 1 match resolves; >1 collapses to the tab's focused pane when every
 * match sits in ONE tab (the user named the tab — its focused pane is the
 * canonical target, same semantics as inject-into-active-pty). Otherwise
 * ambiguity is an error listing the candidates, never a best-effort pick. */
function narrow(
  matches: TerminalTargetInfo[],
  via: string,
  target: string,
): ResolvedTarget | null {
  if (matches.length === 0) return null;
  if (matches.length === 1) return { ok: true, pane: matches[0], via };
  const tabId = matches[0].tabId;
  if (matches.every((m) => m.tabId === tabId)) {
    const focused = matches.find((m) => m.tabActive);
    if (focused)
      return { ok: true, pane: focused, via: `${via} (tab's focused pane)` };
  }
  return {
    ok: false,
    error: `'${target}' is ambiguous — ${matches.length} panes match: ${candidateList(matches)}. Target the pane id instead.`,
  };
}

/**
 * Fuzzy pane resolution, strictly tiered: pane id > exact title >
 * case-insensitive title > title substring > agent name > cwd basename.
 * Titles match both the pane's own title and its tab's label.
 */
export function resolveTerminalTarget(
  rawTarget: string,
  panes: TerminalTargetInfo[],
): ResolvedTarget {
  const target = rawTarget.trim();
  if (panes.length === 0)
    return { ok: false, error: "no terminal panes are open" };
  if (!target)
    return { ok: false, error: `empty target. Panes: ${candidateList(panes)}` };

  // Pane id (from workspace_list_terminals) — unambiguous, wins outright.
  if (/^#?\d+$/.test(target)) {
    const id = Number(target.replace("#", ""));
    const byId = panes.find((p) => p.paneId === id);
    if (byId) return { ok: true, pane: byId, via: "pane-id" };
  }

  const lower = target.toLowerCase();
  const tiers: Array<{
    via: string;
    match: (p: TerminalTargetInfo) => boolean;
  }> = [
    {
      via: "title",
      match: (p) => p.title === target || p.tabTitle === target,
    },
    {
      via: "title-ci",
      match: (p) =>
        p.title.toLowerCase() === lower || p.tabTitle.toLowerCase() === lower,
    },
    {
      via: "title-substring",
      match: (p) =>
        p.title.toLowerCase().includes(lower) ||
        p.tabTitle.toLowerCase().includes(lower),
    },
    {
      via: "agent-name",
      match: (p) => p.agent?.name.toLowerCase().includes(lower) === true,
    },
    {
      via: "cwd-basename",
      match: (p) => paneBasename(p.cwd).includes(lower),
    },
  ];
  for (const tier of tiers) {
    const res = narrow(panes.filter(tier.match), tier.via, target);
    if (res) return res;
  }
  return {
    ok: false,
    error: `no terminal matches '${target}'. Panes: ${candidateList(panes)}`,
  };
}

export type SendShape =
  | { ok: true; payload: string; display: string; multiline: boolean }
  | { ok: false; error: string };

/**
 * Shape text for a pty write per the pty rules:
 * - type-only (submit=false): always flatten to one line — CR-less multiline
 *   still poisons a shell prompt line-by-line on paste-unaware apps.
 * - submit into a TUI (agent pane / foreground app): keep newlines, wrap
 *   multiline in bracketed paste (spawnManagedAgent's convention); Enter
 *   goes as a separate delayed chunk in the Live bridge.
 * - submit into a shell: flatten + checkShellCommand — one logical line, so
 *   what the approval card showed is exactly what runs.
 */
export function shapeSendText(
  rawText: string,
  opts: { submit: boolean; tui: boolean },
): SendShape {
  const text = rawText.replace(/\r\n/g, "\n").trim();
  if (!text) return { ok: false, error: "empty text" };
  if (text.length > MAX_SEND_CHARS) {
    return {
      ok: false,
      error: `text too large (${text.length} chars; max ${MAX_SEND_CHARS})`,
    };
  }

  if (!opts.submit || !opts.tui) {
    const oneLine = flattenToLine(text);
    if (hasDisallowedControls(oneLine, false))
      return { ok: false, error: "text contains control characters" };
    if (!opts.tui) {
      const safety = checkShellCommand(oneLine);
      if (!safety.ok) return { ok: false, error: safety.reason };
    }
    return { ok: true, payload: oneLine, display: oneLine, multiline: false };
  }

  if (hasDisallowedControls(text, true))
    return { ok: false, error: "text contains control characters" };
  const multiline = text.includes("\n");
  return {
    ok: true,
    payload: multiline ? `\x1b[200~${text}\x1b[201~` : text,
    display: text,
    multiline,
  };
}

/** Hands-free sends stay loud: a toast per send, on top of the tool card the
 * transcript already renders. Lazy import keeps this module hermetic for
 * unit tests (no UI dependency at import time). */
function announceHandsFree(title: string, text: string): void {
  void import("sonner")
    .then(({ toast }) =>
      toast(`Hands-free: sent to ${title}`, {
        description: text.length > 120 ? `${text.slice(0, 119)}…` : text,
      }),
    )
    .catch(() => {});
}

function shapeListEntry(p: TerminalTargetInfo) {
  return {
    paneId: p.paneId,
    tabId: p.tabId,
    space: p.space,
    title: p.title,
    cwd: p.cwd,
    agent: p.agent,
    active: p.active,
    ...(p.private ? { private: true } : {}),
    ...(p.cold
      ? {
          cold: true,
          note: "restored but never activated — unreadable/unwritable until the user opens it",
        }
      : {}),
  };
}

export function buildTerminalTargetTools(ctx: ToolContext) {
  return {
    workspace_list_terminals: tool({
      description:
        "List every terminal pane across ALL spaces (workspace_layout_state only covers the active space): pane id, title, space, cwd, the agent running there (if any), and which pane is active. Call this before terminal_read / terminal_send when the target is named loosely, and to answer 'what's running where'. Auto-executes.",
      inputSchema: z.object({}),
      execute: async () => {
        const panes = ctx.listTerminalTargets();
        return { count: panes.length, terminals: panes.map(shapeListEntry) };
      },
    }),

    terminal_read: tool({
      description:
        "Read the last ~100 lines of a named terminal pane's buffer — any pane in any space, not just the active one (use get_terminal_output for the active terminal). target: a pane id from workspace_list_terminals, a pane/tab title, an agent name, or a folder name; ambiguity returns the candidates. Output is secret-redacted. Refuses Privacy-mode panes. Auto-executes.",
      inputSchema: z.object({
        target: z
          .string()
          .min(1)
          .describe(
            "Pane id (e.g. '7'), pane/tab title, agent name, or cwd folder name.",
          ),
      }),
      execute: async ({ target }) => {
        const r = resolveTerminalTarget(target, ctx.listTerminalTargets());
        if (!r.ok) return { error: r.error };
        const pane = r.pane;
        if (pane.private) {
          return {
            error: `'${pane.title}' is in Privacy mode; its buffer is withheld. Ask the user to switch the tab out of Privacy mode if they want you to see it.`,
          };
        }
        if (pane.cold) {
          return {
            error: `'${pane.title}' is a restored tab that was never activated — no live session to read. Ask the user to open it first.`,
          };
        }
        const buf = ctx.readTerminalBuffer(pane.paneId);
        if (buf === null) {
          return {
            error: `no live buffer for '${pane.title}' (pane #${pane.paneId}) — the session may have closed`,
          };
        }
        const parts = buf.split("\n");
        const sliced =
          parts.length <= READ_LINES
            ? buf
            : parts.slice(parts.length - READ_LINES).join("\n");
        const output =
          sliced.length > READ_MAX_CHARS
            ? `…[truncated]…\n${sliced.slice(sliced.length - READ_MAX_CHARS)}`
            : sliced;
        return {
          pane: { paneId: pane.paneId, title: pane.title, space: pane.space },
          matched_by: r.via,
          output,
        };
      },
    }),

    terminal_send: tool({
      description:
        "Type text into a named terminal pane — any pane in any space, resolved like terminal_read. submit: false (default) TYPES the text at the prompt without pressing Enter — the user reviews and submits; this is free, prefer it for shell commands. submit: true presses Enter too: into an agent pane it delivers the message; into a shell it executes the command (safety-checked, single line) — this pauses for user approval unless hands-free mode is armed. Never steals focus. Refuses Privacy panes.",
      inputSchema: z.object({
        target: z
          .string()
          .min(1)
          .describe(
            "Pane id (e.g. '7'), pane/tab title, agent name, or cwd folder name.",
          ),
        text: z
          .string()
          .min(1)
          .describe(
            "What to type. Multiline is flattened to one line except when submitting to an agent pane (kept, bracketed-paste).",
          ),
        submit: z
          .boolean()
          .optional()
          .describe(
            "false/omitted = type only (user presses Enter). true = submit (approval-gated unless hands-free is armed).",
          ),
      }),
      // Dynamic approval (ADR-017 addendum): typing is free; submitting pauses
      // for the approval card unless the user armed hands-free. Read at
      // call-time so mid-session toggles apply immediately.
      needsApproval: async ({ submit }) =>
        submit === true && !ctx.isHandsFreeArmed(),
      execute: async ({ target, text, submit }) => {
        const doSubmit = submit === true;
        const r = resolveTerminalTarget(target, ctx.listTerminalTargets());
        if (!r.ok) return { error: r.error };
        const pane = r.pane;
        if (pane.private) {
          return {
            error: `'${pane.title}' is in Privacy mode; refusing to type into it.`,
          };
        }
        if (pane.cold) {
          return {
            error: `'${pane.title}' is a restored tab that was never activated — no live session to type into. Ask the user to open it first.`,
          };
        }
        const isAgentPane = pane.agent !== null;
        const hasForeground = await ctx.terminalHasForegroundProcess(
          pane.paneId,
        );
        const tui = isAgentPane || hasForeground;
        const handsFree = doSubmit && ctx.isHandsFreeArmed();
        // Hands-free submits are scoped to known agent panes and bare shells.
        // An unrecognized foreground app (vim? less? a repl?) only takes
        // approved sends — the approval card names the pane, hands-free
        // wouldn't surface anything before the bytes land.
        if (handsFree && hasForeground && !isAgentPane) {
          return {
            error: `'${pane.title}' is running a foreground app that isn't a known agent — hands-free submits are refused there. Ask the user, or send with hands-free off (per-send approval).`,
          };
        }
        const shaped = shapeSendText(text, { submit: doSubmit, tui });
        if (!shaped.ok) return { error: shaped.error };
        if (!ctx.sendToTerminal(pane.paneId, shaped.payload, doSubmit)) {
          return {
            error: `pane #${pane.paneId} ('${pane.title}') has no live session (closed?)`,
          };
        }
        if (handsFree) announceHandsFree(pane.title, shaped.display);
        return {
          ok: true,
          pane: { paneId: pane.paneId, title: pane.title, space: pane.space },
          matched_by: r.via,
          action: doSubmit
            ? "submitted"
            : "typed only — the user presses Enter to run it",
          target_kind: isAgentPane ? "agent" : tui ? "app" : "shell",
          text: shaped.display,
          ...(handsFree ? { hands_free: true } : {}),
        };
      },
    }),
  } as const;
}
