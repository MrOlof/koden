import { describe, expect, it } from "vitest";
import type { RecoveredPane } from "./bindings";
import {
  buildResumeCards,
  frontendCwd,
  matchRecoveredPanes,
  normalizeCwd,
  recoveredLauncherSections,
  relativeTime,
  resumeCommandFor,
  shortenCwd,
} from "./resumeCards";

function pane(over: Partial<RecoveredPane> = {}): RecoveredPane {
  return {
    key: "k1",
    last_kind: "working",
    agent: "claude",
    cwd: "C:/Users/me/proj",
    project: "p",
    claude_session_id: "0198d2fc-3c4b-7a10-9f2e-1b2c3d4e5f60",
    last_ts: T0,
    ...over,
  };
}

// An epoch-ms base far enough from 0 that "hours ago" fixtures stay positive.
const T0 = 1_700_000_000_000;

describe("normalizeCwd", () => {
  it("folds journal and OSC 7 forms of the same Windows folder", () => {
    const forms = [
      "C:\\Users\\me\\proj",
      "c:/users/me/proj/",
      "/C:/Users/me/proj",
      "\\\\?\\C:\\Users\\me\\proj",
    ];
    for (const f of forms) expect(normalizeCwd(f)).toBe("c:/users/me/proj");
  });

  it("keeps Unix paths case-sensitive", () => {
    expect(normalizeCwd("/home/Me/proj/")).toBe("/home/Me/proj");
    expect(normalizeCwd("/home/me/proj")).not.toBe(normalizeCwd("/home/Me/proj"));
  });

  it("frontendCwd keeps case but fixes separators and the verbatim prefix", () => {
    expect(frontendCwd("\\\\?\\C:\\Users\\Me\\proj\\")).toBe("C:/Users/Me/proj");
    expect(frontendCwd("/")).toBe("/");
    const [card] = buildResumeCards([pane({ cwd: "C:\\Users\\me\\proj" })], {
      now: T0,
    });
    expect(card.cwd).toBe("C:/Users/me/proj");
  });
});

describe("buildResumeCards", () => {
  it("shapes, sorts by recency, and drops hidden or agent-less panes", () => {
    const now = T0 + 5 * 60_000;
    const cards = buildResumeCards(
      [
        pane({ key: "old", last_ts: T0 - 3 * 3_600_000 }),
        pane({ key: "new", claude_session_id: null }),
        pane({ key: "shell", agent: null }),
        pane({ key: "gone" }),
      ],
      { now, home: "C:\\Users\\me", hidden: new Set(["gone"]) },
    );
    expect(cards.map((c) => c.key)).toEqual(["new", "old"]);
    expect(cards[0]).toMatchObject({
      cwdShort: "~/proj",
      agentLabel: "Claude Code",
      sessionShort: null,
      resumable: false,
      lastActivity: "5 min ago",
    });
    expect(cards[1]).toMatchObject({
      sessionShort: "0198d2fc",
      resumable: true,
      lastActivity: "3 h ago",
    });
  });

  it("shortens deep paths and formats relative time", () => {
    expect(shortenCwd("/a/b/c/d/e")).toBe(".../c/d/e");
    expect(shortenCwd("/a/b")).toBe("/a/b");
    expect(shortenCwd("/home/me/x", "/home/me")).toBe("~/x");
    expect(shortenCwd("/home/meow/x", "/home/me")).toBe("/home/meow/x");
    expect(relativeTime(0, 10)).toBe("");
    expect(relativeTime(10_000, 20_000)).toBe("just now");
    expect(relativeTime(0, 2 * 86_400_000)).toBe("");
    expect(relativeTime(1, 2 * 86_400_000)).toBe("2 d ago");
  });
});

describe("matchRecoveredPanes", () => {
  const leaves = [
    { leafId: 11, tabId: 1, cwd: "C:/Users/me/proj" },
    { leafId: 12, tabId: 2, cwd: "C:/Users/me/proj" },
    { leafId: 13, tabId: 3, cwd: "C:/Users/me/other" },
    { leafId: 14, tabId: 4, cwd: undefined },
  ];

  it("matches by normalized cwd, one leaf per pane, in order", () => {
    const m = matchRecoveredPanes(
      [
        pane({ key: "a", cwd: "C:\\Users\\me\\proj" }),
        pane({ key: "b", cwd: "c:/users/me/PROJ/" }),
        pane({ key: "c", cwd: "C:/Users/me/proj" }),
      ],
      leaves,
    );
    expect(m.map((x) => [x.pane.key, x.leafId, x.tabId])).toEqual([
      ["a", 11, 1],
      ["b", 12, 2],
    ]);
  });

  it("only matches panes that can actually resume (claude + captured id)", () => {
    const m = matchRecoveredPanes(
      [
        pane({ key: "noid", claude_session_id: null }),
        pane({ key: "codex", agent: "codex", cwd: "C:/Users/me/other" }),
        pane({ key: "ok", cwd: "C:/Users/me/other" }),
      ],
      leaves,
    );
    expect(m.map((x) => x.pane.key)).toEqual(["ok"]);
    expect(m[0].leafId).toBe(13);
  });

  it("never matches a leaf without a cwd", () => {
    expect(
      matchRecoveredPanes([pane({ cwd: "" })], [{ leafId: 1, tabId: 1, cwd: "" }]),
    ).toEqual([]);
  });
});

describe("resumeCommandFor", () => {
  it("uses the Tier-2 command verbatim, falls back to a plain claude launch, else nothing", () => {
    const p = pane();
    expect(
      resumeCommandFor({ tier: "tier2", command: "claude --resume x" }, p, "claude"),
    ).toBe("claude --resume x");
    expect(resumeCommandFor({ tier: "tier1", cwd: p.cwd }, p, "claude")).toBe(
      "claude",
    );
    expect(resumeCommandFor(null, p, "claude")).toBe("claude");
    expect(
      resumeCommandFor(null, pane({ agent: "codex" }), "claude"),
    ).toBeNull();
  });
});

describe("recoveredLauncherSections", () => {
  it("is empty with no cards and otherwise one section wired to resume", () => {
    expect(recoveredLauncherSections([], () => {})).toEqual([]);
    const cards = buildResumeCards([pane()], {
      now: T0 + 1000,
      home: "C:/Users/me",
    });
    const picked: string[] = [];
    const [section] = recoveredLauncherSections(cards, (k) => picked.push(k));
    expect(section.id).toBe("resume");
    expect(section.items).toHaveLength(1);
    expect(section.items[0]).toMatchObject({
      id: "resume:k1",
      label: "Claude Code in proj",
      description: "~/proj",
      hint: "just now",
      badge: "0198d2fc",
    });
    section.items[0].onSelect();
    expect(picked).toEqual(["k1"]);
  });
});
