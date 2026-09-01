import { create } from "zustand";

export type SyncStatus = "disabled" | "idle" | "syncing" | "offline" | "error";

// Written by the engine, read by the statusbar segment.
type State = {
  status: SyncStatus;
  lastSyncAt: number | null;
  lastError: string | null;
  setStatus: (status: SyncStatus, error?: string | null) => void;
  markSynced: () => void;
};

export const useSyncStore = create<State>((set) => ({
  status: "disabled",
  lastSyncAt: null,
  lastError: null,
  setStatus: (status, error = null) => set({ status, lastError: error }),
  markSynced: () =>
    set({ status: "idle", lastSyncAt: Date.now(), lastError: null }),
}));
