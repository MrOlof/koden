import { describe, expect, it } from "vitest";
import {
  deriveBranch,
  formatSymlinkPaths,
  isPlausibleBranchName,
  nextFreeColorIndex,
  orderBases,
  parseSymlinkPaths,
  planWorktreeAdd,
  worktreePathFor,
} from "./worktreeModel";

describe("deriveBranch", () => {
  it("prefixes the slug", () => {
    expect(deriveBranch("Fix Login")).toBe("feat/fix-login");
  });
  it("is empty when the name has no usable characters", () => {
    expect(deriveBranch("   ")).toBe("");
  });
});

describe("worktreePathFor", () => {
  it("nests under .koden/worktrees with forward slashes", () => {
    expect(worktreePathFor("C:\\repo\\", "x")).toBe(
      "C:/repo/.koden/worktrees/x",
    );
    expect(worktreePathFor("/home/u/repo", "y")).toBe(
      "/home/u/repo/.koden/worktrees/y",
    );
  });
});

describe("orderBases", () => {
  it("puts current first, then locals, then remotes, deduped", () => {
    expect(
      orderBases({
        current: "main",
        local: ["feature", "main"],
        remote: ["origin/main", "origin/feature"],
      }),
    ).toEqual(["main", "feature", "origin/main", "origin/feature"]);
  });
  it("copes with a detached head", () => {
    expect(orderBases({ current: null, local: ["a"], remote: [] })).toEqual([
      "a",
    ]);
  });
});

describe("planWorktreeAdd", () => {
  it("creates a new branch off the base by default", () => {
    expect(planWorktreeAdd("feat/x", "main", ["main"])).toEqual({
      newBranch: "feat/x",
      base: "main",
    });
  });
  it("checks out an existing local branch instead of recreating it", () => {
    expect(planWorktreeAdd(" existing ", "main", ["main", "existing"])).toEqual(
      { newBranch: null, base: "existing" },
    );
  });
});

describe("isPlausibleBranchName", () => {
  it("accepts ordinary names", () => {
    for (const ok of ["main", "feat/x", "release-1.2", "a/b/c"]) {
      expect(isPlausibleBranchName(ok), ok).toBe(true);
    }
  });
  it("rejects option-looking and malformed names", () => {
    for (const bad of [
      "",
      "-b",
      "a b",
      "a..b",
      "x/",
      "/x",
      "x.lock",
      "HEAD",
      "a@{1}",
      ".hidden",
      "a//b",
      "tab\there",
      "x:y",
    ]) {
      expect(isPlausibleBranchName(bad), bad).toBe(false);
    }
  });
});

describe("nextFreeColorIndex", () => {
  it("picks the first unused index", () => {
    expect(nextFreeColorIndex([0, 1, undefined], 4)).toBe(2);
  });
  it("falls back to the least used index when all are taken", () => {
    expect(nextFreeColorIndex([0, 0, 1, 2, 3], 4)).toBe(1);
  });
  it("ignores out-of-range entries", () => {
    expect(nextFreeColorIndex([99, -1], 3)).toBe(0);
  });
});

describe("parseSymlinkPaths", () => {
  it("splits on commas and newlines, trims and dedupes", () => {
    expect(parseSymlinkPaths(" node_modules, ./.venv\nnode_modules/ ")).toEqual(
      ["node_modules", ".venv"],
    );
  });
  it("drops escapes and empties", () => {
    expect(parseSymlinkPaths("../x, , a/../b, /abs/, c\\d")).toEqual([
      "abs",
      "c/d",
    ]);
  });
  it("round-trips through formatSymlinkPaths", () => {
    const list = ["node_modules", ".venv"];
    expect(parseSymlinkPaths(formatSymlinkPaths(list))).toEqual(list);
  });
});
