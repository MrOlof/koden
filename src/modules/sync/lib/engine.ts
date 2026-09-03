import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  deleteSpaceData,
  type LoadedSpaces,
  loadAll,
  type SpaceState,
  type SpaceStateMeta,
  saveSpacesList,
  saveState,
} from "@/modules/spaces/lib/store";
import { tabIdentities } from "@/modules/spaces/lib/tabClocks";
import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import {
  hydrateDocs,
  useDocsStore,
} from "@/modules/workspace-docs/store/docsStore";
import { toast } from "sonner";
import { expectClock, OBSERVED_CLOCK } from "./adoptionLedger";
import { appendJournal, type JournalEntry } from "./journal";
import {
  getLiveAdopters,
  planLiveDocAdoption,
  planLiveRenames,
} from "./liveAdopt";
import { mergeDocs } from "./mergeDocs";
import type { TabChange } from "./mergeState";
import {
  isSyncableSpace,
  mergeWorkspace,
  type WorkspaceLocal,
  type WorkspaceMergeResult,
} from "./mergeWorkspace";
import {
  getBootFailCount,
  getDeviceId,
  getLastGen,
  getLocalTombstones,
  getWsSig,
  setBootFailCount,
  setLastGen,
  setLocalTombstones,
  setWsSig,
} from "./meta";
import {
  fromWirePath,
  mapSpacePaths,
  mapStatePaths,
  toWirePath,
} from "./pathMap";
import { setWsChangedListener } from "./syncSignals";
import { useSyncStore } from "./syncStore";
import {
  isValidSyncHost,
  peekGen,
  pullDomain,
  pushDomain,
  TORN_MESSAGE,
} from "./transport";
import {
  type DocsEnvelope,
  type StateMeta,
  SYNC_WIRE_VERSION,
  type WorkspaceEnvelope,
} from "./types";

// Cadences (ADR-023). Every remote round-trip is a full ssh handshake (no
// ControlMaster), so polls are slow and gen-gated to one call when unchanged.
//
// Consistency model, revised after adversarial review:
// - docs "lastGen" means "fully merged into the live store".
// - ws "lastGen" is BOOT-ONLY: pushes never advance it, because layout
//   adoption happens only at boot. A mid-session pull folded into a push is
//   NOT locally adopted, so marking it consumed would both skip the next
//   boot's adoption and let a later skip-merge push erase it from the host.
// ADR-024 liveness: while the window is visible, both domains poll fast —
// an unchanged gen costs exactly one ssh handshake, so a 10 s cadence is
// cheap on a LAN/tailnet host. Hidden windows fall back to the slow timer.
const DOCS_PULL_INTERVAL_MS = 5 * 60_000;
const DOCS_PULL_FAST_MS = 10_000;
const DOCS_PULL_FOCUS_GAP_MS = 8_000;
const DOCS_PUSH_DEBOUNCE_MS = 2_500;
const WS_CHECK_INTERVAL_MS = 60_000;
const WS_LIVE_POLL_MS = 10_000;
const WS_PUSH_SOON_MS = 3_000;
const BOOT_PULL_BUDGET_MS = 8_000;
// After a failed boot pull, later boots wait far less: an unreachable host
// (asleep, VPN down) must not stall every launch for the full budget.
const BOOT_PULL_RETRY_BUDGET_MS = 2_000;
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
 * writer's entries are merged in, never clobbered (merge-then-write). Gated
 * on docs hydration: adopting into an empty pre-hydration store would let
 * older remote entries shadow newer local ones when hydration lands. */
async function syncDocs(cfg: SyncConfig, force = false): Promise<void> {
  if (backedOff(force)) return;
  await hydrateDocs();
  useSyncStore.getState().setStatus("syncing");
  try {
    const lastGen = await getLastGen("docs");
    const pulled = await pullDomain<DocsEnvelope>(cfg.host, "docs", lastGen);
    let pushNeeded = false;
    if (pulled.status === "error") {
      if (pulled.message !== TORN_MESSAGE) {
        noteFailure(pulled.message);
        return;
      }
      // Torn manifests (a writer crashed mid-push): a fresh push rewrites
      // parts and index consistently, healing the host for every peer.
      pushNeeded = true;
    } else if (pulled.status === "absent") {
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
    // Clear before the push (an edit landing mid-push re-dirties; the push
    // reads live state), but restore on failure so the edit isn't stranded
    // out of the pipeline until the next unrelated edit.
    const shouldPush = pushNeeded || (force && docsDirty);
    const wasDirty = docsDirty;
    docsDirty = false;
    if (shouldPush) {
      try {
        await pushDocs(cfg);
      } catch (e) {
        docsDirty = docsDirty || wasDirty || pushNeeded;
        throw e;
      }
    }
    noteSuccess();
  } catch (e) {
    noteFailure(e instanceof Error ? e.message : String(e));
  }
}

// ------------------------------------------------------------ workspace sync

function metaMapToRecord(m: Map<string, SpaceStateMeta>): StateMeta {
  const out: StateMeta = {};
  for (const [k, v] of m) out[k] = v;
  return out;
}

function tabLabel(t: TabChange["before"]): string {
  if (!t) return "";
  if (t.kind === "terminal") return t.customTitle ?? "";
  return "title" in t ? t.title : "";
}

/** Journal entries for what a merge did to tabs this device already had
 * (ADR-025): replaced layouts and peer-closed tabs. Additions change
 * nothing the user made. */
function journalOf(
  merged: WorkspaceMergeResult,
  via: JournalEntry["via"],
  now: number,
): JournalEntry[] {
  const out: JournalEntry[] = [];
  for (const [spaceId, changes] of Object.entries(merged.stateChanges)) {
    for (const c of changes) {
      if (c.kind === "added") continue;
      const isRename =
        c.kind === "replaced" && tabLabel(c.before) !== tabLabel(c.after);
      out.push({
        at: now,
        spaceId,
        tabId: c.id,
        field: c.kind === "removed" ? "closed" : isRename ? "title" : "layout",
        before: isRename
          ? tabLabel(c.before)
          : c.before
            ? JSON.stringify(c.before)
            : "",
        after: isRename
          ? tabLabel(c.after)
          : c.after
            ? JSON.stringify(c.after)
            : "",
        via,
      });
    }
  }
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
    const meta = local.stateMeta[id];
    if (meta !== undefined) stateMeta[id] = meta;
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
 * it, persist what changed, and hand the merged result back to boot.
 *
 * Bounded and CANCELLED past the budget: a late completion must not write
 * merged state computed from a pre-boot snapshot behind the back of a UI
 * that proceeded local-only (it could drop a just-created Default space or
 * mark the pull consumed without adopting it).
 */
export async function bootPullWorkspace(
  loaded: LoadedSpaces,
): Promise<LoadedSpaces> {
  const cfg = config();
  if (!cfg) return loaded;
  let cancelled = false;
  try {
    const budget =
      (await getBootFailCount()) > 0
        ? BOOT_PULL_RETRY_BUDGET_MS
        : BOOT_PULL_BUDGET_MS;
    const merged = await Promise.race([
      bootPullInner(loaded, cfg, () => cancelled),
      new Promise<null>((resolve) =>
        setTimeout(() => {
          cancelled = true;
          resolve(null);
        }, budget),
      ),
    ]);
    if (merged === null) void setBootFailCount((await getBootFailCount()) + 1);
    return merged ?? loaded;
  } catch (e) {
    console.warn("[koden] sync boot pull failed:", e);
    void setBootFailCount(1);
    return loaded;
  }
}

async function bootPullInner(
  loaded: LoadedSpaces,
  cfg: SyncConfig,
  isCancelled: () => boolean,
): Promise<LoadedSpaces | null> {
  const lastGen = await getLastGen("ws");
  const pulled = await pullDomain<WorkspaceEnvelope>(cfg.host, "ws", lastGen);
  if (isCancelled()) return null;
  if (pulled.status === "error") {
    noteFailure(pulled.message);
    // A torn host heals on the next push; make sure one happens.
    if (pulled.message === TORN_MESSAGE) wsPushSoon(true);
    return null;
  }
  void setBootFailCount(0);
  if (pulled.status === "absent" || pulled.status === "unchanged") {
    noteSuccess();
    wsPushSoon();
    return null;
  }
  const local = await loadedToLocal(loaded);
  const remote = envelopeFromWire(pulled.envelope, cfg.pathRoot);
  const merged = mergeWorkspace(local, remote);
  if (isCancelled()) return null;

  const spacesChanged =
    merged.changedSpaces.length > 0 || merged.removedSpaces.length > 0;
  if (spacesChanged) await saveSpacesList(merged.spaces);
  for (const id of merged.changedStates) {
    if (isCancelled()) return null;
    const st = merged.states.get(id);
    if (st) await saveState(id, st, merged.stateMeta[id] ?? { at: Date.now() });
  }
  void appendJournal(journalOf(merged, "boot", Date.now()));
  for (const id of merged.removedSpaces) {
    if (isCancelled()) return null;
    await deleteSpaceData(id);
  }
  if (isCancelled()) return null;
  await setLocalTombstones(merged.tombstones);
  await setLastGen("ws", pulled.gen);
  noteSuccess();
  if (merged.pushNeeded) wsPushSoon();

  const stateMeta = new Map(
    Object.entries(merged.stateMeta).map(([k, v]) => [k, { at: v.at }]),
  );
  // An absorbed duplicate follows its survivor; a truly removed space
  // (tombstone) falls back to boot's first-space default.
  const mappedActive = loaded.activeId
    ? (merged.idRemap[loaded.activeId] ?? loaded.activeId)
    : null;
  const activeId =
    mappedActive && merged.removedSpaces.includes(mappedActive)
      ? null
      : mappedActive;
  return { spaces: merged.spaces, activeId, states: merged.states, stateMeta };
}

/** Push the local workspace envelope, merge-then-write. Remote changes found
 * mid-session are folded into the pushed envelope but NOT applied locally
 * (layout adoption is boot-only), and ws lastGen is deliberately NOT
 * advanced here, so the next boot still pulls and adopts them. */
async function syncWorkspace(cfg: SyncConfig, force = false): Promise<void> {
  if (backedOff(force)) return;
  try {
    const loaded = await loadAll();
    const local = await loadedToLocal(loaded);
    let envelope = envelopeToWire(local, cfg.pathRoot);
    const sig = JSON.stringify(envelope);
    if (lastPushedWsSig === null) lastPushedWsSig = await getWsSig();
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
      } else if (pulled.status === "error" && pulled.message !== TORN_MESSAGE) {
        noteFailure(pulled.message);
        return;
      }
      // A torn remote falls through to the push, which rewrites the
      // manifests consistently and heals the host.
    }
    const deviceId = await getDeviceId();
    await pushDomain(cfg.host, "ws", envelope, deviceId);
    lastPushedWsSig = sig;
    await setWsSig(sig);
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
// null until loaded from the meta store; persisted so a fresh boot doesn't
// re-push a byte-identical envelope with a new gen.
let lastPushedWsSig: string | null = null;
let stopFns: (() => void)[] = [];
let wsPushTimer: ReturnType<typeof setTimeout> | null = null;

function wsPushSoon(force = false): void {
  if (wsPushTimer) return;
  wsPushTimer = setTimeout(() => {
    wsPushTimer = null;
    const cfg = config();
    if (cfg) void syncWorkspace(cfg, force);
  }, WS_PUSH_SOON_MS);
}

// ---------------------------------------------------- live adoption (ADR-024)

// Tracked separately from the persisted ws lastGen ON PURPOSE: live adoption
// is additive-only (new doc tabs, doc renames) and must not consume the gen —
// the next boot still pulls the same envelope and does the full structural
// merge. Session-local, so a restart re-adopts harmlessly (idempotent).
let lastLiveWsGen = 0;

async function liveWsAdopt(cfg: SyncConfig): Promise<void> {
  const a = getLiveAdopters();
  if (!a) return;
  try {
    const pulled = await pullDomain<WorkspaceEnvelope>(
      cfg.host,
      "ws",
      lastLiveWsGen,
    );
    if (pulled.status !== "ok") return;
    lastLiveWsGen = pulled.gen;
    const remote = envelopeFromWire(pulled.envelope, cfg.pathRoot);
    const localSpaces = new Set(useSpaces.getState().spaces.map((s) => s.id));
    const tabs = a.listTabs();
    const loaded = await loadAll();
    const now = Date.now();
    const journal: JournalEntry[] = [];
    for (const [spaceId, st] of Object.entries(remote.states ?? {})) {
      if (!localSpaces.has(spaceId)) continue; // unseen spaces are boot's job
      const remoteMeta = remote.stateMeta?.[spaceId];
      const rClocks = new Map<string, number>();
      for (const id of tabIdentities(st.tabs)) {
        rClocks.set(id, remoteMeta?.tabs?.[id] ?? remoteMeta?.at ?? 0);
      }
      const plan = planLiveDocAdoption(spaceId, tabs, st);
      for (const d of plan.create) {
        // The adopted tab persists with the AUTHOR's clock, never ours.
        const prefix =
          d.kind === "notes" ? "n" : d.kind === "board" ? "b" : "k";
        const identity = `${prefix}:${d.id}`;
        expectClock(identity, rClocks.get(identity) ?? OBSERVED_CLOCK);
        a.adoptDocTab(spaceId, d.kind, d.id, d.title);
      }
      const renames = planLiveRenames(
        spaceId,
        tabs,
        loaded.states.get(spaceId),
        loaded.stateMeta.get(spaceId),
        st,
        remoteMeta,
        a.leafKey,
      );
      for (const r of renames) {
        expectClock(r.identity, r.clock, (tab) =>
          tab.kind === "terminal"
            ? (tab.customTitle ?? "") === r.title
            : "title" in tab && tab.title === r.title,
        );
        if (r.kind === "terminal") a.setCustomTitle(r.tabId, r.title);
        else a.renameTab(r.tabId, r.title);
        journal.push({
          at: now,
          spaceId,
          tabId: r.identity,
          field: "title",
          before: r.before,
          after: r.title,
          via: "live",
        });
        announceRename(a, r.tabId, r.kind, r.before, r.title);
      }
    }
    void appendJournal(journal);
    // Self-healing push (ADR-025): if the host lacks something local wins
    // on clock, a lost push race is repaired here instead of waiting for a
    // local edit that may never come.
    const merged = mergeWorkspace(await loadedToLocal(loaded), remote, now);
    if (merged.pushNeeded) wsPushSoon();
  } catch {
    // Best-effort by design; the docs/ws sync paths own error surfacing.
  }
}

/** A peer renamed a tab you are looking at: say so and offer the way back.
 * Undo writes without a ledger entry, so it stamps now and wins the merge. */
function announceRename(
  a: NonNullable<ReturnType<typeof getLiveAdopters>>,
  tabId: number,
  kind: "terminal" | "doc",
  before: string,
  after: string,
): void {
  const from = before || "(no name)";
  const to = after || "(no name)";
  toast(`Sync renamed "${from}" to "${to}"`, {
    description: "Changed on another device",
    action: {
      label: "Undo",
      onClick: () => {
        if (kind === "terminal") a.setCustomTitle(tabId, before);
        else a.renameTab(tabId, before);
      },
    },
  });
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
  let unsubDocs: (() => void) | null = null;

  const applyStatus = () => {
    const p = usePreferencesStore.getState();
    if (!p.syncEnabled) {
      useSyncStore.getState().setStatus("disabled");
      return;
    }
    if (!isValidSyncHost(p.syncHost.trim())) {
      // Enabled but misconfigured must be visible, not silently disabled.
      useSyncStore.getState().setStatus("error", "invalid sync host");
      return;
    }
    if (useSyncStore.getState().status === "disabled") {
      useSyncStore.getState().setStatus("idle");
    }
  };
  applyStatus();

  // Subscribe to the docs store only after hydration: the hydration setState
  // itself must not mark the store dirty, or every boot pushes a
  // byte-identical envelope and forces every peer into a full re-pull.
  void hydrateDocs().then(() => {
    docsDirty = false;
    unsubDocs = useDocsStore.subscribe(() => {
      if (!adopting) docsDirty = true;
    });
    const cfg = config();
    if (cfg) {
      lastDocsPull = Date.now();
      void syncDocs(cfg);
    }
  });

  const docsPushTimer = setInterval(() => {
    const cfg = config();
    if (cfg && docsDirty) void syncDocs(cfg);
  }, DOCS_PUSH_DEBOUNCE_MS);

  // Visible windows poll fast (ADR-024 liveness: a note typed on the other
  // machine lands within ~10 s); hidden windows keep the slow timer only.
  const docsFastTimer = setInterval(() => {
    if (document.visibilityState !== "visible") return;
    const cfg = config();
    if (!cfg) return;
    lastDocsPull = Date.now();
    void syncDocs(cfg);
  }, DOCS_PULL_FAST_MS);

  const docsPullTimer = setInterval(() => {
    if (document.visibilityState === "visible") return; // fast timer covers it
    const cfg = config();
    if (!cfg) return;
    lastDocsPull = Date.now();
    void syncDocs(cfg);
  }, DOCS_PULL_INTERVAL_MS);

  const wsTimer = setInterval(() => {
    const cfg = config();
    if (cfg) void syncWorkspace(cfg);
  }, WS_CHECK_INTERVAL_MS);

  const wsLiveTimer = setInterval(() => {
    if (document.visibilityState !== "visible") return;
    const cfg = config();
    if (cfg) void liveWsAdopt(cfg);
  }, WS_LIVE_POLL_MS);

  // Local layout edits push soon instead of waiting for the 60 s check.
  setWsChangedListener(() => wsPushSoon());

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
    () => unsubDocs?.(),
    unsubPrefs,
    () => clearInterval(docsPushTimer),
    () => clearInterval(docsFastTimer),
    () => clearInterval(docsPullTimer),
    () => clearInterval(wsTimer),
    () => clearInterval(wsLiveTimer),
    () => setWsChangedListener(null),
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
