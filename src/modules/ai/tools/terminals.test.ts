import { describe, expect, it } from "vitest";
import type { TerminalTargetInfo, ToolContext } from "./context";
import {
  buildTerminalTargetTools,
  flattenToLine,
  hasDisallowedControls,
  resolveTerminalTarget,
  shapeSendText,
} from "./terminals";

const CALL_OPTS = { toolCallId: "t", messages: [] };

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

// Fixture: two spaces, an agent pane, a split tab, a private tab, a cold tab.
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
    title: "API server",
    tabTitle: "API server",
    cwd: "C:/code/api-server",
  }),
  pane({
    paneId: 3,
    tabId: 12,
    space: "Fleet",
    title: "claude · fix the tests",
    tabTitle: "claude · fix the tests",
    cwd: "C:/code/fleet",
    agent: { name: "claude", status: "working" },
  }),
  // One tab, two panes ("koden" names the tab; pane 5 is its focused leaf).
  pane({
    paneId: 4,
    tabId: 13,
    title: "server",
    tabTitle: "koden",
    cwd: "C:/code/koden",
    tabActive: false,
  }),
  pane({
    paneId: 5,
    tabId: 13,
    title: "logs",
    tabTitle: "koden",
    cwd: "C:/code/koden",
    tabActive: true,
  }),
  pane({
    paneId: 6,
    tabId: 14,
    title: "secrets",
    tabTitle: "secrets",
    private: true,
  }),
  pane({
    paneId: 7,
    tabId: 15,
    title: "restored",
    tabTitle: "restored",
    cold: true,
  }),
];

describe("resolveTerminalTarget", () => {
  it("resolves a pane id outright, with or without '#'", () => {
    expect(resolveTerminalTarget("3", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 3 },
      via: "pane-id",
    });
    expect(resolveTerminalTarget("#5", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 5 },
    });
  });

  it("exact title beats case-insensitive and substring", () => {
    // 'api' is exact on pane 1; ci would also hit nothing else exactly, and
    // substring would hit 'API server' — the exact tier wins first.
    expect(resolveTerminalTarget("api", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 1 },
      via: "title",
    });
  });

  it("falls through to case-insensitive title", () => {
    expect(resolveTerminalTarget("API SERVER", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 2 },
      via: "title-ci",
    });
  });

  it("falls through to substring", () => {
    expect(resolveTerminalTarget("serv", PANES)).toMatchObject({
      ok: false,
    }); // 'API server' + 'server' both match → ambiguous across tabs
    expect(resolveTerminalTarget("resto", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 7 },
      via: "title-substring",
    });
  });

  it("matches agent names when titles miss", () => {
    // 'claude' substring-matches the agent tab's TITLE first (tier 3) — use a
    // fixture where only the agent field can match.
    const panes = [
      pane({ paneId: 1, title: "work", tabTitle: "work" }),
      pane({
        paneId: 2,
        tabId: 11,
        title: "pane two",
        tabTitle: "pane two",
        agent: { name: "reviewer", status: "ready" },
      }),
    ];
    expect(resolveTerminalTarget("reviewer", panes)).toMatchObject({
      ok: true,
      pane: { paneId: 2 },
      via: "agent-name",
    });
  });

  it("falls back to cwd basename", () => {
    const panes = [
      pane({ paneId: 1, title: "one", tabTitle: "one", cwd: "C:/code/alpha" }),
      pane({
        paneId: 2,
        tabId: 11,
        title: "two",
        tabTitle: "two",
        cwd: "C:/code/beta-svc",
      }),
    ];
    expect(resolveTerminalTarget("beta-svc", panes)).toMatchObject({
      ok: true,
      pane: { paneId: 2 },
      via: "cwd-basename",
    });
  });

  it("collapses within-one-tab ambiguity to the tab's focused pane", () => {
    // 'koden' matches both panes of tab 13 via the tab label → the focused
    // leaf (pane 5) is the canonical target.
    expect(resolveTerminalTarget("koden", PANES)).toMatchObject({
      ok: true,
      pane: { paneId: 5 },
    });
  });

  it("errors on cross-tab ambiguity, listing every candidate with ids", () => {
    const r = resolveTerminalTarget("serv", PANES);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toContain("ambiguous");
      expect(r.error).toContain("#2");
      expect(r.error).toContain("#4");
    }
  });

  it("errors on no match, listing all panes", () => {
    const r = resolveTerminalTarget("zzz-nothing", PANES);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toContain("no terminal matches 'zzz-nothing'");
      expect(r.error).toContain("#1");
      expect(r.error).toContain("#7");
    }
  });

  it("errors when no panes exist", () => {
    expect(resolveTerminalTarget("api", [])).toMatchObject({ ok: false });
  });
});

describe("shapeSendText", () => {
  it("type-only flattens multiline exactly like the agent-prompt collapse", () => {
    const text = "line one\n  line two\r\n\tline three";
    const r = shapeSendText(text, { submit: false, tui: false });
    expect(r).toMatchObject({
      ok: true,
      payload: "line one line two line three",
      multiline: false,
    });
    // Same collapse as flattenPrompt / send_to_agent.
    expect(flattenToLine(text)).toBe("line one line two line three");
  });

  it("shell submit flattens and passes the command safety check", () => {
    const good = shapeSendText("pnpm\ntest", { submit: true, tui: false });
    expect(good).toMatchObject({ ok: true, payload: "pnpm test" });
    const bad = shapeSendText("rm -rf /", { submit: true, tui: false });
    expect(bad.ok).toBe(false);
  });

  it("agent submit keeps newlines and wraps multiline in bracketed paste", () => {
    const r = shapeSendText("do this:\n- a\n- b", { submit: true, tui: true });
    expect(r).toMatchObject({ ok: true, multiline: true });
    if (r.ok) {
      expect(r.payload).toBe("\x1b[200~do this:\n- a\n- b\x1b[201~");
      expect(r.display).toBe("do this:\n- a\n- b");
    }
  });

  it("agent submit leaves single lines unbracketed", () => {
    const r = shapeSendText("continue", { submit: true, tui: true });
    expect(r).toMatchObject({
      ok: true,
      payload: "continue",
      multiline: false,
    });
  });

  it("rejects control bytes and empties", () => {
    expect(shapeSendText("a\x07b", { submit: false, tui: true }).ok).toBe(
      false,
    );
    expect(shapeSendText("a\x1b[2Jb", { submit: true, tui: true }).ok).toBe(
      false,
    );
    expect(shapeSendText("   \n ", { submit: false, tui: false }).ok).toBe(
      false,
    );
    expect(
      shapeSendText("x".repeat(9000), { submit: false, tui: true }).ok,
    ).toBe(false);
  });

  it("hasDisallowedControls flags bidi overrides but allows newlines when told", () => {
    expect(hasDisallowedControls("a\nb", true)).toBe(false);
    expect(hasDisallowedControls("a\nb", false)).toBe(true);
    expect(hasDisallowedControls("a\u202Eb", true)).toBe(true);
  });
});

type SendCall = { leafId: number; data: string; submit: boolean };

function makeCtx(over: {
  panes?: TerminalTargetInfo[];
  armed?: boolean;
  foreground?: boolean;
  buffer?: string | null;
  sendOk?: boolean;
  calls?: SendCall[];
}): ToolContext {
  const calls = over.calls ?? [];
  return {
    listTerminalTargets: () => over.panes ?? PANES,
    readTerminalBuffer: () =>
      over.buffer === undefined ? "line1\nline2" : over.buffer,
    sendToTerminal: (leafId: number, data: string, submit: boolean) => {
      calls.push({ leafId, data, submit });
      return over.sendOk ?? true;
    },
    terminalHasForegroundProcess: async () => over.foreground ?? false,
    isHandsFreeArmed: () => over.armed ?? false,
  } as unknown as ToolContext;
}

describe("terminal_send approval policy", () => {
  const needsApproval = (armed: boolean) => {
    const na = buildTerminalTargetTools(makeCtx({ armed })).terminal_send
      .needsApproval;
    if (typeof na !== "function")
      throw new Error("expected dynamic needsApproval");
    return na;
  };

  it("submit: true takes the approval path when hands-free is off", async () => {
    expect(
      await needsApproval(false)(
        { target: "api", text: "ls", submit: true },
        CALL_OPTS,
      ),
    ).toBe(true);
  });

  it("submit: true is free when hands-free is armed", async () => {
    expect(
      await needsApproval(true)(
        { target: "api", text: "ls", submit: true },
        CALL_OPTS,
      ),
    ).toBe(false);
  });

  it("type-only never needs approval, armed or not", async () => {
    expect(
      await needsApproval(false)({ target: "api", text: "ls" }, CALL_OPTS),
    ).toBe(false);
    expect(
      await needsApproval(false)(
        { target: "api", text: "ls", submit: false },
        CALL_OPTS,
      ),
    ).toBe(false);
  });
});

describe("terminal_send execute", () => {
  it("types without Enter on submit: false and reports the shell target", async () => {
    const calls: SendCall[] = [];
    const tools = buildTerminalTargetTools(makeCtx({ calls }));
    const res = await tools.terminal_send.execute?.(
      { target: "api", text: "git status\n", submit: false },
      CALL_OPTS,
    );
    expect(res).toMatchObject({ ok: true, target_kind: "shell" });
    expect(calls).toEqual([{ leafId: 1, data: "git status", submit: false }]);
  });

  it("submits to an agent pane with bracketed multiline", async () => {
    const calls: SendCall[] = [];
    const tools = buildTerminalTargetTools(makeCtx({ calls, armed: true }));
    const res = await tools.terminal_send.execute?.(
      { target: "claude", text: "fix a\nthen b", submit: true },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      ok: true,
      target_kind: "agent",
      hands_free: true,
    });
    expect(calls).toEqual([
      { leafId: 3, data: "\x1b[200~fix a\nthen b\x1b[201~", submit: true },
    ]);
  });

  it("refuses Privacy panes for both read and send", async () => {
    const tools = buildTerminalTargetTools(makeCtx({}));
    const sent = await tools.terminal_send.execute?.(
      { target: "secrets", text: "ls", submit: false },
      CALL_OPTS,
    );
    expect(sent).toMatchObject({ error: expect.stringContaining("Privacy") });
    const read = await tools.terminal_read.execute?.(
      { target: "secrets" },
      CALL_OPTS,
    );
    expect(read).toMatchObject({ error: expect.stringContaining("Privacy") });
  });

  it("refuses cold tabs with an activate-first error", async () => {
    const tools = buildTerminalTargetTools(makeCtx({}));
    const res = await tools.terminal_send.execute?.(
      { target: "restored", text: "ls", submit: false },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      error: expect.stringContaining("never activated"),
    });
  });

  it("refuses hands-free submits into an unknown foreground app", async () => {
    const calls: SendCall[] = [];
    const tools = buildTerminalTargetTools(
      makeCtx({ calls, armed: true, foreground: true }),
    );
    const res = await tools.terminal_send.execute?.(
      { target: "api", text: "hello", submit: true },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      error: expect.stringContaining("foreground app"),
    });
    expect(calls).toHaveLength(0);
    // The same send with approval (hands-free off) goes through as a TUI send.
    const approvedCalls: SendCall[] = [];
    const approved = buildTerminalTargetTools(
      makeCtx({ calls: approvedCalls, armed: false, foreground: true }),
    );
    const ok = await approved.terminal_send.execute?.(
      { target: "api", text: "hello", submit: true },
      CALL_OPTS,
    );
    expect(ok).toMatchObject({ ok: true, target_kind: "app" });
    expect(approvedCalls).toEqual([{ leafId: 1, data: "hello", submit: true }]);
  });

  it("surfaces a dead session instead of pretending it sent", async () => {
    const tools = buildTerminalTargetTools(makeCtx({ sendOk: false }));
    const res = await tools.terminal_send.execute?.(
      { target: "api", text: "ls", submit: false },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      error: expect.stringContaining("no live session"),
    });
  });
});

describe("terminal_read / workspace_list_terminals", () => {
  it("reads the tail of a named pane", async () => {
    const tools = buildTerminalTargetTools(makeCtx({}));
    const res = await tools.terminal_read.execute?.(
      { target: "api" },
      CALL_OPTS,
    );
    expect(res).toMatchObject({
      pane: { paneId: 1, title: "api" },
      output: "line1\nline2",
    });
  });

  it("lists every pane across spaces, flagging private/cold", async () => {
    const tools = buildTerminalTargetTools(makeCtx({}));
    const res = (await tools.workspace_list_terminals.execute?.(
      {},
      CALL_OPTS,
    )) as { count: number; terminals: Array<Record<string, unknown>> };
    expect(res.count).toBe(PANES.length);
    const spaces = new Set(res.terminals.map((t) => t.space));
    expect(spaces.has("Default")).toBe(true);
    expect(spaces.has("Fleet")).toBe(true);
    expect(res.terminals.find((t) => t.paneId === 6)?.private).toBe(true);
    expect(res.terminals.find((t) => t.paneId === 7)?.cold).toBe(true);
    expect(res.terminals.find((t) => t.paneId === 3)?.agent).toEqual({
      name: "claude",
      status: "working",
    });
  });
});
