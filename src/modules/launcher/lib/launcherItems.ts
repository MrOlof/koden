import { accentFor } from "@/modules/spaces/lib/spaceColor";
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
  /** Muted second line (e.g. a shortened path). */
  description?: string | null;
  /** Right-aligned muted text (e.g. a shortcut or "2 panes"). */
  hint?: string | null;
  /** Small pill after the label (e.g. "WSL: Ubuntu"). */
  badge?: string | null;
  /** CSS color for the leading dot; omitted = no dot. */
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

/** Last two path segments, the same shortening the space switcher uses. */
export function shortenRoot(root: string | null | undefined): string | null {
  if (!root) return null;
  const segs = root.split(/[\\/]/).filter(Boolean);
  return segs.slice(-2).join("/") || root;
}

/** Canonical forward-slash form without a trailing separator ("C:/" keeps its). */
export function normalizeFolderPath(path: string): string {
  const fwd = path.trim().replace(/\\/g, "/");
  const stripped = fwd.replace(/\/+$/, "");
  return /^[A-Za-z]:$/.test(stripped) || stripped === "" ? `${stripped}/` : stripped;
}

export function folderBasename(path: string): string {
  const segs = path.split(/[\\/]/).filter(Boolean);
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

export type LauncherIcons = {
  terminal: IconSvgElement;
  wsl: IconSvgElement;
  folder: IconSvgElement;
  setup: IconSvgElement;
};

export type LauncherBuildInput = {
  spaces: readonly SpaceMeta[];
  activeSpaceId: string | null;
  distros: readonly WslDistro[];
  isWindows: boolean;
  /** Badge for local Spaces ("Windows", "macOS", "Linux"). */
  localLabel: string;
  /** Where "Terminal here" opens; null hides the description. */
  localCwd: string | null;
  newTabShortcut?: string | null;
  icons: LauncherIcons;
};

export type LauncherHandlers = {
  switchSpace: (id: string) => void;
  newTerminal: (env: WorkspaceEnv) => void;
  openFolder: () => void;
  openSetup: () => void;
};

export const LAUNCHER_SECTION_IDS = {
  continue: "continue",
  newTerminal: "new-terminal",
  openFolder: "open-folder",
  setup: "setup",
} as const;

export function buildLauncherSections(
  input: LauncherBuildInput,
  on: LauncherHandlers,
): LauncherSectionModel[] {
  const recent = recentSpaces(input.spaces, input.activeSpaceId);
  const continueSection: LauncherSectionModel = {
    id: LAUNCHER_SECTION_IDS.continue,
    title: "Continue",
    empty: "No other Spaces yet. Open a folder to start one.",
    items: recent.map((s) => ({
      id: `space:${s.id}`,
      label: s.name,
      description: shortenRoot(s.root),
      badge: envBadgeLabel(s.env, input.localLabel),
      accent: accentFor(s),
      onSelect: () => on.switchSpace(s.id),
    })),
  };

  const terminalItems: LauncherItemModel[] = [
    {
      id: "terminal:local",
      label: "Terminal here",
      description: input.localCwd,
      hint: input.newTabShortcut ?? null,
      icon: input.icons.terminal,
      onSelect: () => on.newTerminal({ kind: "local" }),
    },
  ];
  if (input.isWindows) {
    for (const d of input.distros) {
      terminalItems.push({
        id: `terminal:wsl:${d.name}`,
        label: d.name,
        description: d.default ? "Default WSL distro" : "WSL distro",
        badge: d.running ? "running" : null,
        icon: input.icons.wsl,
        onSelect: () => on.newTerminal({ kind: "wsl", distro: d.name }),
      });
    }
  }

  return [
    continueSection,
    {
      id: LAUNCHER_SECTION_IDS.newTerminal,
      title: "New terminal",
      items: terminalItems,
    },
    {
      id: LAUNCHER_SECTION_IDS.openFolder,
      title: "Open",
      items: [
        {
          id: "open-folder",
          label: "Open folder as a new Space",
          description: "Pick a folder; it becomes a Space with a terminal in it.",
          icon: input.icons.folder,
          onSelect: on.openFolder,
        },
      ],
    },
    {
      id: LAUNCHER_SECTION_IDS.setup,
      title: "Set up",
      items: [
        {
          id: "setup-guide",
          label: "Run the setup guide",
          description: "Connect a model, choose a projects folder, enable the Librarian.",
          icon: input.icons.setup,
          onSelect: on.openSetup,
        },
      ],
    },
  ];
}
