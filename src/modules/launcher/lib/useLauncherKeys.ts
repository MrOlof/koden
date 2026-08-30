import { type RefObject, useEffect, useRef } from "react";

// Elements carrying `data-launcher-stop` are what the arrow keys walk between;
// buttons and text inputs both work.
const LAUNCHER_STOP_ATTR = "data-launcher-stop";

export type LauncherNavKey = "ArrowDown" | "ArrowUp" | "Home" | "End";

export function isLauncherNavKey(key: string): key is LauncherNavKey {
  return (
    key === "ArrowDown" || key === "ArrowUp" || key === "Home" || key === "End"
  );
}

/**
 * Next stop index for a navigation key. `current` may sit between two stops
 * (i + 0.5) when focus is on something that is not a stop, so Down lands on
 * the following stop and Up on the preceding one; -0.5 means "before all".
 */
export function stepIndex(
  current: number,
  count: number,
  key: LauncherNavKey,
): number {
  if (count <= 0) return -1;
  const wrap = (i: number) => ((i % count) + count) % count;
  switch (key) {
    case "Home":
      return 0;
    case "End":
      return count - 1;
    case "ArrowDown":
      return wrap(Math.floor(current) + 1);
    case "ArrowUp":
      return wrap(Math.ceil(current) - 1);
  }
}

/** Index of `target` among `stops`, or i + 0.5 when it sits between stops. */
function locateStop(stops: readonly Node[], target: Node): number {
  const exact = stops.findIndex((el) => el === target || el.contains(target));
  if (exact >= 0) return exact;
  let last = -1;
  for (let i = 0; i < stops.length; i++) {
    const rel = stops[i].compareDocumentPosition(target);
    if (rel & Node.DOCUMENT_POSITION_FOLLOWING) last = i;
  }
  return last + 0.5;
}

function isTextField(el: EventTarget | null): boolean {
  return (
    el instanceof HTMLInputElement ||
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLElement && el.isContentEditable)
  );
}

type Options = {
  /** Focus the first stop on mount (default true). */
  focusFirst?: boolean;
};

function stopsIn(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(`[${LAUNCHER_STOP_ATTR}]`),
  );
}

/** Arrow / Home / End navigation across the launcher's stops. */
export function useLauncherKeys(
  ref: RefObject<HTMLElement | null>,
  { focusFirst = true }: Options = {},
) {
  // Read once: the option only decides the mount-time focus, and must not
  // pull focus back to the first row when it flips later.
  const focusFirstAtMount = useRef(focusFirst);

  useEffect(() => {
    const root = ref.current;
    if (!root || !focusFirstAtMount.current) return;
    const raf = requestAnimationFrame(() => stopsIn(root)[0]?.focus());
    return () => cancelAnimationFrame(raf);
  }, [ref]);

  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const stops = () => stopsIn(root);

    const onKey = (e: KeyboardEvent) => {
      if (!isLauncherNavKey(e.key)) return;
      if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      const target = e.target;
      if (!(target instanceof Node)) return;
      // Home / End move the caret inside text fields; only the arrows leave.
      if (isTextField(target) && (e.key === "Home" || e.key === "End")) return;
      const list = stops();
      if (list.length === 0) return;
      const next = list[stepIndex(locateStop(list, target), list.length, e.key)];
      if (!next) return;
      e.preventDefault();
      next.focus();
      next.scrollIntoView({ block: "nearest" });
    };

    root.addEventListener("keydown", onKey);
    return () => root.removeEventListener("keydown", onKey);
  }, [ref]);
}
