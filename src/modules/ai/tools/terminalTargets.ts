import { checkShellCommand } from "../lib/security";
import type { TerminalTargetInfo } from "./context";

// Pane resolution + send shaping shared by the Librarian's terminal tools
// (terminals.ts) and the koden CLI bridge (modules/cli). This file must stay
// free of ai/zod imports: the CLI bridge is in the eager graph
// (src/app/eager-budget.test.ts).

export const MAX_SEND_CHARS = 8_000;

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
