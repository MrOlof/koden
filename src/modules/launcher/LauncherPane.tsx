import { ScrollArea } from "@/components/ui/scroll-area";
import { Wordmark } from "@/components/Wordmark";
import { IS_WINDOWS } from "@/lib/platform";
import { type ShortcutId, useShortcutLabel } from "@/modules/shortcuts";
import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import { useWorkspaceEnvStore, type WorkspaceEnv } from "@/modules/workspace";
import {
  CloudServerIcon,
  ComputerTerminal02Icon,
  Folder01Icon,
  FolderOpenIcon,
  Note01Icon,
  PencilEdit02Icon,
  ServerStack03Icon,
  TerminalIcon,
} from "@hugeicons/core-free-icons";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { KeyTokens } from "./KeyTokens";
import { LauncherSection, LauncherSectionTitle } from "./LauncherSection";
import {
  buildStartPage,
  type LauncherSectionModel,
  type SshEnv,
  type SshHost,
  START_ITEM_IDS,
  type StartIcons,
} from "./lib/launcherItems";
import { useLauncherKeys } from "./lib/useLauncherKeys";
import { RemoteConnectForm } from "./RemoteConnectForm";

export type LauncherFocusTarget = "remote";

type Props = {
  /** Land focus somewhere specific on mount (the palette's connect command). */
  initialFocus?: LauncherFocusTarget | null;
  onFocusHandled?: () => void;
  onSwitchSpace: (spaceId: string) => void;
  onNewTerminal: (env: WorkspaceEnv) => void;
  onOpenFolder: () => void;
  onConnectRemote: (env: SshEnv) => Promise<void> | void;
  onOpenSetup: () => void;
  onNewEditor?: () => void;
  onNewNote?: () => void;
  /** Local home; recent paths under it read as `~/…`. */
  home?: string | null;
  /** Extra list sections (resume cards), rendered above the two columns. */
  extraSections?: LauncherSectionModel[];
};

const ICONS: StartIcons = {
  openFolder: FolderOpenIcon,
  remote: CloudServerIcon,
  terminal: ComputerTerminal02Icon,
  wsl: TerminalIcon,
  editor: PencilEdit02Icon,
  note: Note01Icon,
  folder: Folder01Icon,
  server: ServerStack03Icon,
};

const SHORTCUT_ROWS: { id: ShortcutId; label: string }[] = [
  { id: "tab.new", label: "New terminal" },
  { id: "tab.newEditor", label: "New editor" },
  { id: "commandPalette.open", label: "Command palette" },
  { id: "sidebar.toggle", label: "Toggle sidebar" },
  { id: "launcher.show", label: "Start page" },
  { id: "ai.toggle", label: "Ask the Librarian" },
];

const CONNECT_ROW_SELECTOR = `[data-launcher-item="${START_ITEM_IDS.connectRemote}"]`;
const FIRST_START_ROW_SELECTOR = `[data-launcher-item="${START_ITEM_IDS.openFolder}"]`;

/**
 * The start page: brand, resume cards, START and RECENT columns, the live
 * shortcut sheet and the version. Lives in a `launcher` tab (never persisted)
 * and is also the content of any Space with no tabs.
 */
export function LauncherPane({
  initialFocus = null,
  onFocusHandled,
  onSwitchSpace,
  onNewTerminal,
  onOpenFolder,
  onConnectRemote,
  onOpenSetup,
  onNewEditor,
  onNewNote,
  home = null,
  extraSections,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const hostInputRef = useRef<HTMLInputElement>(null);
  const spaces = useSpaces((s) => s.spaces);
  const activeSpaceId = useSpaces((s) => s.activeId);
  const distros = useWorkspaceEnvStore((s) => s.distros);
  const refreshDistros = useWorkspaceEnvStore((s) => s.refreshDistros);
  const newTerminalShortcut = useShortcutLabel("tab.new");
  const newEditorShortcut = useShortcutLabel("tab.newEditor");
  const [hosts, setHosts] = useState<SshHost[] | null>(null);
  const [remoteOpen, setRemoteOpen] = useState(initialFocus === "remote");
  const [version, setVersion] = useState("");

  useLauncherKeys(rootRef, {
    focusFirst: initialFocus === null,
    initialStop: FIRST_START_ROW_SELECTOR,
  });

  useEffect(() => {
    if (initialFocus !== "remote") return;
    setRemoteOpen(true);
    const raf = requestAnimationFrame(() => {
      hostInputRef.current?.focus();
      onFocusHandled?.();
    });
    return () => cancelAnimationFrame(raf);
  }, [initialFocus, onFocusHandled]);

  useEffect(() => {
    if (!remoteOpen) return;
    const raf = requestAnimationFrame(() => hostInputRef.current?.focus());
    return () => cancelAnimationFrame(raf);
  }, [remoteOpen]);

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

  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => {
        if (alive) setVersion(v);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const toggleRemote = useCallback(() => setRemoteOpen((v) => !v), []);

  const closeRemote = useCallback(() => {
    setRemoteOpen(false);
    rootRef.current?.querySelector<HTMLElement>(CONNECT_ROW_SELECTOR)?.focus();
  }, []);

  const page = useMemo(
    () =>
      buildStartPage(
        {
          spaces,
          activeSpaceId,
          distros,
          isWindows: IS_WINDOWS,
          home,
          newTerminalShortcut,
          newEditorShortcut,
          icons: ICONS,
        },
        {
          switchSpace: onSwitchSpace,
          newTerminal: onNewTerminal,
          openFolder: onOpenFolder,
          connectRemote: toggleRemote,
          newEditor: onNewEditor,
          newNote: onNewNote,
        },
      ),
    [
      spaces,
      activeSpaceId,
      distros,
      home,
      newTerminalShortcut,
      newEditorShortcut,
      onSwitchSpace,
      onNewTerminal,
      onOpenFolder,
      toggleRemote,
      onNewEditor,
      onNewNote,
    ],
  );

  return (
    // Size containment gives the page a container to center against
    // (100cqh) and to query for the two-column breakpoint; the Radix viewport
    // wraps content in a table box, so a percentage min-height would not.
    <ScrollArea className="h-full [container-type:size]">
      <div className="flex min-h-[100cqh] flex-col justify-center px-6 py-10">
        <div
          ref={rootRef}
          className="koden-panel-in mx-auto flex w-full max-w-[720px] flex-col gap-10"
        >
          <header className="flex flex-col items-center gap-3 text-center">
            <img
              src="/logo.png"
              alt=""
              className="size-10 select-none"
              draggable={false}
            />
            <Wordmark className="text-[26px] leading-none" />
            <p className="text-[11.5px] text-muted-foreground/70">
              A terminal-first AI workspace.
            </p>
          </header>

          {extraSections?.map((s) => (
            <LauncherSection key={s.id} section={s} />
          ))}

          <div className="grid grid-cols-1 gap-x-12 gap-y-8 @min-[560px]:grid-cols-2">
            <div className="flex min-w-0 flex-col gap-3">
              <LauncherSection section={page.start} />
              {remoteOpen ? (
                <RemoteConnectForm
                  hosts={hosts}
                  onConnect={onConnectRemote}
                  hostInputRef={hostInputRef}
                  onCancel={closeRemote}
                />
              ) : null}
            </div>
            <LauncherSection section={page.recent} className="min-w-0" />
          </div>

          <section
            aria-label="Keyboard shortcuts"
            className="flex flex-col gap-1.5"
          >
            <LauncherSectionTitle>Keyboard shortcuts</LauncherSectionTitle>
            <ul className="grid grid-cols-1 gap-x-12 @min-[560px]:grid-cols-2">
              {SHORTCUT_ROWS.map((row) => (
                <ShortcutRow key={row.id} id={row.id} label={row.label} />
              ))}
            </ul>
          </section>

          <footer className="flex items-center justify-center gap-2 font-mono text-[10.5px] text-muted-foreground/40">
            <span>{version ? `koden v${version}` : "koden"}</span>
            <span aria-hidden>·</span>
            <button
              type="button"
              onClick={onOpenSetup}
              className="rounded-sm outline-none transition-colors hover:text-muted-foreground focus-visible:text-muted-foreground focus-visible:ring-1 focus-visible:ring-primary/40"
            >
              Setup guide
            </button>
          </footer>
        </div>
      </div>
    </ScrollArea>
  );
}

function ShortcutRow({ id, label }: { id: ShortcutId; label: string }) {
  const binding = useShortcutLabel(id);
  return (
    <li className="flex h-7 items-center justify-between gap-3 px-2.5">
      <span className="truncate text-[12px] text-muted-foreground">
        {label}
      </span>
      <KeyTokens label={binding} />
    </li>
  );
}
