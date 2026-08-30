import { type RefObject, useCallback, useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { native } from "@/modules/ai/lib/native";
import type { Tab } from "@/modules/tabs";
import {
  getSshHome,
  getWslHome,
  LOCAL_WORKSPACE,
  type WorkspaceEnv,
  workspaceScopeKey,
} from "@/modules/workspace";

type Params = {
  tabsRef: RefObject<Tab[]>;
  workspaceEnv: WorkspaceEnv;
  setWorkspaceEnv: (env: WorkspaceEnv) => void;
  resetWorkspace: (home?: string) => void;
  /** Dispose live sessions and clear App-owned pane/handle ref maps. */
  clearWorkspaceState: () => void;
};

/**
 * Owns the resolved home / launch cwd and the local⇄WSL workspace switch. The
 * switch tears down live sessions (via clearWorkspaceState), re-authorizes the
 * new home, and resets the tab workspace.
 */
export function useWorkspaceSwitcher({
  tabsRef,
  workspaceEnv,
  setWorkspaceEnv,
  resetWorkspace,
  clearWorkspaceState,
}: Params) {
  const [home, setHome] = useState<string | null>(null);
  const [launchCwd, setLaunchCwd] = useState<string | null>(null);
  const [launchCwdResolved, setLaunchCwdResolved] = useState(false);

  useEffect(() => {
    homeDir()
      .then(async (p) => {
        const normalized = p.replace(/\\/g, "/");
        setHome(normalized);
        try {
          await native.workspaceAuthorize(normalized);
        } catch {
          // Bootstrap already authorizes home from Rust; ignore.
        }
      })
      .catch(() => setHome(null));
  }, []);

  useEffect(() => {
    native
      .workspaceCurrentDir()
      .then(setLaunchCwd)
      .catch(() => setLaunchCwd(null))
      .finally(() => setLaunchCwdResolved(true));
  }, []);

  const switchWorkspace = useCallback(
    async (env: WorkspaceEnv) => {
      const same =
        workspaceScopeKey(env) === workspaceScopeKey(workspaceEnv) &&
        (env.kind !== "ssh" ||
          workspaceEnv.kind !== "ssh" ||
          env.path === workspaceEnv.path);
      if (same) return;
      const dirty = tabsRef.current.some((t) => t.kind === "editor" && t.dirty);
      if (dirty) {
        window.alert(
          "Save or close unsaved editor tabs before switching workspace.",
        );
        return;
      }

      let nextHome: string | null = null;
      try {
        if (env.kind === "wsl") {
          nextHome = await getWslHome(env.distro);
        } else if (env.kind === "ssh") {
          nextHome =
            env.path.trim() !== "" ? env.path : await getSshHome(env.host);
        } else {
          nextHome = (await homeDir()).replace(/\\/g, "/");
        }
      } catch (e) {
        window.alert(String(e));
        return;
      }

      // An ssh env picked without a path settles on the remote home, so the
      // Space that persists it reconnects to the same place.
      const nextEnv: WorkspaceEnv =
        env.kind === "local"
          ? LOCAL_WORKSPACE
          : env.kind === "ssh"
            ? { kind: "ssh", host: env.host, path: nextHome ?? "" }
            : env;

      clearWorkspaceState();
      setWorkspaceEnv(nextEnv);
      setHome(nextHome);
      setLaunchCwd(nextHome);
      if (nextHome) {
        try {
          await native.workspaceAuthorize(nextHome);
        } catch {
          // Non-fatal — git panel will surface "not authorized" if needed.
        }
      }
      resetWorkspace(nextHome ?? undefined);
    },
    [
      workspaceEnv,
      setWorkspaceEnv,
      resetWorkspace,
      tabsRef,
      clearWorkspaceState,
    ],
  );

  return { home, launchCwd, launchCwdResolved, switchWorkspace };
}
