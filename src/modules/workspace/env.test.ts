import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  setLastWslDistro: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/modules/settings/store", () => ({
  setLastWslDistro: mocks.setLastWslDistro,
}));

import {
  getSshHome,
  isSshWorkspace,
  LOCAL_WORKSPACE,
  SSH_LOCAL_FS_UNAVAILABLE,
  type SshHost,
  useWorkspaceEnvStore,
  type WorkspaceEnv,
  workspaceEnvLabel,
  workspaceScopeKey,
} from "./env";

const SSH: WorkspaceEnv = { kind: "ssh", host: "workbench", path: "/srv/app" };

describe("workspace env helpers", () => {
  it("scopes ssh by host only, so every path on a host shares state", () => {
    expect(workspaceScopeKey(SSH)).toBe("ssh:workbench");
    expect(workspaceScopeKey({ ...SSH, path: "/other" })).toBe("ssh:workbench");
    expect(workspaceScopeKey({ kind: "wsl", distro: "Ubuntu" })).toBe(
      "wsl:Ubuntu",
    );
    expect(workspaceScopeKey(LOCAL_WORKSPACE)).toBe("local");
  });

  it("labels each env kind", () => {
    expect(workspaceEnvLabel(SSH, "Windows")).toBe("ssh: workbench");
    expect(workspaceEnvLabel({ kind: "wsl", distro: "Ubuntu" }, "Windows")).toBe(
      "WSL: Ubuntu",
    );
    expect(workspaceEnvLabel(LOCAL_WORKSPACE, "Local")).toBe("Local");
  });

  it("narrows ssh envs", () => {
    expect(isSshWorkspace(SSH)).toBe(true);
    expect(isSshWorkspace(LOCAL_WORKSPACE)).toBe(false);
    expect(SSH_LOCAL_FS_UNAVAILABLE).toMatch(/SSH workspaces/);
  });
});

describe("useWorkspaceEnvStore ssh hosts", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.setLastWslDistro.mockReset();
    useWorkspaceEnvStore.setState({
      env: LOCAL_WORKSPACE,
      sshHosts: [],
      sshLoading: false,
      sshError: null,
    });
  });

  it("loads hosts from ssh_list_hosts", async () => {
    const hosts: SshHost[] = [
      { alias: "workbench", hostName: "10.0.0.5", user: "kosta" },
      { alias: "docker" },
    ];
    mocks.invoke.mockResolvedValueOnce(hosts);
    const result = await useWorkspaceEnvStore.getState().refreshSshHosts();
    expect(mocks.invoke).toHaveBeenCalledWith("ssh_list_hosts");
    expect(result).toEqual(hosts);
    const s = useWorkspaceEnvStore.getState();
    expect(s.sshHosts).toEqual(hosts);
    expect(s.sshLoading).toBe(false);
    expect(s.sshError).toBeNull();
  });

  it("records the error and clears hosts when listing fails", async () => {
    useWorkspaceEnvStore.setState({ sshHosts: [{ alias: "stale" }] });
    mocks.invoke.mockRejectedValueOnce(new Error("boom"));
    const result = await useWorkspaceEnvStore.getState().refreshSshHosts();
    expect(result).toEqual([]);
    const s = useWorkspaceEnvStore.getState();
    expect(s.sshHosts).toEqual([]);
    expect(s.sshError).toContain("boom");
  });

  it("resolves the remote home through ssh_home", async () => {
    mocks.invoke.mockResolvedValueOnce("/home/kosta");
    await expect(getSshHome("workbench")).resolves.toBe("/home/kosta");
    expect(mocks.invoke).toHaveBeenCalledWith("ssh_home", {
      host: "workbench",
    });
  });

  it("does not persist an ssh env as the last WSL distro", () => {
    useWorkspaceEnvStore.getState().setEnv(SSH);
    expect(useWorkspaceEnvStore.getState().env).toEqual(SSH);
    expect(mocks.setLastWslDistro).not.toHaveBeenCalled();
  });
});
