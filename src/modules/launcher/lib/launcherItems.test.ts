import type { SpaceMeta } from "@/modules/spaces/lib/store";
import type { IconSvgElement } from "@hugeicons/react";
import { describe, expect, it, vi } from "vitest";
import {
  buildLauncherSections,
  envBadgeLabel,
  filterHosts,
  folderBasename,
  hostHint,
  type LauncherIcons,
  normalizeFolderPath,
  RECENT_SPACES_CAP,
  recentSpaces,
  sameEnv,
  shortenRoot,
  validateHost,
} from "./launcherItems";

const icon: IconSvgElement = [];
const icons: LauncherIcons = {
  terminal: icon,
  wsl: icon,
  folder: icon,
  setup: icon,
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
    expect(envBadgeLabel({ kind: "wsl", distro: "Ubuntu" })).toBe("WSL: Ubuntu");
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
  it("shortens a root to its last two segments on either separator", () => {
    expect(shortenRoot("C:\\Users\\me\\Products\\koden")).toBe("Products/koden");
    expect(shortenRoot("/home/me")).toBe("home/me");
    expect(shortenRoot("/")).toBe("/");
    expect(shortenRoot(null)).toBeNull();
  });

  it("normalizes a picked folder to forward slashes without a trailing slash", () => {
    expect(normalizeFolderPath("C:\\Users\\me\\proj\\")).toBe("C:/Users/me/proj");
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

describe("buildLauncherSections", () => {
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
  ];
  const handlers = () => ({
    switchSpace: vi.fn(),
    newTerminal: vi.fn(),
    openFolder: vi.fn(),
    openSetup: vi.fn(),
  });
  const input = {
    spaces,
    activeSpaceId: "active",
    distros: [
      { name: "Ubuntu", default: true, running: true },
      { name: "Debian", default: false, running: false },
    ],
    isWindows: true,
    localLabel: "Windows",
    localCwd: "C:/Users/me",
    newTabShortcut: "Ctrl+T",
    icons,
  };

  it("orders the sections continue, new terminal, open, set up", () => {
    const sections = buildLauncherSections(input, handlers());
    expect(sections.map((s) => s.id)).toEqual([
      "continue",
      "new-terminal",
      "open-folder",
      "setup",
    ]);
  });

  it("lists recent spaces with root, env badge and accent, wired to switchSpace", () => {
    const on = handlers();
    const [cont] = buildLauncherSections(input, on);
    expect(cont.items.map((i) => i.label)).toEqual(["Homelab", "vps"]);
    expect(cont.items[0]).toMatchObject({
      description: "me/lab",
      badge: "WSL: Ubuntu",
    });
    expect(cont.items[0].accent).not.toBe("var(--primary)");
    expect(cont.items[1]).toMatchObject({
      description: null,
      badge: "ssh: vps",
      accent: "var(--primary)",
    });
    cont.items[1].onSelect();
    expect(on.switchSpace).toHaveBeenCalledWith("ssh");
  });

  it("offers a local terminal plus one item per WSL distro on Windows", () => {
    const on = handlers();
    const [, term] = buildLauncherSections(input, on);
    expect(term.items.map((i) => i.id)).toEqual([
      "terminal:local",
      "terminal:wsl:Ubuntu",
      "terminal:wsl:Debian",
    ]);
    expect(term.items[0]).toMatchObject({
      description: "C:/Users/me",
      hint: "Ctrl+T",
    });
    expect(term.items[1].badge).toBe("running");
    expect(term.items[2].badge).toBeNull();
    term.items[0].onSelect();
    term.items[2].onSelect();
    expect(on.newTerminal).toHaveBeenNthCalledWith(1, { kind: "local" });
    expect(on.newTerminal).toHaveBeenNthCalledWith(2, {
      kind: "wsl",
      distro: "Debian",
    });
  });

  it("never lists WSL distros off Windows", () => {
    const [, term] = buildLauncherSections(
      { ...input, isWindows: false, localLabel: "macOS" },
      handlers(),
    );
    expect(term.items.map((i) => i.id)).toEqual(["terminal:local"]);
  });

  it("routes open folder and setup to their handlers", () => {
    const on = handlers();
    const [, , open, setup] = buildLauncherSections(input, on);
    open.items[0].onSelect();
    setup.items[0].onSelect();
    expect(on.openFolder).toHaveBeenCalledOnce();
    expect(on.openSetup).toHaveBeenCalledOnce();
  });

  it("keeps an empty-state line for Continue when there is nothing to continue", () => {
    const [cont] = buildLauncherSections(
      { ...input, spaces: [spaces[0]] },
      handlers(),
    );
    expect(cont.items).toEqual([]);
    expect(cont.empty).toBeTruthy();
  });
});
