import type { TerminalTargetInfo } from "@/modules/ai/tools/context";
import { describe, expect, it, vi } from "vitest";
import {
  type CliContext,
  DEFAULT_READ_LINES,
  dispatch,
  MAX_READ_LINES,
  PRESS_KEYS,
} from "./dispatch";
import type { CliPrefs } from "./permissions";

const ALL_ON: CliPrefs = {
  cliEnabled: true,
  cliTerminalRead: true,
  cliTerminalInput: true,
  cliPanelControl: true,
  cliNotify: true,
};

function pane(over: Partial<TerminalTargetInfo>): TerminalTargetInfo {
  return {
    paneId: 1,
    tabId: 10,
    space: "Default",
    title: "shell",
    tabTitle: "shell",
    cwd: null,
    agent: null,
    active: false,
    tabActive: true,
    private: false,
    cold: false,
    ...over,
  };
}

// Pane 1 is the caller (pty 41). Pane 2 is a named shell in another tab.
// Pane 3 runs a managed claude. Pane 4 is private. Pane 5 is cold.
const PANES: TerminalTargetInfo[] = [
  pane({
    paneId: 1,
    tabId: 10,
    title: "api",
    tabTitle: "api",
    cwd: "C:/code/api",
    active: true,
  }),
  pane({
    paneId: 2,
    tabId: 11,
    title: "worker",
    tabTitle: "worker",
    cwd: "C:/code/worker",
  }),
  pane({
    paneId: 3,
    tabId: 12,
    space: "Fleet",
    title: "claude: fix tests",
    tabTitle: "claude: fix tests",
    cwd: "C:/code/fleet",
    agent: { name: "claude", status: "working" },
  }),
  pane({
    paneId: 4,
    tabId: 13,
    title: "secrets",
    tabTitle: "secrets",
    private: true,
  }),
  pane({ paneId: 5, tabId: 14, title: "old", tabTitle: "old", cold: true }),
];

type Calls = {
  send: Array<[number, string, boolean]>;
  openTab: Array<[string, { title?: string; cwd?: string }]>;
  split: Array<[string, string, string | undefined]>;
  createSpace: Array<[string, string | undefined]>;
  notify: Array<{ message: string; paneId: number | null }>;
};

function makeCtx(over: Partial<CliContext> = {}): {
  ctx: CliContext;
  calls: Calls;
} {
  const calls: Calls = {
    send: [],
    openTab: [],
    split: [],
    createSpace: [],
    notify: [],
  };
  const ctx: CliContext = {
    prefs: ALL_ON,
    listTerminalTargets: () => PANES,
    currentPaneId: (session) => (session === "41" ? 1 : null),
    agentState: () => null,
    readBuffer: (paneId, lines, raw) =>
      paneId === 2
        ? null
        : `${raw ? "\x1b[31m" : ""}pane ${paneId} lines=${lines}\nAPI_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz1234`,
    hasForeground: async (paneId) => paneId === 2,
    send: (paneId, data, submit) => {
      calls.send.push([paneId, data, submit]);
      return paneId !== 5;
    },
    openTab: (kind, opts) => {
      calls.openTab.push([kind, opts]);
      return { tabId: 99, action: "opened", title: opts.title ?? "shell" };
    },
    splitPane: (kind, side, title) => {
      calls.split.push([kind, side, title]);
      return { tabId: 10, paneId: 77 };
    },
    listSpaces: () => [
      { id: "sp-1", name: "Default", active: true, tabCount: 3 },
      { id: "sp-2", name: "Fleet", active: false, tabCount: 1 },
    ],
    createSpace: (name, root) => {
      calls.createSpace.push([name, root]);
      return { spaceId: "sp-3", name, switched: true };
    },
    fallbackCwd: () => "C:/fallback",
    isDir: async (path) => path.endsWith("ok"),
    notify: ({ message, pane }) => {
      calls.notify.push({ message, paneId: pane?.paneId ?? null });
      return "toast";
    },
    ...over,
  };
  return { ctx, calls };
}

function okResult(
  r: Awaited<ReturnType<typeof dispatch>>,
): Record<string, unknown> {
  if (!r.ok) throw new Error(`expected ok, got: ${r.error}`);
  return r.result as Record<string, unknown>;
}

function errOf(r: Awaited<ReturnType<typeof dispatch>>): string {
  if (r.ok) throw new Error(`expected error, got: ${JSON.stringify(r.result)}`);
  return r.error;
}

describe("dispatch: gate + ping", () => {
  it("denies before touching the context", async () => {
    const listSpy = vi.fn(() => PANES);
    const { ctx } = makeCtx({
      prefs: { ...ALL_ON, cliTerminalRead: false },
      listTerminalTargets: listSpy,
    });
    expect(errOf(await dispatch("terminal.list", {}, "41", ctx))).toBe(
      "Terminal read is disabled in Settings > CLI",
    );
    expect(listSpy).not.toHaveBeenCalled();
    expect(errOf(await dispatch("bogus", {}, "41", ctx))).toContain(
      "unknown command",
    );
  });

  it("ping answers pong; Rust adds version and pid", async () => {
    const { ctx } = makeCtx();
    expect(okResult(await dispatch("ping", {}, null, ctx))).toEqual({
      pong: true,
    });
  });
});

describe("dispatch: terminal.list", () => {
  it("marks the caller current and merges detected agent state", async () => {
    const { ctx } = makeCtx({
      agentState: (id) =>
        id === 2 ? { name: "claude", status: "waiting" } : null,
    });
    const r = okResult(await dispatch("terminal.list", {}, "41", ctx));
    expect(r.count).toBe(5);
    expect(r.current).toBe(1);
    const list = r.terminals as Array<Record<string, unknown>>;
    expect(list[0]).toMatchObject({
      paneId: 1,
      current: true,
      active: true,
      agent: null,
    });
    expect(list[1]).toMatchObject({
      paneId: 2,
      current: false,
      agent: { name: "claude", status: "waiting" },
    });
    expect(list[2].agent).toEqual({ name: "claude", status: "working" });
    expect(list[3]).toMatchObject({ private: true });
    expect(list[4]).toMatchObject({ cold: true });
    expect("private" in list[0]).toBe(false);
  });

  it("no session means no current pane, list still works", async () => {
    const { ctx } = makeCtx();
    const r = okResult(await dispatch("terminal.list", {}, null, ctx));
    expect(r.current).toBeNull();
    expect(
      (r.terminals as Array<{ current: boolean }>).every((t) => !t.current),
    ).toBe(true);
  });
});

describe("dispatch: terminal.read", () => {
  it("defaults to the caller and 200 lines, redacts secrets", async () => {
    const { ctx } = makeCtx();
    const r = okResult(await dispatch("terminal.read", {}, "41", ctx));
    expect(r.pane).toMatchObject({ paneId: 1, title: "api" });
    expect(r.matched_by).toBe("current");
    expect(r.lines).toBe(DEFAULT_READ_LINES);
    expect(r.raw).toBe(false);
    expect(r.output).toContain(`pane 1 lines=${DEFAULT_READ_LINES}`);
    expect(r.output).toContain("API_KEY=<REDACTED>");
    expect(r.output).not.toContain("sk-proj-");
  });

  it("honors --panel (fuzzy), --lines and --raw", async () => {
    const { ctx } = makeCtx();
    const r = okResult(
      await dispatch(
        "terminal.read",
        { panel: "FIX TESTS", lines: 40, raw: true },
        "41",
        ctx,
      ),
    );
    expect(r.pane).toMatchObject({ paneId: 3, space: "Fleet" });
    expect(r.matched_by).toBe("title-substring");
    expect(r.output).toContain("\x1b[31m");
    expect(r.output).toContain("lines=40");
  });

  it("validates lines and refuses private, cold, dead and unknown panes", async () => {
    const { ctx } = makeCtx();
    expect(
      errOf(await dispatch("terminal.read", { lines: 0 }, "41", ctx)),
    ).toContain("--lines");
    expect(
      errOf(
        await dispatch(
          "terminal.read",
          { lines: MAX_READ_LINES + 1 },
          "41",
          ctx,
        ),
      ),
    ).toContain("--lines");
    expect(
      errOf(await dispatch("terminal.read", { lines: "50" }, "41", ctx)),
    ).toContain("--lines");
    expect(
      errOf(await dispatch("terminal.read", { panel: "secrets" }, "41", ctx)),
    ).toContain("Privacy");
    expect(
      errOf(await dispatch("terminal.read", { panel: "old" }, "41", ctx)),
    ).toContain("never activated");
    expect(
      errOf(await dispatch("terminal.read", { panel: "worker" }, "41", ctx)),
    ).toContain("no live buffer");
    expect(
      errOf(await dispatch("terminal.read", { panel: "nope" }, "41", ctx)),
    ).toContain("no terminal matches");
  });

  it("without a session and without --panel it explains what to pass", async () => {
    const { ctx } = makeCtx();
    const e = errOf(await dispatch("terminal.read", {}, null, ctx));
    expect(e).toContain("KODEN_SESSION");
    expect(e).toContain("--panel");
    // A stale session id (pane gone) is the same case.
    expect(errOf(await dispatch("terminal.read", {}, "999", ctx))).toContain(
      "--panel",
    );
  });
});

describe("dispatch: terminal.type / run / press", () => {
  it("type flattens to one line, never submits", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch("terminal.type", { text: "echo hi\n  there" }, "41", ctx),
    );
    expect(r).toMatchObject({
      action: "typed",
      target_kind: "shell",
      text: "echo hi there",
    });
    expect(calls.send).toEqual([[1, "echo hi there", false]]);
  });

  it("run submits; into a shell the safety filter applies", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch("terminal.run", { text: "pnpm test" }, "41", ctx),
    );
    expect(r).toMatchObject({ action: "submitted", target_kind: "shell" });
    expect(calls.send).toEqual([[1, "pnpm test", true]]);
    const e = errOf(
      await dispatch("terminal.run", { text: "rm -rf /" }, "41", ctx),
    );
    expect(e.length).toBeGreaterThan(0);
    expect(calls.send).toHaveLength(1);
  });

  it("run into an agent pane keeps newlines as a bracketed paste", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch(
        "terminal.run",
        { text: "fix the tests\nthen lint", panel: "3" },
        "41",
        ctx,
      ),
    );
    expect(r).toMatchObject({ target_kind: "agent", matched_by: "pane-id" });
    expect(calls.send[0][1]).toBe("\x1b[200~fix the tests\nthen lint\x1b[201~");
    expect(calls.send[0][2]).toBe(true);
  });

  it("a foreground app counts as a TUI target", async () => {
    const { ctx } = makeCtx({ readBuffer: () => "x" });
    const r = okResult(
      await dispatch(
        "terminal.type",
        { text: ":wq", panel: "worker" },
        "41",
        ctx,
      ),
    );
    expect(r.target_kind).toBe("app");
  });

  it("press maps keys to bytes and validates them", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch("terminal.press", { key: "Enter" }, "41", ctx),
    );
    expect(r).toMatchObject({ key: "enter", target_kind: "shell" });
    expect(calls.send).toEqual([[1, "\r", false]]);
    okResult(
      await dispatch(
        "terminal.press",
        { key: "ctrl-c", panel: "3" },
        "41",
        ctx,
      ),
    );
    expect(calls.send[1]).toEqual([3, PRESS_KEYS["ctrl-c"], false]);
    expect(
      errOf(await dispatch("terminal.press", { key: "f13" }, "41", ctx)),
    ).toContain("key must be");
    expect(errOf(await dispatch("terminal.press", {}, "41", ctx))).toContain(
      "key must be",
    );
  });

  it("refuses private, cold and dead panes, and empty or huge text", async () => {
    const { ctx, calls } = makeCtx();
    expect(
      errOf(
        await dispatch(
          "terminal.type",
          { text: "x", panel: "secrets" },
          "41",
          ctx,
        ),
      ),
    ).toContain("Privacy");
    expect(
      errOf(
        await dispatch("terminal.type", { text: "x", panel: "old" }, "41", ctx),
      ),
    ).toContain("never activated");
    expect(
      errOf(await dispatch("terminal.type", { text: "" }, "41", ctx)),
    ).toContain("text is required");
    expect(
      errOf(
        await dispatch("terminal.type", { text: "y".repeat(9000) }, "41", ctx),
      ),
    ).toContain("too large");
    expect(
      errOf(await dispatch("terminal.type", { text: "a\x07b" }, "41", ctx)),
    ).toContain("control");
    expect(calls.send).toEqual([]);
  });
});

describe("dispatch: tab.open", () => {
  it("normalizes kinds and forwards the title", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch(
        "tab.open",
        { kind: "Note", title: " scratch " },
        "41",
        ctx,
      ),
    );
    expect(r).toMatchObject({ tabId: 99, action: "opened", kind: "notes" });
    expect(calls.openTab).toEqual([
      ["notes", { title: "scratch", cwd: undefined }],
    ]);
    expect(
      errOf(await dispatch("tab.open", { kind: "editor" }, "41", ctx)),
    ).toContain("kind must be");
  });

  it("resolves --cwd against the caller and checks it exists", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch(
        "tab.open",
        { kind: "terminal", cwd: "sub/ok" },
        "41",
        ctx,
      ),
    );
    expect(r.cwd).toBe("C:/code/api/sub/ok");
    expect(calls.openTab[0][1].cwd).toBe("C:/code/api/sub/ok");
    expect(
      errOf(
        await dispatch(
          "tab.open",
          { kind: "terminal", cwd: "sub/missing" },
          "41",
          ctx,
        ),
      ),
    ).toContain("not a directory");
    expect(
      errOf(
        await dispatch("tab.open", { kind: "notes", cwd: "ok" }, "41", ctx),
      ),
    ).toContain("only applies");
    // No caller pane: relative paths fall back to the workspace cwd.
    const r2 = okResult(
      await dispatch("tab.open", { kind: "terminal", cwd: "ok" }, null, ctx),
    );
    expect(r2.cwd).toBe("C:/fallback/ok");
  });
});

describe("dispatch: pane.split", () => {
  it("maps kind and direction onto the layout lane", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch(
        "pane.split",
        { kind: "notes", dir: "down", title: "log" },
        "41",
        ctx,
      ),
    );
    expect(r).toMatchObject({
      tabId: 10,
      paneId: 77,
      kind: "note",
      dir: "down",
    });
    expect(r.note).toBeUndefined();
    expect(calls.split).toEqual([["note", "bottom", "log"]]);
    expect(
      errOf(
        await dispatch("pane.split", { kind: "board", dir: "left" }, "41", ctx),
      ),
    ).toContain("kind must be");
    expect(
      errOf(
        await dispatch(
          "pane.split",
          { kind: "terminal", dir: "sideways" },
          "41",
          ctx,
        ),
      ),
    ).toContain("--dir");
  });

  it("notes when the split landed away from the calling terminal's tab", async () => {
    const { ctx } = makeCtx({ splitPane: () => ({ tabId: 11, paneId: 78 }) });
    const r = okResult(
      await dispatch(
        "pane.split",
        { kind: "terminal", dir: "right" },
        "41",
        ctx,
      ),
    );
    expect(r.note).toContain("active tab");
  });
});

describe("dispatch: spaces", () => {
  it("lists spaces with the active name", async () => {
    const { ctx } = makeCtx();
    const r = okResult(await dispatch("space.list", {}, null, ctx));
    expect(r).toMatchObject({ count: 2, active: "Default" });
  });

  it("creates with an optional validated root and flags duplicate names", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch(
        "space.new",
        { name: "review", root: "../wt/ok" },
        "41",
        ctx,
      ),
    );
    expect(r).toMatchObject({
      spaceId: "sp-3",
      name: "review",
      root: "C:/code/api/../wt/ok",
    });
    expect(calls.createSpace).toEqual([["review", "C:/code/api/../wt/ok"]]);
    const dup = okResult(
      await dispatch("space.new", { name: "Fleet" }, "41", ctx),
    );
    expect(dup.note).toContain("shares this name");
    expect(
      errOf(await dispatch("space.new", { name: "  " }, "41", ctx)),
    ).toContain("name");
    expect(
      errOf(
        await dispatch("space.new", { name: "x", root: "missing" }, "41", ctx),
      ),
    ).toContain("not a directory");
  });
});

describe("dispatch: notify", () => {
  it("attributes to the calling pane and reports the channel", async () => {
    const { ctx, calls } = makeCtx();
    const r = okResult(
      await dispatch("notify", { message: " tests green " }, "41", ctx),
    );
    expect(r).toMatchObject({
      notified: true,
      via: "toast",
      pane: { paneId: 1 },
    });
    expect(calls.notify).toEqual([{ message: "tests green", paneId: 1 }]);
    const r2 = okResult(await dispatch("notify", { message: "hi" }, null, ctx));
    expect(r2.pane).toBeUndefined();
  });

  it("reports muted honestly and validates the message", async () => {
    const { ctx } = makeCtx({ notify: () => "muted" });
    const r = okResult(await dispatch("notify", { message: "x" }, "41", ctx));
    expect(r).toMatchObject({ notified: false, via: "muted" });
    expect(errOf(await dispatch("notify", {}, "41", ctx))).toContain("message");
    expect(
      errOf(await dispatch("notify", { message: "m".repeat(501) }, "41", ctx)),
    ).toContain("message");
  });
});
