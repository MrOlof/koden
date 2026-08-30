import { type RefObject, useCallback, useEffect, useRef, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { toast } from "sonner";
import { LOCAL_ENV_LABEL } from "@/lib/platform";
import { native } from "@/modules/ai/lib/native";
import {
  envEquals,
  resolveEnvSwitch,
  spaceEnv,
  spaceNameForEnv,
} from "@/modules/spaces/lib/envSwitch";
import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import type { Tab } from "@/modules/tabs";
import {
  getSshHome,
  getWslHome,
  LOCAL_WORKSPACE,
  type WorkspaceEnv,
} from "@/modules/workspace";

type Params = {
  tabsRef: RefObject<Tab[]>;
  setWorkspaceEnv: (env: WorkspaceEnv) => void;
  setActiveSpaceForNewTabs: (id: string) => void;
  newTab: (cwd?: string) => number;
};

async function resolveEnvHome(env: WorkspaceEnv): Promise<string> {
  if (env.kind === "wsl") return getWslHome(env.distro);
  if (env.kind === "ssh") {
    return env.path.trim() !== "" ? env.path : getSshHome(env.host);
  }
  return (await homeDir()).replace(/\\/g, "/");
}

/**
 * Env is a property of the Space: the global workspace env, home and launch
 * cwd follow the active Space, and asking for another env goes to a Space of
 * that env (existing or new) instead of mutating the current one. Nothing
 * here disposes a session; tabs of inactive Spaces stay parked as they are.
 */
export function useWorkspaceSwitcher({
  tabsRef,
  setWorkspaceEnv,
  setActiveSpaceForNewTabs,
  newTab,
}: Params) {
  const [localHome, setLocalHome] = useState<string | null>(null);
  const [home, setHome] = useState<string | null>(null);
  const [launchCwd, setLaunchCwd] = useState<string | null>(null);
  const [launchCwdResolved, setLaunchCwdResolved] = useState(false);
  // The env `home` reflects. gen guards against a slow ssh_home landing after
  // the user already moved to another Space.
  const applied = useRef<{ env: WorkspaceEnv; gen: number }>({
    env: LOCAL_WORKSPACE,
    gen: 0,
  });

  const activeSpaceEnv = useSpaces((s) => {
    if (!s.hydrated || s.activeId === null) return null;
    const active = s.spaces.find((x) => x.id === s.activeId);
    return active ? spaceEnv(active) : null;
  });

  useEffect(() => {
    homeDir()
      .then(async (p) => {
        const normalized = p.replace(/\\/g, "/");
        setLocalHome(normalized);
        if (applied.current.gen === 0) setHome(normalized);
        try {
          await native.workspaceAuthorize(normalized);
        } catch {
          // Bootstrap already authorizes home from Rust; ignore.
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    native
      .workspaceCurrentDir()
      .then(setLaunchCwd)
      .catch(() => setLaunchCwd(null))
      .finally(() => setLaunchCwdResolved(true));
  }, []);

  // The env goes live synchronously (before the first await): a tab opened
  // right after reads it at PTY spawn.
  const applyEnv = useCallback(
    async (env: WorkspaceEnv) => {
      if (envEquals(env, applied.current.env)) return;
      const gen = applied.current.gen + 1;
      applied.current = { env, gen };
      setWorkspaceEnv(env);
      let nextHome: string | null;
      try {
        nextHome = await resolveEnvHome(env);
      } catch {
        nextHome = null;
      }
      if (applied.current.gen !== gen) return;
      setHome(nextHome);
      setLaunchCwd(nextHome);
      if (nextHome) native.workspaceAuthorize(nextHome).catch(() => {});
    },
    [setWorkspaceEnv],
  );

  useEffect(() => {
    if (activeSpaceEnv) void applyEnv(activeSpaceEnv);
  }, [activeSpaceEnv, applyEnv]);

  /** Go to a Space of `env`; resolves true when the active Space changed. */
  const switchToEnv = useCallback(
    async (env: WorkspaceEnv): Promise<boolean> => {
      const { spaces, activeId, create, setActive } = useSpaces.getState();
      const plan = resolveEnvSwitch(env, spaces, activeId, LOCAL_ENV_LABEL);
      if (plan.action === "switch") {
        if (plan.id === activeId) return false;
        const target = spaces.find((s) => s.id === plan.id);
        if (!target) return false;
        void applyEnv(spaceEnv(target));
        const hasTerminal = tabsRef.current.some(
          (t) => t.spaceId === plan.id && t.kind === "terminal",
        );
        if (!hasTerminal) {
          setActiveSpaceForNewTabs(plan.id);
          newTab(target.root ?? undefined);
        }
        setActive(plan.id);
        return true;
      }
      let root: string;
      try {
        root = await resolveEnvHome(env);
      } catch (e) {
        toast.error(
          `Could not open ${spaceNameForEnv(env, LOCAL_ENV_LABEL)}: ${String(e)}`,
        );
        return false;
      }
      // An ssh env picked without a path settles on the remote home, so the
      // Space reconnects to the same place next time.
      const nextEnv: WorkspaceEnv =
        env.kind === "ssh"
          ? { kind: "ssh", host: env.host, path: root }
          : env.kind === "local"
            ? LOCAL_WORKSPACE
            : env;
      const meta = create({ name: plan.meta.name, root, env: nextEnv });
      void applyEnv(nextEnv);
      setActiveSpaceForNewTabs(meta.id);
      newTab(root);
      setActive(meta.id);
      return true;
    },
    [applyEnv, tabsRef, setActiveSpaceForNewTabs, newTab],
  );

  return { home, localHome, launchCwd, launchCwdResolved, switchToEnv };
}
