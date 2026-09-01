import {
  deleteSpaceData,
  loadAll,
  type LoadedSpaces,
  saveSpacesList,
  saveState,
  type SpaceState,
} from "@/modules/spaces/lib/store";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  hydrateDocs,
  useDocsStore,
} from "@/modules/workspace-docs/store/docsStore";
import { mergeDocs } from "./mergeDocs";
import {
  isSyncableSpace,
  mergeWorkspace,
  type WorkspaceLocal,
} from "./mergeWorkspace";
import {
  getDeviceId,
  getLastGen,
  getLocalTombstones,
  setLastGen,
  setLocalTombstones,
} from "./meta";
import {
  fromWirePath,
  mapSpacePaths,
  mapStatePaths,
  toWirePath,
} from "./pathMap";
import { useSyncStore } from "./syncStore";
import { isValidSyncHost, peekGen, pullDomain, pushDomain } from "./transport";
import {
  type DocsEnvelope,
  type StateMeta,
  SYNC_WIRE_VERSION,
  type WorkspaceEnvelope,
} from "./types";

// Cadences (ADR-023). Every remote round-trip is a full ssh handshake (no
// ControlMaster), so polls are slow and gen-gated to one call when unchanged.
const DOCS_PULL_INTERVAL_MS = 5 * 60_000;
const DOCS_PULL_FOCUS_GAP_MS = 30_000;
const DOCS_PUSH_DEBOUNCE_MS = 15_000;
const WS_CHECK_INTERVAL_MS = 60_000;
const BOOT_PULL_BUDGET_MS = 8_000;
const BACKOFF_MS = [60_000, 300_000];

type SyncConfig = { host: string; pathRoot: string };

function config(): SyncConfig | null {
  const p = usePreferencesStore.getState();
  if (!p.syncEnabled) return null;
  const host = p.syncHost.trim();
  if (!isValidSyncHost(host)) return null;
  return { host, pathRoot: p.syncPathRoot.trim() };
}

let failCount = 0;
let nextAllowedAt = 0;

function noteFailure(message: string): void {
  failCount++;
  const idx = Math.min(failCount - 1, BACKOFF_MS.length - 1);
  nextAllowedAt = Date.now() + BACKOFF_MS[idx];
  useSyncStore.getState().setStatus("offline", message);
}

function noteSuccess(): void {
  failCount = 0;
  nextAllowedAt = 0;
  useSyncStore.getState().markSynced();
}

function backedOff(force: boolean): boolean {
  return !force && Date.now() < nextAllowedAt;
}

// ---------------------------------------------------------------- docs sync

function localDocs(): Pick<DocsEnvelope, "notes" | "boards" | "tasks"> {
  const s = useDocsStore.getState();
  return { notes: s.notes, boards: s.boards, tasks: s.tasks };
}

async function pushDocs(cfg: SyncConfig): Promise<void> {
  const deviceId = await getDeviceId();
  const envelope: DocsEnvelope = { v: SYNC_WIRE_VERSION, ...localDocs() };
  const gen = await pushDomain(cfg.host, "docs", envelope, deviceId);
  await setLastGen("docs", gen);
}

/** Pull-merge-and-maybe-push. Push always goes through here so a concurrent
 * writer's entries are merged in, never clobbered (ADR-023: merge-then-write). */
async function syncDocs(cfg: SyncConfig, force = false): Promise<void> {
  if (backedOff(force)) return;
  const sync = useSyncStore.getState();
  sync.setStatus("syncing");
  try {
    const lastGen = await getLastGen("docs");
    const pulled = await pullDomain<DocsEnvelope>(cfg.host, "docs", lastGen);
    if (pulled.status === "error") {
      noteFailure(pulled.message);
      return;
    }
    let pushNeeded = false;
    if (pulled.status === "absent") {
      pushNeeded = true;
    } else if (pulled.status === "ok") {
      const remote = {
        notes: pulled.envelope.notes ?? {},
        boards: pulled.envelope.boards ?? {},
        tasks: pulled.envelope.tasks ?? {},
      };
      const merged = mergeDocs(localDocs(), remote);
      adopting = true;
      try {
        useDocsStore.getState().adoptRemote(remote);
      } finally {
        adopting = false;
      }
      await setLastGen("docs", pulled.gen);
      pushNeeded = merged.pushNeeded;
    } else {
      // unchanged remote: push only when local edits happened since last push.
      pushNeeded = docsDirty;
    }
    // Clear before the push: an edit landing mid-push re-dirties and the next
    // interval pushes it; the push itself reads live state.
    const shouldPush = pushNeeded || (force && docsDirty);
    docsDirty = false;
    if (shouldPush) await pushDocs(cfg);
    noteSuccess();
  } catch (e) {
    noteFailure(e instanceof Error ? e.message : String(e));
  }
}

// ------------------------------------------------------------ workspace sync

function metaMapToRecord(m: Map<string, { at: number }>): StateMeta {
  const out: StateMeta = {};
  for (const [k, v] of m) out[k] = { at: v.at };
  return out;
}

function envelopeFromWire(
  env: WorkspaceEnvelope,
  pathRoot: string,
): WorkspaceEnvelope {
  const map = (p: string) => fromWirePath(p, pathRoot);
  const sshIds = new Set(
    (env.spaces ?? []).filter((s) => s.env?.kind === "ssh").map((s) => s.id),
  );
  const states: Record<string, SpaceState> = {};
  for (const [id, st] of Object.entries(env.states ?? {})) {
    states[id] = sshIds.has(id) ? st : mapStatePaths(st, map);
  }
  return {
    ...env,
    spaces: (env.spaces ?? []).map((s) => mapSpacePaths(s, map)),
    states,
  };
}

function envelopeToWire(
  local: WorkspaceLocal,
  pathRoot: string,
): WorkspaceEnvelope {
  const map = (p: string) => toWirePath(p, pathRoot);
  const spaces = local.spaces.filter(isSyncableSpace);
  const sshIds = new Set(
    spaces.filter((s) => s.env?.kind === "ssh").map((s) => s.id),
  );
  const keep = new Set(spaces.map((s) => s.id));
  const states: Record<string, SpaceState> = {};
  const stateMeta: StateMeta = {};
  for (const [id, st] of local.states) {
    if (!keep.has(id)) continue;
    states[id] = sshIds.has(id) ? st : mapStatePaths(st, map);
    const at = local.stateMeta[id]?.at;
    if (at !== undefined) stateMeta[id] = { at };
  }
  return {
    v: SYNC_WIRE_VERSION,
    spaces: spaces.map((s) => mapSpacePaths(s, map)),
    states,
    stateMeta,
    tombstones: local.tombstones,
  };
}

async function loadedToLocal(loaded: LoadedSpaces): Promise<WorkspaceLocal> {
  return {
    spaces: loaded.spaces,
    states: new Map(loaded.states),
    stateMeta: metaMapToRecord(loaded.stateMeta),
    tombstones: await getLocalTombstones(),
  };
}

/**
 * Boot-time layout adoption (ADR-023): merge the remote workspace envelope
 * into the just-loaded local triple BEFORE useSpaces.hydrate/replaceTabs see
 * it, persist what changed, and hand the merged result back to boot. Bounded:
 * past the budget the boot proceeds local-only and layout waits for the next
 * boot (docs still sync live).
 */
export async function bootPullWorkspace(
  loaded: LoadedSpaces,
): Promise<LoadedSpaces> {
  const cfg = config();
  if (!cfg) return loaded;
  try {
    const merged = await Promise.race([
      bootPullInner(loaded, cfg),
      new Promise<null>((resolve) =>
        setTimeout(() => resolve(null), BOOT_PULL_BUDGET_MS),
      ),
    ]);
    return merged ?? loaded;
  } catch (e) {
    console.warn("[koden] sync boot pull failed:", e);
    return loaded;
  }
}

async function bootPullInner(
  loaded: LoadedSpaces,
  cfg: SyncConfig,
): Promise<LoadedSpaces | null> {
  const lastGen = await getLastGen("ws");
  const pulled = await pullDomain<WorkspaceEnvelope>(cfg.host, "ws", lastGen);
  if (pulled.status === "error") {
    noteFailure(pulled.message);
    return null;
  }
  if (pulled.status === "absent" || pulled.status === "unchanged") {
    noteSuccess();
    wsPushSoon();
    return null;
  }
  const local = await loadedToLocal(loaded);
  const remote = envelopeFromWire(pulled.envelope, cfg.pathRoot);
  const merged = mergeWorkspace(local, remote);

  const spacesChanged =
    merged.changedSpaces.length > 0 || merged.removedSpaces.length > 0;
  if (spacesChanged) await saveSpacesList(merged.spaces);
  for (const id of merged.changedStates) {
    const st = merged.states.get(id);
    if (st) await saveState(id, st, merged.stateMeta[id]?.at ?? Date.now());
  }
  for (const id of merged.removedSpaces) await deleteSpaceData(id);
  await setLocalTombstones(merged.tombstones);
  await setLastGen("ws", pulled.gen);
  noteSuccess();
  if (merged.pushNeeded) wsPushSoon();

  const stateMeta = new Map(
    Object.entries(merged.stateMeta).map(([k, v]) => [k, { at: v.at }]),
  );
  const activeId =
    loaded.activeId && merged.removedSpaces.includes(loaded.activeId)
      ? null
      : loaded.activeId;
  return { spaces: merged.spaces, activeId, states: merged.states, stateMeta };
}

let lastPushedWsSig = "";

/** Push the local workspace envelope, merge-then-write. Remote layout changes
 * found here are folded into the pushed envelope but NOT applied to the
 * running UI — layout adoption is boot-only by design. */
async function syncWorkspace(cfg: SyncConfig, force = false): Promise<void> {
  if (backedOff(force)) return;
  try {
    const loaded = await loadAll();
    const local = await loadedToLocal(loaded);
    let envelope = envelopeToWire(local, cfg.pathRoot);
    const sig = JSON.stringify(envelope);
    if (!force && sig === lastPushedWsSig) return;

    const lastGen = await getLastGen("ws");
    const remoteGen = await peekGen(cfg.host, "ws");
    if (remoteGen === null) {
      noteFailure("sync host unreachable");
      return;
    }
    if (remoteGen !== 0 && remoteGen !== lastGen) {
      const pulled = await pullDomain<WorkspaceEnvelope>(
        cfg.host,
        "ws",
        lastGen,
      );
      if (pulled.status === "ok") {
        const remote = envelopeFromWire(pulled.envelope, cfg.pathRoot);
        const merged = mergeWorkspace(local, remote);
        envelope = envelopeToWire(
          {
            spaces: merged.spaces,
            states: merged.states,
            stateMeta: merged.stateMeta,
            tombstones: merged.tombstones,
          },
          cfg.pathRoot,
        );
        await setLocalTombstones(merged.tombstones);
      } else if (pulled.status === "error") {
        noteFailure(pulled.message);
        return;
      }
    }
    const deviceId = await getDeviceId();
    const gen = await pushDomain(cfg.host, "ws", envelope, deviceId);
    await setLastGen("ws", gen);
    lastPushedWsSig = sig;
    noteSuccess();
  } catch (e) {
    noteFailure(e instanceof Error ? e.message : String(e));
  }
}

// -------------------------------------------------------------- scheduling

let started = false;
let docsDirty = false;
// Suppresses the docs-store subscription while we apply remote entries, so
// our own adoption doesn't schedule a redundant push.
let adopting = false;
let stopFns: (() => void)[] = [];
let wsPushTimer: ReturnType<typeof setTimeout> | null = null;

function wsPushSoon(): void {
  if (wsPushTimer) return;
  wsPushTimer = setTimeout(() => {
    wsPushTimer = null;
    const cfg = config();
    if (cfg) void syncWorkspace(cfg);
  }, 15_000);
}

/** Manual "sync now" from the statusbar segment; ignores backoff. */
export function syncNow(): void {
  const cfg = config();
  if (!cfg) return;
  docsDirty = true;
  void syncDocs(cfg, true);
  void syncWorkspace(cfg, true);
}

/** Idempotent engine start; returns a stop function. Main window only. */
export function startSyncEngine(): () => void {
  if (started) return () => {};
  started = true;

  let lastDocsPull = 0;

  const applyStatus = () => {
    const cfg = config();
    if (!cfg) useSyncStore.getState().setStatus("disabled");
    else if (useSyncStore.getState().status === "disabled")
      useSyncStore.getState().setStatus("idle");
  };
  applyStatus();

  void hydrateDocs().then(() => {
    const cfg = config();
    if (cfg) {
      lastDocsPull = Date.now();
      void syncDocs(cfg);
    }
  });

  const unsubDocs = useDocsStore.subscribe(() => {
    if (!adopting) docsDirty = true;
  });
  const docsPushTimer = setInterval(() => {
    const cfg = config();
    if (cfg && docsDirty) void syncDocs(cfg);
  }, DOCS_PUSH_DEBOUNCE_MS);

  const docsPullTimer = setInterval(() => {
    const cfg = config();
    if (!cfg) return;
    lastDocsPull = Date.now();
    void syncDocs(cfg);
  }, DOCS_PULL_INTERVAL_MS);

  const wsTimer = setInterval(() => {
    const cfg = config();
    if (cfg) void syncWorkspace(cfg);
  }, WS_CHECK_INTERVAL_MS);

  const onFocus = () => {
    const cfg = config();
    if (!cfg) return;
    if (Date.now() - lastDocsPull < DOCS_PULL_FOCUS_GAP_MS) return;
    lastDocsPull = Date.now();
    void syncDocs(cfg);
  };
  const onHide = () => {
    if (document.visibilityState !== "hidden") return;
    const cfg = config();
    if (!cfg) return;
    if (docsDirty) void syncDocs(cfg);
    void syncWorkspace(cfg);
  };
  window.addEventListener("focus", onFocus);
  document.addEventListener("visibilitychange", onHide);

  const unsubPrefs = usePreferencesStore.subscribe(applyStatus);

  stopFns = [
    unsubDocs,
    unsubPrefs,
    () => clearInterval(docsPushTimer),
    () => clearInterval(docsPullTimer),
    () => clearInterval(wsTimer),
    () => window.removeEventListener("focus", onFocus),
    () => document.removeEventListener("visibilitychange", onHide),
    () => {
      if (wsPushTimer) clearTimeout(wsPushTimer);
      wsPushTimer = null;
    },
  ];
  return stopSyncEngine;
}

export function stopSyncEngine(): void {
  if (!started) return;
  started = false;
  for (const fn of stopFns) fn();
  stopFns = [];
}
