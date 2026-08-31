import type { WorkspaceEnv } from "@/modules/workspace";
import { describe, expect, it } from "vitest";
import type { SpaceMeta } from "./store";
import { pathTmuxKey, tmuxKeyFor } from "./tmuxKey";

const SSH: WorkspaceEnv = { kind: "ssh", host: "lab", path: "/home/k" };
const WSL: WorkspaceEnv = { kind: "wsl", distro: "Ubuntu" };

function space(over: Partial<SpaceMeta> & { id: string }): SpaceMeta {
  return {
    name: over.id,
    root: null,
    env: { kind: "local" },
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("pathTmuxKey", () => {
  it("is deterministic and device-independent for the same path", () => {
    expect(pathTmuxKey("/home/k")).toBe(pathTmuxKey("/home/k"));
    expect(pathTmuxKey("/home/k")).not.toBe(pathTmuxKey("/home/k2"));
  });

  it("stays inside the tmux-safe charset and length budget", () => {
    const key = pathTmuxKey("/home/snorlax/My Projects/x!");
    expect(key).toMatch(/^p[a-z0-9]+-[A-Za-z0-9_-]+$/);
    expect(key.length).toBeLessThanOrEqual(40);
    expect(key.endsWith("-")).toBe(false);
    expect(pathTmuxKey(`/${"x".repeat(200)}`).length).toBeLessThanOrEqual(40);
  });
});

describe("tmuxKeyFor", () => {
  it("keys an opted-in ssh Space by its remote path, not the Space id", () => {
    const a = tmuxKeyFor(space({ id: "sp-hq", env: SSH, sshTmux: true }));
    const b = tmuxKeyFor(space({ id: "sp-laptop", env: SSH, sshTmux: true }));
    expect(a).toBe(pathTmuxKey("/home/k"));
    // Different devices mint different Space ids for the same workspace;
    // the tmux key must agree anyway.
    expect(b).toBe(a);
  });

  it("returns undefined for an ssh Space without the flag", () => {
    expect(tmuxKeyFor(space({ id: "sp-1", env: SSH }))).toBeUndefined();
    expect(
      tmuxKeyFor(space({ id: "sp-1", env: SSH, sshTmux: false })),
    ).toBeUndefined();
  });

  it("ignores the flag on local and WSL Spaces", () => {
    expect(tmuxKeyFor(space({ id: "sp-1", sshTmux: true }))).toBeUndefined();
    expect(
      tmuxKeyFor(space({ id: "sp-1", env: WSL, sshTmux: true })),
    ).toBeUndefined();
    const legacy = {
      ...space({ id: "old", sshTmux: true }),
      env: undefined,
    } as unknown as SpaceMeta;
    expect(tmuxKeyFor(legacy)).toBeUndefined();
  });

  it("returns undefined when there is no active Space", () => {
    expect(tmuxKeyFor(undefined)).toBeUndefined();
  });
});
