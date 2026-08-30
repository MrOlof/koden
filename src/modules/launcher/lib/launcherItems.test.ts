import type { SpaceMeta } from "@/modules/spaces/lib/store";
import type { IconSvgElement } from "@hugeicons/react";
import { describe, expect, it, vi } from "vitest";
import {
  buildStartPage,
  envBadgeLabel,
  filterHosts,
  folderBasename,
  hostHint,
  normalizeFolderPath,
  RECENT_EMPTY,
  RECENT_SPACES_CAP,
  recentSpaces,
  sameEnv,
  shortenRoot,
  START_ITEM_IDS,
  type StartIcons,
  validateHost,
} from "./launcherItems";

const icon = (name: string): IconSvgElement =>
  [["path", { d: name }]] as unknown as IconSvgElement;
const icons: StartIcons = {
  openFolder: icon("openFolder"),
  remote: icon("remote"),
  terminal: icon("terminal"),
  wsl: icon("wsl"),
  editor: icon("editor"),
  note: icon("note"),
  folder: icon("folder"),
  server: icon("server"),
};

function space(over: Partial<SpaceMeta> & { id: string }): SpaceMeta {
  return {
    name: over.id,
    root: `/home/me/${over.id}`,
    env: { kind: "local" },
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("envBadgeLabel", () => {
  it("labels each env kind and hides local unless a label is given", () => {
    expect(envBadgeLabel({ kind: "local" })).toBeNull();
    expect(envBadgeLabel({ kind: "local" }, "Windows")).toBe("Windows");
    expect(envBadgeLabel({ kind: "wsl", distro: "Ubuntu" })).toBe(
      "WSL: Ubuntu",
    );
    expect(envBadgeLabel({ kind: "ssh", host: "lab", path: "" })).toBe(
      "ssh: lab",
    );
  });

  it("tolerates a missing env on older persisted spaces", () => {
    expect(envBadgeLabel(undefined, "Windows")).toBeNull();
  });
});

describe("sameEnv", () => {
  it("compares kind, distro and host but not the ssh path", () => {
    expect(sameEnv({ kind: "local" }, { kind: "local" })).toBe(true);
    expect(
      sameEnv({ kind: "wsl", distro: "a" }, { kind: "wsl", distro: "b" }),
    ).toBe(false);
    expect(
      sameEnv(
        { kind: "ssh", host: "h", path: "/a" },
        { kind: "ssh", host: "h", path: "/b" },
      ),
    ).toBe(true);
    expect(sameEnv({ kind: "local" }, { kind: "wsl", distro: "a" })).toBe(
      false,
    );
  });
});

describe("paths", () => {
  it("keeps the last two segments of a deep root on either separator", () => {
    expect(shortenRoot("C:\\Users\\me\\Products\\koden")).toBe(
      "…/Products/koden",
    );
    expect(shortenRoot("/srv/app/current/site")).toBe("…/current/site");
    expect(shortenRoot(null)).toBeNull();
  });

  it("leaves a short root whole in canonical form", () => {
    expect(shortenRoot("/home/me")).toBe("/home/me");
    expect(shortenRoot("C:\\Users\\")).toBe("C:/Users");
    expect(shortenRoot("/")).toBe("/");
  });

  it("folds the home folder to ~ and keeps at most two segments after it", () => {
    const home = "C:/Users/me";
    expect(shortenRoot("C:\\Users\\me", home)).toBe("~");
    expect(shortenRoot("C:\\Users\\me\\Products", home)).toBe("~/Products");
    expect(shortenRoot("C:\\Users\\me\\Products\\koden", home)).toBe(
      "~/Products/koden",
    );
    expect(shortenRoot("C:/Users/me/a/b/c", home)).toBe("~/…/b/c");
    expect(shortenRoot("/home/me/src/koden/app", "/home/me")).toBe(
      "~/…/koden/app",
    );
  });

  it("matches home case-insensitively only on Windows drives", () => {
    expect(shortenRoot("c:/users/me/x", "C:/Users/me")).toBe("~/x");
    expect(shortenRoot("/home/ME/x", "/home/me")).toBe("…/ME/x");
    expect(shortenRoot("/home/melon/x", "/home/me")).toBe("…/melon/x");
  });

  it("normalizes a picked folder to forward slashes without a trailing slash", () => {
    expect(normalizeFolderPath("C:\\Users\\me\\proj\\")).toBe(
      "C:/Users/me/proj",
    );
    expect(normalizeFolderPath("/home/me/proj/")).toBe("/home/me/proj");
    expect(normalizeFolderPath("C:\\")).toBe("C:/");
    expect(normalizeFolderPath("/")).toBe("/");
  });

  it("derives the Space name from the folder basename", () => {
    expect(folderBasename("C:/Users/me/proj")).toBe("proj");
    expect(folderBasename("/home/me/proj")).toBe("proj");
  });
});

describe("recentSpaces", () => {
  it("excludes the active space and sorts by updatedAt desc", () => {
    const spaces = [
      space({ id: "a", updatedAt: 10 }),
      space({ id: "active", updatedAt: 99 }),
      space({ id: "b", updatedAt: 30 }),
      space({ id: "c", updatedAt: 20 }),
    ];
    expect(recentSpaces(spaces, "active").map((s) => s.id)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("caps the list and leaves the input untouched", () => {
    const spaces = Array.from({ length: 12 }, (_, i) =>
      space({ id: `s${i}`, updatedAt: i }),
    );
    const before = spaces.map((s) => s.id);
    const out = recentSpaces(spaces, null);
    expect(out).toHaveLength(RECENT_SPACES_CAP);
    expect(out[0].id).toBe("s11");
    expect(spaces.map((s) => s.id)).toEqual(before);
  });
});

describe("hosts", () => {
  const hosts = [
    { alias: "lab", hostName: "192.168.1.207", user: "kosta" },
    { alias: "haden", hostName: "whitecastle", user: "admin", port: 2222 },
    { alias: "vps", hostName: "vps" },
  ];

  it("filters by alias, host name or user, case-insensitive", () => {
    expect(filterHosts(hosts, "").map((h) => h.alias)).toEqual([
      "lab",
      "haden",
      "vps",
    ]);
    expect(filterHosts(hosts, "WHITE").map((h) => h.alias)).toEqual(["haden"]);
    expect(filterHosts(hosts, "kos").map((h) => h.alias)).toEqual(["lab"]);
    expect(filterHosts(hosts, "zzz")).toEqual([]);
  });

  it("caps suggestions", () => {
    const many = Array.from({ length: 20 }, (_, i) => ({ alias: `h${i}` }));
    expect(filterHosts(many, "", 3)).toHaveLength(3);
  });

  it("builds a compact hint and hides it when it adds nothing", () => {
    expect(hostHint(hosts[0])).toBe("kosta@192.168.1.207");
    expect(hostHint(hosts[1])).toBe("admin@whitecastle:2222");
    expect(hostHint(hosts[2])).toBeNull();
  });

  it("rejects hosts that could be read as ssh flags or extra arguments", () => {
    expect(validateHost("")).not.toBeNull();
    expect(validateHost("-oProxyCommand=x")).not.toBeNull();
    expect(validateHost("host extra")).not.toBeNull();
    expect(validateHost("a".repeat(300))).not.toBeNull();
    expect(validateHost(" kosta@lab ")).toBeNull();
  });
});

describe("buildStartPage", () => {
  const spaces = [
    space({ id: "active", name: "Active", updatedAt: 5 }),
    space({
      id: "wsl",
      name: "Homelab",
      root: "/home/me/lab",
      env: { kind: "wsl", distro: "Ubuntu" },
      updatedAt: 9,
      color: 2,
    }),
    space({
      id: "ssh",
      name: "vps",
      root: null,
      env: { kind: "ssh", host: "vps", path: "" },
      updatedAt: 7,
    }),
    space({
      id: "local",
      name: "koden",
      root: "C:/Users/me/Products/koden",
      updatedAt: 8,
    }),
  ];
  const handlers = () => ({
    switchSpace: vi.fn(),
    newTerminal: vi.fn(),
    openFolder: vi.fn(),
    connectRemote: vi.fn(),
    newEditor: vi.fn(),
    newNote: vi.fn(),
  });
  const input = {
    spaces,
    activeSpaceId: "active",
    distros: [
      { name: "Ubuntu", default: true, running: true },
      { name: "Debian", default: false, running: false },
    ],
    isWindows: true,
    home: "C:/Users/me",
    newTerminalShortcut: "Ctrl T",
    newEditorShortcut: "Ctrl E",
    icons,
  };

  it("orders START: open folder, remote, terminal, WSL distros, editor, note", () => {
    const { start } = buildStartPage(input, handlers());
    expect(start.items.map((i) => i.id)).toEqual([
      START_ITEM_IDS.openFolder,
      START_ITEM_IDS.connectRemote,
      START_ITEM_IDS.newTerminal,
      "terminal:wsl:Ubuntu",
      "terminal:wsl:Debian",
      START_ITEM_IDS.newEditor,
      START_ITEM_IDS.newNote,
    ]);
    expect(start.items.map((i) => i.label)).toEqual([
      "Open folder…",
      "Connect to remote…",
      "New terminal",
      "Terminal in Ubuntu",
      "Terminal in Debian",
      "New editor",
      "New note",
    ]);
  });

  it("shows the live bindings as keycap hints, never a fixed key", () => {
    const { start } = buildStartPage(input, handlers());
    const byId = new Map(start.items.map((i) => [i.id, i]));
    expect(byId.get(START_ITEM_IDS.newTerminal)?.shortcut).toBe("Ctrl T");
    expect(byId.get(START_ITEM_IDS.newEditor)?.shortcut).toBe("Ctrl E");
    expect(byId.get(START_ITEM_IDS.openFolder)?.shortcut).toBeUndefined();
    const rebound = buildStartPage(
      { ...input, newTerminalShortcut: "Ctrl Shift T" },
      handlers(),
    );
    expect(rebound.start.items[2].shortcut).toBe("Ctrl Shift T");
  });

  it("marks WSL rows with a badge and routes them to the distro env", () => {
    const on = handlers();
    const { start } = buildStartPage(input, on);
    const wslRows = start.items.filter((i) => i.id.startsWith("terminal:wsl:"));
    expect(wslRows.every((i) => i.badge === "WSL")).toBe(true);
    expect(wslRows.every((i) => i.icon === icons.wsl)).toBe(true);
    start.items[2].onSelect();
    wslRows[1].onSelect();
    expect(on.newTerminal).toHaveBeenNthCalledWith(1, { kind: "local" });
    expect(on.newTerminal).toHaveBeenNthCalledWith(2, {
      kind: "wsl",
      distro: "Debian",
    });
  });

  it("never lists WSL distros off Windows", () => {
    const { start } = buildStartPage(
      { ...input, isWindows: false },
      handlers(),
    );
    expect(start.items.some((i) => i.id.startsWith("terminal:wsl:"))).toBe(
      false,
    );
  });

  it("omits the editor and note rows when the shell offers no handler", () => {
    const on = handlers();
    const { start } = buildStartPage(input, {
      ...on,
      newEditor: undefined,
      newNote: undefined,
    });
    expect(start.items.map((i) => i.id)).not.toContain(
      START_ITEM_IDS.newEditor,
    );
    expect(start.items.map((i) => i.id)).not.toContain(START_ITEM_IDS.newNote);
  });

  it("routes open folder, remote, editor and note to their handlers", () => {
    const on = handlers();
    const { start } = buildStartPage(input, on);
    const byId = new Map(start.items.map((i) => [i.id, i]));
    byId.get(START_ITEM_IDS.openFolder)?.onSelect();
    byId.get(START_ITEM_IDS.connectRemote)?.onSelect();
    byId.get(START_ITEM_IDS.newEditor)?.onSelect();
    byId.get(START_ITEM_IDS.newNote)?.onSelect();
    expect(on.openFolder).toHaveBeenCalledOnce();
    expect(on.connectRemote).toHaveBeenCalledOnce();
    expect(on.newEditor).toHaveBeenCalledOnce();
    expect(on.newNote).toHaveBeenCalledOnce();
  });

  it("lists recent spaces newest first with env icon, badge and short path", () => {
    const on = handlers();
    const { recent } = buildStartPage(input, on);
    expect(recent.items.map((i) => i.label)).toEqual([
      "Homelab",
      "koden",
      "vps",
    ]);
    expect(recent.items[0]).toMatchObject({
      description: "…/me/lab",
      badge: "WSL: Ubuntu",
      icon: icons.wsl,
    });
    expect(recent.items[1]).toMatchObject({
      description: "~/Products/koden",
      badge: null,
      icon: icons.folder,
    });
    expect(recent.items[2]).toMatchObject({
      description: null,
      badge: "ssh: vps",
      icon: icons.server,
    });
    recent.items[2].onSelect();
    expect(on.switchSpace).toHaveBeenCalledWith("ssh");
  });

  it("caps recent at the shared limit", () => {
    const many = Array.from({ length: 12 }, (_, i) =>
      space({ id: `s${i}`, updatedAt: i }),
    );
    const { recent } = buildStartPage({ ...input, spaces: many }, handlers());
    expect(recent.items).toHaveLength(RECENT_SPACES_CAP);
  });

  it("keeps an empty-state line for Recent when there is nothing to reopen", () => {
    const { recent } = buildStartPage(
      { ...input, spaces: [spaces[0]] },
      handlers(),
    );
    expect(recent.items).toEqual([]);
    expect(recent.empty).toBe(RECENT_EMPTY);
  });
});
