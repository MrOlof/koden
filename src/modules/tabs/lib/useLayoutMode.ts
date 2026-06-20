import { useCallback, useEffect, useState } from "react";

export type LayoutMode = "top" | "sidebar";

const STORAGE_KEY = "koden.layout.mode";

function read(): LayoutMode {
  try {
    const v = window.localStorage.getItem(STORAGE_KEY);
    if (v === "top" || v === "sidebar") return v;
  } catch {
    // private mode / storage disabled
  }
  // Default to the VS Code-style sidebar layout: a single always-on left
  // sidebar (Tabs over Agents). The header tab strip ("top") is opt-in.
  return "sidebar";
}

/**
 * Shell layout mode: horizontal tab strip in the header ("top") or a vertical
 * tab rail beside the sidebar ("sidebar", VS Code style). Persisted to
 * localStorage like the other shell-chrome state (sidebar width/view).
 */
export function useLayoutMode() {
  const [layoutMode, setLayoutModeState] = useState<LayoutMode>(read);

  const setLayoutMode = useCallback((mode: LayoutMode) => {
    setLayoutModeState(mode);
    try {
      window.localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      // ignore
    }
  }, []);

  const toggleLayoutMode = useCallback(() => {
    setLayoutMode(read() === "top" ? "sidebar" : "top");
  }, [setLayoutMode]);

  // Mirror cross-window changes (settings window etc.) without a store.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === STORAGE_KEY) setLayoutModeState(read());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  return { layoutMode, setLayoutMode, toggleLayoutMode };
}
