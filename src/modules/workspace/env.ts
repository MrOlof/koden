import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { setLastWslDistro } from "@/modules/settings/store";

export type WorkspaceEnv =
  | { kind: "local" }
  | { kind: "wsl"; distro: string }
  | { kind: "ssh"; host: string; path: string };

export type WslDistro = {
  name: string;
  default: boolean;
  running: boolean;
};

export type SshHost = {
  alias: string;
  hostName?: string;
  user?: string;
  port?: number;
};

/** Same text Rust returns from its gate, so the UI can short-circuit without
 * a round-trip and still read identically. */
export const SSH_LOCAL_FS_UNAVAILABLE =
  "Files, git and search are not available for SSH workspaces yet";

type State = {
  env: WorkspaceEnv;
  distros: WslDistro[];
  loading: boolean;
  error: string | null;
  sshHosts: SshHost[];
  sshLoading: boolean;
  sshError: string | null;
  setEnv: (env: WorkspaceEnv) => void;
  refreshDistros: () => Promise<WslDistro[]>;
  refreshSshHosts: () => Promise<SshHost[]>;
};

export const LOCAL_WORKSPACE: WorkspaceEnv = { kind: "local" };

export const useWorkspaceEnvStore = create<State>((set) => ({
  env: LOCAL_WORKSPACE,
  distros: [],
  loading: false,
  error: null,
  sshHosts: [],
  sshLoading: false,
  sshError: null,
  setEnv: (env) => {
    set({ env });
    if (env.kind === "wsl") void setLastWslDistro(env.distro);
  },
  refreshDistros: async () => {
    set({ loading: true, error: null });
    try {
      const distros = await invoke<WslDistro[]>("wsl_list_distros");
      set({ distros, loading: false });
      return distros;
    } catch (e) {
      set({ distros: [], loading: false, error: String(e) });
      return [];
    }
  },
  refreshSshHosts: async () => {
    set({ sshLoading: true, sshError: null });
    try {
      const sshHosts = await invoke<SshHost[]>("ssh_list_hosts");
      set({ sshHosts, sshLoading: false });
      return sshHosts;
    } catch (e) {
      set({ sshHosts: [], sshLoading: false, sshError: String(e) });
      return [];
    }
  },
}));

export function currentWorkspaceEnv(): WorkspaceEnv {
  return useWorkspaceEnvStore.getState().env;
}

export function isSshWorkspace(
  env: WorkspaceEnv,
): env is Extract<WorkspaceEnv, { kind: "ssh" }> {
  return env.kind === "ssh";
}

export function workspaceScopeKey(env: WorkspaceEnv): string {
  if (env.kind === "ssh") return `ssh:${env.host}`;
  return env.kind === "wsl" ? `wsl:${env.distro}` : "local";
}

export function workspaceEnvLabel(
  env: WorkspaceEnv,
  localLabel: string,
): string {
  if (env.kind === "ssh") return `ssh: ${env.host}`;
  return env.kind === "wsl" ? `WSL: ${env.distro}` : localLabel;
}

export function currentWorkspaceScopeKey(): string {
  return workspaceScopeKey(currentWorkspaceEnv());
}

export async function getWslHome(distro: string): Promise<string> {
  return invoke<string>("wsl_home", { distro });
}

export async function getSshHome(host: string): Promise<string> {
  return invoke<string>("ssh_home", { host });
}
