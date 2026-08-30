import type { WorkspaceEnv } from "@/modules/workspace";
import { describe, expect, it } from "vitest";
import {
  envEquals,
  resolveEnvSwitch,
  spaceMatchesEnv,
  spaceNameForEnv,
} from "./envSwitch";
import type { SpaceMeta } from "./store";

const LOCAL: WorkspaceEnv = { kind: "local" };
const UBUNTU: WorkspaceEnv = { kind: "wsl", distro: "Ubuntu" };
const DOCKER: WorkspaceEnv = { kind: "ssh", host: "docker", path: "/home/k" };

function space(
  over: Partial<SpaceMeta> & { id: string; env?: WorkspaceEnv },
): SpaceMeta {
  return {
    name: over.id,
    root: `/root/${over.id}`,
    env: LOCAL,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("envEquals", () => {
  it("compares scope for local and WSL, scope plus path for ssh", () => {
    expect(envEquals(LOCAL, { kind: "local" })).toBe(true);
    expect(envEquals(UBUNTU, { kind: "wsl", distro: "Debian" })).toBe(false);
    expect(envEquals(DOCKER, { ...DOCKER })).toBe(true);
    expect(envEquals(DOCKER, { ...DOCKER, path: "/srv" })).toBe(false);
    expect(envEquals(DOCKER, LOCAL)).toBe(false);
  });
});

describe("spaceMatchesEnv", () => {
  it("treats a Space without env as local", () => {
    const legacy = { ...space({ id: "old" }), env: undefined } as unknown as SpaceMeta;
    expect(spaceMatchesEnv(legacy, LOCAL)).toBe(true);
    expect(spaceMatchesEnv(legacy, UBUNTU)).toBe(false);
  });

  it("matches any Space on the host when the target names no path", () => {
    const s = space({ id: "d", env: DOCKER });
    expect(spaceMatchesEnv(s, { kind: "ssh", host: "docker", path: "" })).toBe(
      true,
    );
    expect(spaceMatchesEnv(s, { ...DOCKER, path: "/srv" })).toBe(false);
    expect(spaceMatchesEnv(s, { kind: "ssh", host: "lab", path: "" })).toBe(
      false,
    );
  });
});

describe("spaceNameForEnv", () => {
  it("names local by platform, WSL by distro, ssh by host", () => {
    expect(spaceNameForEnv(LOCAL, "Windows")).toBe("Windows");
    expect(spaceNameForEnv(UBUNTU, "Windows")).toBe("WSL: Ubuntu");
    expect(spaceNameForEnv(DOCKER, "Windows")).toBe("docker");
  });
});

describe("resolveEnvSwitch", () => {
  const spaces = [
    space({ id: "win-old", updatedAt: 1 }),
    space({ id: "win-new", updatedAt: 5 }),
    space({ id: "docker", env: DOCKER, updatedAt: 3 }),
    space({ id: "docker-srv", env: { ...DOCKER, path: "/srv" }, updatedAt: 9 }),
  ];

  it("keeps the active Space when it already has the env", () => {
    expect(resolveEnvSwitch(LOCAL, spaces, "win-old")).toEqual({
      action: "switch",
      id: "win-old",
    });
  });

  it("switches to the most recently used Space of the env", () => {
    expect(resolveEnvSwitch(LOCAL, spaces, "docker")).toEqual({
      action: "switch",
      id: "win-new",
    });
    expect(
      resolveEnvSwitch({ kind: "ssh", host: "docker", path: "" }, spaces, "win-new"),
    ).toEqual({ action: "switch", id: "docker-srv" });
    expect(resolveEnvSwitch(DOCKER, spaces, "win-new")).toEqual({
      action: "switch",
      id: "docker",
    });
  });

  it("creates a Space named after the env when none matches", () => {
    expect(resolveEnvSwitch(UBUNTU, spaces, "win-new", "Windows")).toEqual({
      action: "create",
      meta: { name: "WSL: Ubuntu", env: UBUNTU },
    });
    const lab: WorkspaceEnv = { kind: "ssh", host: "lab", path: "" };
    expect(resolveEnvSwitch(lab, spaces, "win-new", "Windows")).toEqual({
      action: "create",
      meta: { name: "lab", env: lab },
    });
    expect(resolveEnvSwitch(LOCAL, [], null, "Windows")).toEqual({
      action: "create",
      meta: { name: "Windows", env: LOCAL },
    });
  });

  it("never mutates the current Space: a different env always leaves it", () => {
    const r = resolveEnvSwitch(LOCAL, spaces, "docker");
    expect(r.action).toBe("switch");
    expect(r.action === "switch" && r.id).not.toBe("docker");
  });

  it("carries the tmux flag to the Space it lands on, ssh only", () => {
    const lab: WorkspaceEnv = { kind: "ssh", host: "lab", path: "" };
    expect(
      resolveEnvSwitch(lab, spaces, "win-new", "Windows", { sshTmux: true }),
    ).toEqual({
      action: "create",
      meta: { name: "lab", env: lab, sshTmux: true },
    });
    expect(
      resolveEnvSwitch(DOCKER, spaces, "win-new", "Windows", { sshTmux: true }),
    ).toEqual({ action: "switch", id: "docker", sshTmux: true });
    expect(
      resolveEnvSwitch(DOCKER, spaces, "docker", "Windows", { sshTmux: false }),
    ).toEqual({ action: "switch", id: "docker", sshTmux: false });
    expect(
      resolveEnvSwitch(UBUNTU, spaces, "win-new", "Windows", { sshTmux: true }),
    ).toEqual({
      action: "create",
      meta: { name: "WSL: Ubuntu", env: UBUNTU },
    });
    expect(resolveEnvSwitch(DOCKER, spaces, "win-new", "Windows", {})).toEqual({
      action: "switch",
      id: "docker",
    });
  });
});
