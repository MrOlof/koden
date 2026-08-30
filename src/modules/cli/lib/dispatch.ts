import { redactSensitive } from "@/modules/ai/lib/redact";
import {
  type LayoutOpenTabResult,
  type LayoutSplitKind,
  type LayoutSplitResult,
  type LayoutSplitSide,
  type LayoutTabKind,
  resolvePath,
  type SpaceCreateResult,
  type SpaceInfo,
  type TerminalTargetInfo,
} from "@/modules/ai/tools/context";
import {
  normalizeSplitKind,
  sideForDirection,
  SPLIT_KINDS,
} from "@/modules/ai/tools/layoutShared";
import {
  resolveTerminalTarget,
  shapeSendText,
} from "@/modules/ai/tools/terminalTargets";
import { type CliPrefs, checkPermission } from "./permissions";
import { type CliResult, cliError, cliOk } from "./protocol";

// Pure command dispatcher for the koden CLI: every side effect goes through
// `CliContext`, so the whole surface is unit-testable with a fake context and
// the bridge component stays a thin adapter. Arguments are re-validated here;
// the client is just another local process and is never trusted.

export const DEFAULT_READ_LINES = 200;
export const MAX_READ_LINES = 5000;
const MAX_NOTIFY_CHARS = 500;
const MAX_TITLE_CHARS = 120;
const MAX_PATH_CHARS = 4096;

/** Escape bytes for `terminal press`. Arrow keys use normal-mode CSI; a TUI
 * in application-cursor mode still accepts them in practice. */
export const PRESS_KEYS: Record<string, string> = {
  enter: "\r",
  escape: "\x1b",
  "ctrl-c": "\x03",
  "ctrl-d": "\x04",
  "ctrl-l": "\x0c",
  "ctrl-z": "\x1a",
  tab: "\t",
  backspace: "\x7f",
  up: "\x1b[A",
  down: "\x1b[B",
  left: "\x1b[D",
  right: "\x1b[C",
};

const TAB_KINDS: Record<string, LayoutTabKind> = {
  terminal: "terminal",
  note: "notes",
  notes: "notes",
  tasks: "tasks",
  board: "board",
};

export type NotifyVia = "toast" | "os" | "muted";

export type CliContext = {
  prefs: CliPrefs;
  listTerminalTargets: () => TerminalTargetInfo[];
  /** Pane (leaf) id of the shell that made the call, from KODEN_SESSION. */
  currentPaneId: (session: string | null) => number | null;
  /** Detected coding-agent session on a pane (Claude Code hooks), if any. */
  agentState: (paneId: number) => { name: string; status: string } | null;
  /** Last `lines` of a pane's buffer; null when the pane has no session. */
  readBuffer: (paneId: number, lines: number, raw: boolean) => string | null;
  hasForeground: (paneId: number) => Promise<boolean>;
  send: (paneId: number, data: string, submit: boolean) => boolean;
  openTab: (
    kind: LayoutTabKind,
    opts: { title?: string; cwd?: string },
  ) => LayoutOpenTabResult;
  splitPane: (
    kind: LayoutSplitKind,
    side: LayoutSplitSide,
    title?: string,
  ) => LayoutSplitResult;
  listSpaces: () => SpaceInfo[];
  createSpace: (name: string, root?: string) => SpaceCreateResult;
  /** Fallback cwd for relative paths when the caller has no pane. */
  fallbackCwd: () => string | null;
  isDir: (path: string) => Promise<boolean>;
  notify: (args: {
    message: string;
    pane: TerminalTargetInfo | null;
  }) => NotifyVia;
};

type Args = Record<string, unknown>;

function str(args: Args, key: string, max = MAX_TITLE_CHARS): string | null {
  const v = args[key];
  if (typeof v !== "string") return null;
  const t = v.trim();
  if (!t || t.length > max) return null;
  return t;
}

function paneSummary(p: TerminalTargetInfo) {
  return { paneId: p.paneId, title: p.title, space: p.space, tabId: p.tabId };
}

type Target =
  | { ok: true; pane: TerminalTargetInfo; via: string }
  | { ok: false; error: string };

function resolveTarget(
  args: Args,
  session: string | null,
  ctx: CliContext,
): Target {
  const panes = ctx.listTerminalTargets();
  const panel = args.panel;
  if (typeof panel === "string" && panel.trim()) {
    if (panel.length > MAX_TITLE_CHARS)
      return { ok: false, error: "--panel is too long" };
    return resolveTerminalTarget(panel, panes);
  }
  const current = ctx.currentPaneId(session);
  const pane =
    current === null ? undefined : panes.find((p) => p.paneId === current);
  if (!pane) {
    return {
      ok: false,
      error:
        "no calling terminal: KODEN_SESSION is unset or that pane is gone. Pass --panel <id|title> (see 'koden terminal list').",
    };
  }
  return { ok: true, pane, via: "current" };
}

/** Privacy and cold panes refuse both reads and input, like the Librarian. */
function paneUsable(pane: TerminalTargetInfo, verb: string): string | null {
  if (pane.private)
    return `'${pane.title}' is in Privacy mode; refusing to ${verb} it.`;
  if (pane.cold)
    return `'${pane.title}' is a restored tab that was never activated (no live session). Ask the user to open it first.`;
  return null;
}

async function targetKind(
  pane: TerminalTargetInfo,
  ctx: CliContext,
): Promise<{ tui: boolean; kind: "agent" | "app" | "shell" }> {
  if (pane.agent !== null || ctx.agentState(pane.paneId) !== null)
    return { tui: true, kind: "agent" };
  if (await ctx.hasForeground(pane.paneId)) return { tui: true, kind: "app" };
  return { tui: false, kind: "shell" };
}

async function resolveDir(
  raw: string,
  session: string | null,
  ctx: CliContext,
  flag: string,
): Promise<{ ok: true; path: string } | { ok: false; error: string }> {
  if (raw.length > MAX_PATH_CHARS)
    return { ok: false, error: `${flag} is too long` };
  const current = ctx.currentPaneId(session);
  let callerCwd: string | null = null;
  if (current !== null) {
    callerCwd =
      ctx.listTerminalTargets().find((p) => p.paneId === current)?.cwd ?? null;
  }
  callerCwd ??= ctx.fallbackCwd();
  let path: string;
  try {
    path = resolvePath(raw, callerCwd);
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  if (!(await ctx.isDir(path)))
    return { ok: false, error: `${flag}: '${path}' is not a directory` };
  return { ok: true, path };
}

export async function dispatch(
  cmd: string,
  args: Args,
  session: string | null,
  ctx: CliContext,
): Promise<CliResult> {
  const denied = checkPermission(cmd, ctx.prefs);
  if (denied) return cliError(denied);

  switch (cmd) {
    case "ping":
      return cliOk({ pong: true });

    case "terminal.list": {
      const current = ctx.currentPaneId(session);
      const terminals = ctx.listTerminalTargets().map((p) => ({
        paneId: p.paneId,
        tabId: p.tabId,
        space: p.space,
        title: p.title,
        cwd: p.cwd,
        agent: p.agent ?? ctx.agentState(p.paneId),
        active: p.active,
        current: p.paneId === current,
        ...(p.private ? { private: true } : {}),
        ...(p.cold ? { cold: true } : {}),
      }));
      return cliOk({ count: terminals.length, current, terminals });
    }

    case "terminal.read": {
      const t = resolveTarget(args, session, ctx);
      if (!t.ok) return cliError(t.error);
      const blocked = paneUsable(t.pane, "read");
      if (blocked) return cliError(blocked);
      const rawLines = args.lines;
      let lines = DEFAULT_READ_LINES;
      if (rawLines !== undefined) {
        if (
          typeof rawLines !== "number" ||
          !Number.isInteger(rawLines) ||
          rawLines < 1 ||
          rawLines > MAX_READ_LINES
        )
          return cliError(`--lines must be an integer 1..${MAX_READ_LINES}`);
        lines = rawLines;
      }
      const raw = args.raw === true;
      const buf = ctx.readBuffer(t.pane.paneId, lines, raw);
      if (buf === null)
        return cliError(
          `no live buffer for '${t.pane.title}' (pane #${t.pane.paneId}); the session may have closed`,
        );
      return cliOk({
        pane: paneSummary(t.pane),
        matched_by: t.via,
        lines,
        raw,
        output: redactSensitive(buf),
      });
    }

    case "terminal.type":
    case "terminal.run": {
      const text = args.text;
      if (typeof text !== "string" || !text.trim())
        return cliError("text is required");
      const t = resolveTarget(args, session, ctx);
      if (!t.ok) return cliError(t.error);
      const blocked = paneUsable(t.pane, "type into");
      if (blocked) return cliError(blocked);
      const submit = cmd === "terminal.run";
      const { tui, kind } = await targetKind(t.pane, ctx);
      const shaped = shapeSendText(text, { submit, tui });
      if (!shaped.ok) return cliError(shaped.error);
      if (!ctx.send(t.pane.paneId, shaped.payload, submit))
        return cliError(
          `pane #${t.pane.paneId} ('${t.pane.title}') has no live session`,
        );
      return cliOk({
        pane: paneSummary(t.pane),
        matched_by: t.via,
        action: submit ? "submitted" : "typed",
        target_kind: kind,
        text: shaped.display,
      });
    }

    case "terminal.press": {
      const key = typeof args.key === "string" ? args.key.toLowerCase() : "";
      const bytes = PRESS_KEYS[key];
      if (!bytes)
        return cliError(
          `key must be one of: ${Object.keys(PRESS_KEYS).join(", ")}`,
        );
      const t = resolveTarget(args, session, ctx);
      if (!t.ok) return cliError(t.error);
      const blocked = paneUsable(t.pane, "type into");
      if (blocked) return cliError(blocked);
      const { kind } = await targetKind(t.pane, ctx);
      if (!ctx.send(t.pane.paneId, bytes, false))
        return cliError(
          `pane #${t.pane.paneId} ('${t.pane.title}') has no live session`,
        );
      return cliOk({
        pane: paneSummary(t.pane),
        matched_by: t.via,
        key,
        target_kind: kind,
      });
    }

    case "tab.open": {
      const kindRaw =
        typeof args.kind === "string" ? args.kind.toLowerCase() : "";
      const kind = TAB_KINDS[kindRaw];
      if (!kind)
        return cliError(
          `kind must be one of: ${Object.keys(TAB_KINDS).join(", ")}`,
        );
      const title = str(args, "title") ?? undefined;
      if (args.title !== undefined && !title)
        return cliError("--title must be 1..120 characters");
      let cwd: string | undefined;
      if (args.cwd !== undefined) {
        const rawCwd = typeof args.cwd === "string" ? args.cwd.trim() : "";
        if (!rawCwd) return cliError("--cwd must not be empty");
        if (kind !== "terminal")
          return cliError("--cwd only applies to kind 'terminal'");
        const dir = await resolveDir(rawCwd, session, ctx, "--cwd");
        if (!dir.ok) return cliError(dir.error);
        cwd = dir.path;
      }
      const res = ctx.openTab(kind, { title, cwd });
      if ("error" in res) return cliError(res.error);
      return cliOk({ ...res, kind, ...(cwd ? { cwd } : {}) });
    }

    case "pane.split": {
      const kindRaw = typeof args.kind === "string" ? args.kind : "";
      const kind = normalizeSplitKind(kindRaw);
      if (!kind)
        return cliError(
          `kind must be one of: ${SPLIT_KINDS.join(", ")} (board/editor exist only as tabs)`,
        );
      const dir = typeof args.dir === "string" ? args.dir.toLowerCase() : "";
      if (dir !== "left" && dir !== "right" && dir !== "up" && dir !== "down")
        return cliError("--dir must be one of: left, right, up, down");
      const title = str(args, "title") ?? undefined;
      if (args.title !== undefined && !title)
        return cliError("--title must be 1..120 characters");
      const res = ctx.splitPane(kind, sideForDirection(dir), title);
      if ("error" in res) return cliError(res.error);
      const current = ctx.currentPaneId(session);
      const callerTab =
        current === null
          ? null
          : (ctx.listTerminalTargets().find((p) => p.paneId === current)
              ?.tabId ?? null);
      const note =
        callerTab !== null && callerTab !== res.tabId
          ? "the split landed in the active tab (where the user is looking), not the calling terminal's tab"
          : undefined;
      return cliOk({ ...res, kind, dir, ...(note ? { note } : {}) });
    }

    case "space.list": {
      const spaces = ctx.listSpaces();
      return cliOk({
        count: spaces.length,
        active: spaces.find((s) => s.active)?.name ?? null,
        spaces,
      });
    }

    case "space.new": {
      const name = str(args, "name");
      if (!name) return cliError("name must be 1..120 characters");
      let root: string | undefined;
      if (args.root !== undefined) {
        const rawRoot = typeof args.root === "string" ? args.root.trim() : "";
        if (!rawRoot) return cliError("--root must not be empty");
        const dir = await resolveDir(rawRoot, session, ctx, "--root");
        if (!dir.ok) return cliError(dir.error);
        root = dir.path;
      }
      const dup = ctx.listSpaces().some((s) => s.name === name);
      const res = ctx.createSpace(name, root);
      if ("error" in res) return cliError(res.error);
      return cliOk({
        ...res,
        ...(root ? { root } : {}),
        ...(dup
          ? {
              note: "another space shares this name; target by id when switching",
            }
          : {}),
      });
    }

    case "notify": {
      const message = str(args, "message", MAX_NOTIFY_CHARS);
      if (!message)
        return cliError(`message must be 1..${MAX_NOTIFY_CHARS} characters`);
      const current = ctx.currentPaneId(session);
      const pane =
        current === null
          ? null
          : (ctx.listTerminalTargets().find((p) => p.paneId === current) ??
            null);
      const via = ctx.notify({ message, pane });
      return cliOk({
        notified: via !== "muted",
        via,
        ...(pane ? { pane: paneSummary(pane) } : {}),
        ...(via === "muted"
          ? {
              note: "agent notifications are off in Settings; nothing was shown",
            }
          : {}),
      });
    }

    default:
      return cliError(`unknown command '${cmd}'`);
  }
}
