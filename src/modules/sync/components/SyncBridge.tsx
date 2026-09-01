import { useEffect } from "react";
import { startSyncEngine } from "../lib/engine";

/** Mounts the sync engine once in the main window (ADR-023). Renders nothing.
 * The engine self-gates on the syncEnabled pref, so mounting is unconditional. */
export function SyncBridge() {
  useEffect(() => startSyncEngine(), []);
  return null;
}
