import { livenessHint } from "@/modules/spaces/lib/remoteSessions";
import type { SpaceMeta } from "@/modules/spaces/lib/store";
import type { WorkspaceEnv, WslDistro } from "@/modules/workspace";
import type { IconSvgElement } from "@hugeicons/react";

export type SshHost = {
  alias: string;
  hostName?: string;
  user?: string;
  port?: number;
};

export type SshEnv = Extract<WorkspaceEnv, { kind: "ssh" }>;

export type LauncherItemModel = {
  id: string;
  label: string;
  /** Muted mono text on the right (a shortened path, a cwd). */
  description?: string | null;
  /** Right-aligned muted text (e.g. "2 min ago" or "2 panes"). */
  hint?: string | null;
  /** Right-aligned key tokens ("Ctrl T"), rendered as keycaps. */
  shortcut?: string | null;
  /** Small pill after the label (e.g. "WSL", "ssh: lab"). */
  badge?: string | null;
  /** CSS color for a leading dot when the item has no icon. */
  accent?: string | null;
  /** hugeicons element; omitted = no icon. */
  icon?: IconSvgElement;
  onSelect: () => void;
};

export type LauncherSectionModel = {
  id: string;
  title: string;
  items: LauncherItemModel[];
  /** Shown in place of the list when items is empty; omit to hide the section. */
  empty?: string;
};

export const RECENT_SPACES_CAP = 8;
export const HOST_SUGGESTIONS_CAP = 8;

export function envBadgeLabel(
  env: WorkspaceEnv | null | undefined,
  localLabel: string | null = null,
): string | null {
  if (!env) return null;
  switch (env.kind) {
    case "local":
      return localLabel;
    case "wsl":
      return `WSL: ${env.distro}`;
    case "ssh":
      return `ssh: ${env.host}`;
  }
}

export function sameEnv(a: WorkspaceEnv, b: WorkspaceEnv): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "wsl" && b.kind === "wsl") return a.distro === b.distro;
  if (a.kind === "ssh" && b.kind === "ssh") return a.host === b.host;
  return true;
}

function segmentsOf(path: string): string[] {
  return path.split(/[\\/]/).filter(Boolean);
}

function isDriveLetter(seg: string | undefined): boolean {
  return seg !== undefined && /^[A-Za-z]:$/.test(seg);
}

function startsWithSegments(path: string[], prefix: string[]): boolean {
  if (prefix.length === 0 || prefix.length > path.length) return false;
  // Windows paths compare case-insensitively (C:/Users vs c:/users).
  const fold = isDriveLetter(prefix[0])
    ? (s: string) => s.toLowerCase()
    : (s: string) => s;
  return prefix.every((seg, i) => fold(seg) === fold(path[i]));
}

/**
 * `~/…/last/two`: a root under `home` folds home to `~`, a deep path keeps
 * its last two segments, and a short path stays whole.
 */
export function shortenRoot(
  root: string | null | undefined,
  home: string | null = null,
): string | null {
  if (!root) return null;
  const segs = segmentsOf(root);
  const homeSegs = home ? segmentsOf(home) : [];
  if (startsWithSegments(segs, homeSegs)) {
    const rel = segs.slice(homeSegs.length);
    if (rel.length === 0) return "~";
    if (rel.length <= 2) return `~/${rel.join("/")}`;
    return `~/…/${rel.slice(-2).join("/")}`;
  }
  if (segs.length <= 2) return normalizeFolderPath(root);
  return `…/${segs.slice(-2).join("/")}`;
}

/** Canonical forward-slash form without a trailing separator ("C:/" keeps its). */
export function normalizeFolderPath(path: string): string {
  const fwd = path.trim().replace(/\\/g, "/");
  const stripped = fwd.replace(/\/+$/, "");
  return /^[A-Za-z]:$/.test(stripped) || stripped === ""
    ? `${stripped}/`
    : stripped;
}

export function folderBasename(path: string): string {
  const segs = segmentsOf(path);
  return segs.length ? segs[segs.length - 1] : path;
}

export function recentSpaces(
  spaces: readonly SpaceMeta[],
  activeId: string | null,
  cap = RECENT_SPACES_CAP,
): SpaceMeta[] {
  return spaces
    .filter((s) => s.id !== activeId)
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, cap);
}

export function filterHosts(
  hosts: readonly SshHost[],
  query: string,
  cap = HOST_SUGGESTIONS_CAP,
): SshHost[] {
  const q = query.trim().toLowerCase();
  const match = (h: SshHost) =>
    !q ||
    h.alias.toLowerCase().includes(q) ||
    (h.hostName?.toLowerCase().includes(q) ?? false) ||
    (h.user?.toLowerCase().includes(q) ?? false);
  return hosts.filter(match).slice(0, cap);
}

export function hostHint(h: SshHost): string | null {
  const target = h.hostName && h.hostName !== h.alias ? h.hostName : "";
  const user = h.user ? `${h.user}@` : "";
  const port = h.port && h.port !== 22 ? `:${h.port}` : "";
  const s = `${user}${target}${port}`;
  return s || null;
}

// The host string ends up as an ssh argument: refuse anything that could be
// read as a flag or split into extra arguments before it leaves the UI.
export function validateHost(raw: string): string | null {
  const h = raw.trim();
  if (!h) return "Enter a host to connect to.";
  if (h.startsWith("-")) return "A host cannot start with a dash.";
  if (/\s/.test(h)) return "A host cannot contain spaces.";
  if (h.length > 255) return "That host name is too long.";
  return null;
}

export type StartIcons = {
  openFolder: IconSvgElement;
  remote: IconSvgElement;
  terminal: IconSvgElement;
  wsl: IconSvgElement;
  editor: IconSvgElement;
  note: IconSvgElement;
  /** Recent Space rows: local folder, ssh host. */
  folder: IconSvgElement;
  server: IconSvgElement;
};

export type StartPageInput = {
  spaces: readonly SpaceMeta[];
  activeSpaceId: string | null;
  distros: readonly WslDistro[];
  isWindows: boolean;
  /** Local home, folded to `~` in recent paths; null leaves paths as-is. */
  home: string | null;
  /** Live remote-session counts by space id (ssh+tmux Spaces only); absent
   * or null entries render no hint. Filled in asynchronously by the pane. */
  liveness?: Record<string, number | null>;
  newTerminalShortcut?: string | null;
  newEditorShortcut?: string | null;
  icons: StartIcons;
};

export type StartPageHandlers = {
  switchSpace: (id: string) => void;
  newTerminal: (env: WorkspaceEnv) => void;
  openFolder: () => void;
  /** Expands the inline remote form under the START column. */
  connectRemote: () => void;
  /** Rows are omitted when the shell does not provide these. */
  newEditor?: () => void;
  newNote?: () => void;
};

export type StartPageModel = {
  start: LauncherSectionModel;
  recent: LauncherSectionModel;
};

export const START_SECTION_IDS = {
  start: "start",
  recent: "recent",
} as const;

export const START_ITEM_IDS = {
  openFolder: "open-folder",
  connectRemote: "connect-remote",
  newTerminal: "terminal:local",
  newEditor: "new-editor",
  newNote: "new-note",
} as const;

export const RECENT_EMPTY = "No recent Spaces yet.";

function envIcon(
  env: WorkspaceEnv | undefined,
  icons: StartIcons,
): IconSvgElement {
  switch (env?.kind) {
    case "wsl":
      return icons.wsl;
    case "ssh":
      return icons.server;
    default:
      return icons.folder;
  }
}

export function buildStartPage(
  input: StartPageInput,
  on: StartPageHandlers,
): StartPageModel {
  const { icons } = input;
  const startItems: LauncherItemModel[] = [
    {
      id: START_ITEM_IDS.openFolder,
      label: "Open folder…",
      icon: icons.openFolder,
      onSelect: on.openFolder,
    },
    {
      id: START_ITEM_IDS.connectRemote,
      label: "Connect to remote…",
      icon: icons.remote,
      onSelect: on.connectRemote,
    },
    {
      id: START_ITEM_IDS.newTerminal,
      label: "New terminal",
      shortcut: input.newTerminalShortcut ?? null,
      icon: icons.terminal,
      onSelect: () => on.newTerminal({ kind: "local" }),
    },
  ];
  if (input.isWindows) {
    for (const d of input.distros) {
      startItems.push({
        id: `terminal:wsl:${d.name}`,
        label: `Terminal in ${d.name}`,
        badge: "WSL",
        icon: icons.wsl,
        onSelect: () => on.newTerminal({ kind: "wsl", distro: d.name }),
      });
    }
  }
  if (on.newEditor) {
    startItems.push({
      id: START_ITEM_IDS.newEditor,
      label: "New editor",
      shortcut: input.newEditorShortcut ?? null,
      icon: icons.editor,
      onSelect: on.newEditor,
    });
  }
  if (on.newNote) {
    startItems.push({
      id: START_ITEM_IDS.newNote,
      label: "New note",
      icon: icons.note,
      onSelect: on.newNote,
    });
  }

  const recent = recentSpaces(input.spaces, input.activeSpaceId);
  return {
    start: {
      id: START_SECTION_IDS.start,
      title: "Start",
      items: startItems,
    },
    recent: {
      id: START_SECTION_IDS.recent,
      title: "Recent",
      empty: RECENT_EMPTY,
      items: recent.map((s) => {
        const kind = s.env?.kind ?? "local";
        return {
          id: `space:${s.id}`,
          label: s.name,
          description: shortenRoot(
            s.root,
            kind === "local" ? input.home : null,
          ),
          // The headline feature made visible: a remote Space with live tmux
          // sessions shows them the moment the launcher opens.
          hint:
            kind === "ssh" ? livenessHint(input.liveness?.[s.id]) : null,
          badge: envBadgeLabel(s.env),
          icon: envIcon(s.env, icons),
          onSelect: () => on.switchSpace(s.id),
        };
      }),
    },
  };
}
