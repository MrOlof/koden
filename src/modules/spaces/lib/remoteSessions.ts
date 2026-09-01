// Live remote sessions of an ssh+tmux Space: tmux itself is the source of
// truth for what exists on the host (M2.5 F2, lean version — no host-side
// manifest; window name carries identity, pane command/path carry display).
// ponytail: custom titles and tab ORDER don't survive adoption — that's the
// deferred json manifest's job (KODEN-REMOTE.md M2.5 Feature 2).

import { invoke } from "@tauri-apps/api/core";

export type RemoteWindow = {
  name: string;
  command: string;
  path: string;
  /** tmux pane id (`%N`) of the window's single pane; "" when the host sent
   * something malformed. The pane-events join column (M2.8). */
  pane: string;
};

const WINDOW_PREFIX = "w-";
const NAME_CAP = 48;

/** Mirror of Rust `tmux_window_name` (shell_ssh.rs): the window name a leaf
 * restore key maps to. Keys are `[A-Za-z0-9_-]` already, so this is prefix +
 * dash-collapse + cap + trailing-dash trim. Parity is covered by tests on
 * both sides with shared vectors. */
export function windowNameForKey(key: string): string {
  let out = WINDOW_PREFIX;
  let lastDash = true;
  for (const c of key) {
    const mapped = /[A-Za-z0-9_]/.test(c) ? c : "-";
    if (mapped === "-") {
      if (lastDash) continue;
      lastDash = true;
    } else {
      lastDash = false;
    }
    out += mapped;
    if (out.length >= NAME_CAP) break;
  }
  const trimmed = out.replace(/-+$/, "");
  return trimmed === "w" ? "w-pane" : trimmed;
}

/** The restore key a `w-…` window name encodes, or null for foreign windows
 * (user-created ones in the same session are left alone). */
export function keyFromWindowName(name: string): string | null {
  if (!name.startsWith(WINDOW_PREFIX)) return null;
  const key = name.slice(WINDOW_PREFIX.length);
  return /^[A-Za-z0-9_-]{1,64}$/.test(key) ? key : null;
}

export type AdoptablePane = {
  /** Restore key to seed on the new leaf so its spawn attaches this window. */
  key: string;
  /** Tab title: the window's foreground command ("claude", "htop", …). */
  title: string;
  /** The window's current path on the host; the tab's cwd. */
  cwd?: string;
};

/** Titles by restore key from the host-side manifest json ("manifest is
 * truth for titles"). Tolerates absent/garbled input. */
export function parseManifestTitles(json: string): Map<string, string> {
  const out = new Map<string, string>();
  try {
    const m = JSON.parse(json) as { tabs?: { key?: unknown; title?: unknown }[] };
    for (const t of m.tabs ?? []) {
      if (typeof t.key === "string" && typeof t.title === "string" && t.title) {
        out.set(t.key, t.title);
      }
    }
  } catch {
    // no manifest yet — command names carry the day
  }
  return out;
}

/** Windows live on the host that no local pane owns — each becomes a tab on
 * connect, named from the manifest when it knows the window. A shell
 * sitting at the prompt is still adopted: it may hold scrollback, and
 * dropping it would betray "never miss a session". */
export function planAdoption(
  windows: readonly RemoteWindow[],
  localKeys: ReadonlySet<string>,
  titles?: ReadonlyMap<string, string>,
): AdoptablePane[] {
  const out: AdoptablePane[] = [];
  for (const w of windows) {
    const key = keyFromWindowName(w.name);
    if (!key || localKeys.has(key)) continue;
    out.push({
      key,
      title: titles?.get(key) || w.command.trim() || "session",
      ...(w.path.startsWith("/") || w.path.startsWith("~")
        ? { cwd: w.path }
        : {}),
    });
  }
  return out;
}

/** Launcher hint text for a Space's live-session count; null hides the hint
 * (unknown or zero — an empty host isn't worth a badge). */
export function livenessHint(count: number | null | undefined): string | null {
  if (count == null || count <= 0) return null;
  return count === 1 ? "● 1 live session" : `● ${count} live sessions`;
}

// Short cache so launcher re-renders don't hammer ssh; one probe per Space
// per TTL. Failures cache as null (unknown) so a dead host is probed at most
// once per TTL too.
const LIVENESS_TTL_MS = 30_000;
const livenessCache = new Map<
  string,
  { at: number; value: Promise<number | null> }
>();

export function remoteSessionCount(
  host: string,
  tmuxKey: string,
  ttlMs: number = LIVENESS_TTL_MS,
): Promise<number | null> {
  const cacheKey = `${host}\0${tmuxKey}`;
  const hit = livenessCache.get(cacheKey);
  if (hit && Date.now() - hit.at < ttlMs) return hit.value;
  const value = invoke<RemoteWindow[]>("ssh_tmux_windows", {
    host,
    spaceKey: tmuxKey,
  })
    .then((ws) => ws.filter((w) => keyFromWindowName(w.name) !== null).length)
    .catch(() => null);
  livenessCache.set(cacheKey, { at: Date.now(), value });
  return value;
}
