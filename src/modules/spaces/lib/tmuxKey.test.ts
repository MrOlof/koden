import type { WorkspaceEnv } from "@/modules/workspace";
import { describe, expect, it } from "vitest";
import type { SpaceMeta } from "./store";
import { tmuxKeyFor } from "./tmuxKey";

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

describe("tmuxKeyFor", () => {
  it("returns the Space id for an ssh Space that opted in", () => {
    expect(tmuxKeyFor(space({ id: "sp-1", env: SSH, sshTmux: true }))).toBe(
      "sp-1",
    );
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
