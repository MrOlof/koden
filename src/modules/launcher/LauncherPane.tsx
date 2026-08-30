import { Kbd } from "@/components/ui/kbd";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Wordmark } from "@/components/Wordmark";
import { IS_WINDOWS, LOCAL_ENV_LABEL } from "@/lib/platform";
import { useShortcutLabel } from "@/modules/shortcuts";
import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import { useWorkspaceEnvStore, type WorkspaceEnv } from "@/modules/workspace";
import {
  ComputerTerminal02Icon,
  FolderOpenIcon,
  RocketIcon,
  TerminalIcon,
} from "@hugeicons/core-free-icons";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { LauncherSection, LauncherSectionTitle } from "./LauncherSection";
import {
  buildLauncherSections,
  LAUNCHER_SECTION_IDS,
  type LauncherIcons,
  type LauncherSectionModel,
  type SshEnv,
  type SshHost,
} from "./lib/launcherItems";
import { useLauncherKeys } from "./lib/useLauncherKeys";
import { RemoteConnectForm } from "./RemoteConnectForm";

export type LauncherFocusTarget = "remote";

type Props = {
  /** Where "Terminal here" opens; shown under the item. */
  localCwd: string | null;
  /** Land focus somewhere specific on mount (the palette's connect command). */
  initialFocus?: LauncherFocusTarget | null;
  onFocusHandled?: () => void;
  onSwitchSpace: (spaceId: string) => void;
  onNewTerminal: (env: WorkspaceEnv) => void;
  onOpenFolder: () => void;
  onConnectRemote: (env: SshEnv) => Promise<void> | void;
  onOpenSetup: () => void;
  /** Extra list sections, rendered right after "Continue". */
  extraSections?: LauncherSectionModel[];
};

const ICONS: LauncherIcons = {
  terminal: ComputerTerminal02Icon,
  wsl: TerminalIcon,
  folder: FolderOpenIcon,
  setup: RocketIcon,
};

/**
 * The "What do you want to do?" page: continue a Space, open a terminal or a
 * folder, connect to a remote host, or run setup. Lives in a `launcher` tab
 * (never persisted) and is also the content of any Space with no tabs.
 */
export function LauncherPane({
  localCwd,
  initialFocus = null,
  onFocusHandled,
  onSwitchSpace,
  onNewTerminal,
  onOpenFolder,
  onConnectRemote,
  onOpenSetup,
  extraSections,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const hostInputRef = useRef<HTMLInputElement>(null);
  const spaces = useSpaces((s) => s.spaces);
  const activeSpaceId = useSpaces((s) => s.activeId);
  const distros = useWorkspaceEnvStore((s) => s.distros);
  const refreshDistros = useWorkspaceEnvStore((s) => s.refreshDistros);
  const newTabShortcut = useShortcutLabel("tab.new");
  const [hosts, setHosts] = useState<SshHost[] | null>(null);

  useLauncherKeys(rootRef, { focusFirst: initialFocus === null });

  useEffect(() => {
    if (initialFocus !== "remote") return;
    const raf = requestAnimationFrame(() => {
      hostInputRef.current?.focus();
      onFocusHandled?.();
    });
    return () => cancelAnimationFrame(raf);
  }, [initialFocus, onFocusHandled]);

  useEffect(() => {
    if (IS_WINDOWS) void refreshDistros();
  }, [refreshDistros]);

  // ssh_list_hosts may not exist yet on this build; the form stays free-text.
  useEffect(() => {
    let alive = true;
    invoke<SshHost[]>("ssh_list_hosts")
      .then((list) => {
        if (alive) setHosts(Array.isArray(list) ? list : []);
      })
      .catch(() => {
        if (alive) setHosts([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  const sections = useMemo(
    () =>
      buildLauncherSections(
        {
          spaces,
          activeSpaceId,
          distros,
          isWindows: IS_WINDOWS,
          localLabel: LOCAL_ENV_LABEL,
          localCwd,
          newTabShortcut,
          icons: ICONS,
        },
        {
          switchSpace: onSwitchSpace,
          newTerminal: onNewTerminal,
          openFolder: onOpenFolder,
          openSetup: onOpenSetup,
        },
      ),
    [
      spaces,
      activeSpaceId,
      distros,
      localCwd,
      newTabShortcut,
      onSwitchSpace,
      onNewTerminal,
      onOpenFolder,
      onOpenSetup,
    ],
  );

  // Extra sections slot in right after Continue; the remote form sits just
  // before Set up so every list stays one arrow-key sequence.
  const continueSection = sections.find(
    (s) => s.id === LAUNCHER_SECTION_IDS.continue,
  );
  const setupSection = sections.find(
    (s) => s.id === LAUNCHER_SECTION_IDS.setup,
  );
  const middle = sections.filter(
    (s) => s !== continueSection && s !== setupSection,
  );

  return (
    <ScrollArea className="h-full">
      <div
        ref={rootRef}
        className="koden-panel-in mx-auto flex w-full max-w-2xl flex-col gap-7 px-6 pt-10 pb-12"
      >
        <header className="flex flex-col gap-2 px-3">
          <Wordmark className="text-[13px] text-muted-foreground" />
          <h1 className="text-[22px] font-medium leading-tight tracking-tight text-foreground">
            What do you want to do?
          </h1>
          <p className="text-xs text-muted-foreground">
            Pick up a Space, open a terminal or a folder, or connect somewhere
            new.
          </p>
        </header>

        {continueSection ? <LauncherSection section={continueSection} /> : null}
        {extraSections?.map((s) => (
          <LauncherSection key={s.id} section={s} />
        ))}
        {middle.map((s) => (
          <LauncherSection key={s.id} section={s} />
        ))}

        <section
          aria-label="Connect to a remote host"
          className="flex flex-col gap-2"
        >
          <LauncherSectionTitle>Connect to a remote host</LauncherSectionTitle>
          <RemoteConnectForm
            hosts={hosts}
            onConnect={onConnectRemote}
            hostInputRef={hostInputRef}
          />
        </section>

        {setupSection ? <LauncherSection section={setupSection} /> : null}

        <footer className="flex items-center gap-3 px-3 text-[10.5px] text-muted-foreground/50">
          <span className="flex items-center gap-1">
            <Kbd className="h-4 min-w-4 px-1 text-[10px]">↑</Kbd>
            <Kbd className="h-4 min-w-4 px-1 text-[10px]">↓</Kbd>
            move
          </span>
          <span className="flex items-center gap-1">
            <Kbd className="h-4 min-w-4 px-1 text-[10px]">↵</Kbd>
            open
          </span>
          <span className="flex items-center gap-1">
            <Kbd className="h-4 min-w-4 px-1 text-[10px]">Tab</Kbd>
            form fields
          </span>
        </footer>
      </div>
    </ScrollArea>
  );
}
