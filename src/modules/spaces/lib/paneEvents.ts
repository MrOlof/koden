// Remote agent status over pane events (M2.8): the shared Claude Code hooks
// on an ssh host append one JSON line per lifecycle event to
// ~/.koden/pane-events.jsonl; Koden tails that file (ssh_pane_events_start)
// and joins pane id -> w-<key> window -> leaf restore key -> tab, so remote
// tabs blink working/attention like local ones. This module is the pure core:
// line parsing, the status decision, and the pane->key join.

import type { TabStatus } from "@/modules/tabs";
import { type RemoteWindow, keyFromWindowName } from "./remoteSessions";

export const PANE_EVENT_KINDS = [
  "session-start",
  "user-prompt",
  "notification",
  "stop",
] as const;
export type PaneEventKind = (typeof PANE_EVENT_KINDS)[number];

export type PaneEvent = {
  pane: string;
  sessionId: string;
  event: PaneEventKind;
  ts: number;
};

const PANE_ID = /^%\d{1,15}$/;

/** One tailed line -> a validated event, or null for anything else (foreign
 * writes, torn lines, future event kinds we don't know). Host content is
 * untrusted input; nothing unvalidated leaves this function. */
export function parsePaneEventLine(line: string): PaneEvent | null {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return null;
  }
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.pane !== "string" || !PANE_ID.test(o.pane)) return null;
  if (typeof o.sessionId !== "string" || o.sessionId.length > 128) return null;
  if (
    typeof o.event !== "string" ||
    !(PANE_EVENT_KINDS as readonly string[]).includes(o.event)
  ) {
    return null;
  }
  if (typeof o.ts !== "number" || !Number.isFinite(o.ts)) return null;
  return {
    pane: o.pane,
    sessionId: o.sessionId,
    event: o.event as PaneEventKind,
    ts: o.ts,
  };
}

export type PaneEventStep = {
  /** Escalation to apply to the pane's tab, or null for no pill change. */
  tab: TabStatus | null;
  /** Whether the pane is mid-turn after this event. */
  midTurn: boolean;
};

/** The status decision, matching the LOCAL semantics exactly: a prompt
 * starts a working turn, a stop finishes it green, and a notification is
 * orange "needs you" whenever it arrives, mid-turn or not, because that is
 * what the local OSC 777 path does (App.tsx escalates every attention
 * marker); 66 of 70 notifications on the host were idle "waiting for your
 * input" pings, so the old mid-turn gate was why remote almost never showed
 * amber while local always did (2026-09-03). session-start resets the turn
 * flag: a fresh session has said nothing worth a pill yet. */
export function paneEventStep(
  kind: PaneEventKind,
  midTurn: boolean,
): PaneEventStep {
  switch (kind) {
    case "user-prompt":
      return { tab: "working", midTurn: true };
    case "stop":
      return { tab: "done", midTurn: false };
    case "notification":
      return { tab: "waiting", midTurn };
    case "session-start":
      return { tab: null, midTurn: false };
  }
}

/** pane id -> leaf restore key, from a session's window listing. Windows
 * without a Koden `w-<key>` name or without a usable pane id are skipped. */
export function paneKeyMap(
  windows: readonly RemoteWindow[],
): Map<string, string> {
  const out = new Map<string, string>();
  for (const w of windows) {
    if (!w.pane) continue;
    const key = keyFromWindowName(w.name);
    if (key !== null) out.set(w.pane, key);
  }
  return out;
}
