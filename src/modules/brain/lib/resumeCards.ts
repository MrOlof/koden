// Pure model for the "Resume where you left off" cards: shaping recovered
// panes for display, matching them to restored terminal leaves, and picking
// the command a resume types. No IPC, no DOM.

import { getAgentCommandWithArgs } from "@/modules/orchestration/lib/agentCommand";
import type { RecoveredPane, ResumePlan } from "./bindings";

export type ResumeCardModel = {
  key: string;
  cwd: string;
  cwdShort: string;
  agent: string;
  agentLabel: string;
  sessionId: string | null;
  sessionShort: string | null;
  lastTs: number;
  lastActivity: string;
  lastKind: string;
  /** Tier-2 possible: Claude with a genuinely captured session id. */
  resumable: boolean;
};

/** cwd equality across the journal (Rust canonical form) and the layout file
 * (OSC 7 form): forward slashes, no verbatim prefix, no trailing slash, and
 * case-folded on drive-letter (Windows) paths. */
export function normalizeCwd(raw: string): string {
  let p = raw.trim().replace(/\\/g, "/");
  if (p.startsWith("//?/")) p = p.slice(4);
  if (/^\/[A-Za-z]:/.test(p)) p = p.slice(1);
  p = p.replace(/\/+$/, "");
  if (/^[A-Za-z]:/.test(p)) p = p.toLowerCase();
  return p;
}

/** The journal's cwd in the frontend's canonical form (forward slashes, no
 * verbatim prefix), the shape tab/leaf cwds and PTY spawn expect. */
export function frontendCwd(raw: string): string {
  let p = raw.trim().replace(/\\/g, "/");
  if (p.startsWith("//?/")) p = p.slice(4);
  return p.replace(/(.)\/+$/, "$1");
}

export function basenameOf(cwd: string): string {
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : cwd;
}

export function shortenCwd(
  cwd: string,
  home?: string | null,
  maxSegments = 3,
): string {
  let p = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  if (home) {
    const h = home.replace(/\\/g, "/").replace(/\/+$/, "");
    const under =
      h.length > 0 &&
      p.toLowerCase().startsWith(h.toLowerCase()) &&
      (p.length === h.length || p[h.length] === "/");
    if (under) p = `~${p.slice(h.length)}`;
  }
  const segs = p.split("/").filter(Boolean);
  if (segs.length <= maxSegments) return p || cwd;
  return `.../${segs.slice(-maxSegments).join("/")}`;
}

export function shortSessionId(id: string | null): string | null {
  return id ? id.slice(0, 8) : null;
}

export function agentLabel(agent: string | null): string {
  const a = (agent ?? "").toLowerCase();
  if (a.includes("claude")) return "Claude Code";
  if (a.includes("codex")) return "Codex";
  return agent || "Agent";
}

export function relativeTime(ts: number, now: number): string {
  if (!(ts > 0)) return "";
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

/** Mirrors the Rust Tier-2 gate: agent is exactly `claude` and an id exists.
 * (The allowlist on the id itself is enforced Rust-side when the plan is built.) */
export function isResumable(p: RecoveredPane): boolean {
  return p.agent === "claude" && !!p.claude_session_id;
}

/** The card's one-line meta: agent, the short session id (or an explicit
 * "no session id", so a Tier-1 card says why it can only reopen), last activity. */
export function cardMeta(card: ResumeCardModel): string {
  return [card.agentLabel, card.sessionShort ?? "no session id", card.lastActivity]
    .filter(Boolean)
    .join(" · ");
}

/** Button label: Tier-2 resumes the captured session, Tier-1 only reopens. */
export function resumeActionLabel(card: ResumeCardModel): "Resume" | "Reopen" {
  return card.resumable ? "Resume" : "Reopen";
}

export function buildResumeCards(
  panes: RecoveredPane[],
  opts: { now: number; home?: string | null; hidden?: ReadonlySet<string> },
): ResumeCardModel[] {
  const out: ResumeCardModel[] = [];
  for (const p of panes) {
    if (!p.agent || !p.cwd || opts.hidden?.has(p.key)) continue;
    const cwd = frontendCwd(p.cwd);
    out.push({
      key: p.key,
      cwd,
      cwdShort: shortenCwd(cwd, opts.home),
      agent: p.agent,
      agentLabel: agentLabel(p.agent),
      sessionId: p.claude_session_id,
      sessionShort: shortSessionId(p.claude_session_id),
      lastTs: p.last_ts,
      lastActivity: relativeTime(p.last_ts, opts.now),
      lastKind: p.last_kind,
      resumable: isResumable(p),
    });
  }
  return out.sort((a, b) => b.lastTs - a.lastTs);
}

export type RestoredLeafRef = {
  leafId: number;
  tabId: number;
  cwd: string | null | undefined;
};

export type RecoveredMatch = {
  pane: RecoveredPane;
  leafId: number;
  tabId: number;
};

/** Pair resumable panes with restored shell leaves by cwd. Each leaf and each
 * pane is used at most once, so two Claude sessions in the same folder claim
 * two different leaves (or the second one stays a card). */
export function matchRecoveredPanes(
  panes: RecoveredPane[],
  leaves: RestoredLeafRef[],
): RecoveredMatch[] {
  const byCwd = new Map<string, RestoredLeafRef[]>();
  for (const l of leaves) {
    if (!l.cwd) continue;
    const k = normalizeCwd(l.cwd);
    const arr = byCwd.get(k);
    if (arr) arr.push(l);
    else byCwd.set(k, [l]);
  }
  const out: RecoveredMatch[] = [];
  for (const p of panes) {
    if (!isResumable(p)) continue;
    const leaf = byCwd.get(normalizeCwd(p.cwd))?.shift();
    if (leaf) out.push({ pane: p, leafId: leaf.leafId, tabId: leaf.tabId });
  }
  return out;
}

/** What a manual Resume types once the shell is ready: the Tier-2 command,
 * else a plain relaunch for Claude (Tier-1), else nothing (an unknown agent
 * just gets its terminal back in the right folder). */
export function resumeCommandFor(
  plan: ResumePlan | null,
  pane: RecoveredPane,
  base: string,
): string | null {
  if (plan?.tier === "tier2") return plan.command;
  return pane.agent === "claude" ? base : null;
}

/** The user's agent launch command in its flag-safe form (`claude`, or a
 * custom wrapper), the `base_launch` the Rust plan splices `--resume` onto. */
export function resumeBaseLaunch(): string {
  return getAgentCommandWithArgs();
}

// Structurally matches the launcher's LauncherItemModel / LauncherSectionModel
// (src/modules/launcher, built in parallel) without importing it.
export type RecoveredLauncherItem = {
  id: string;
  label: string;
  description?: string;
  hint?: string;
  badge?: string;
  onSelect: () => void;
};

export type RecoveredLauncherSection = {
  id: string;
  title: string;
  items: RecoveredLauncherItem[];
};

/** One launcher section for the cards, or none when there is nothing to resume
 * (an empty array hides the section entirely). */
export function recoveredLauncherSections(
  cards: ResumeCardModel[],
  resume: (key: string) => void,
): RecoveredLauncherSection[] {
  if (cards.length === 0) return [];
  return [
    {
      id: "resume",
      title: "Resume where you left off",
      items: cards.map((c) => ({
        id: `resume:${c.key}`,
        label: `${c.agentLabel} in ${basenameOf(c.cwd)}`,
        description: c.cwdShort,
        ...(c.lastActivity && { hint: c.lastActivity }),
        ...(c.sessionShort && { badge: c.sessionShort }),
        onSelect: () => resume(c.key),
      })),
    },
  ];
}
