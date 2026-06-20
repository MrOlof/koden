import {
  DndContext,
  type DragEndEvent,
  type DragMoveEvent,
  type DragStartEvent,
  PointerSensor,
  useSensor,
  useSensors,
} from "@/modules/dnd";
import type { Tab } from "@/modules/tabs";
import type { SearchAddon } from "@xterm/addon-search";
import { useEffect, useMemo, useRef, useState } from "react";
import { selectLiveTerminals } from "./lib/liveTerminals";
import { leafIds, type SplitSide } from "./lib/panes";
import {
  type PaneDropTarget,
  PaneTreeView,
  type SplitDirection,
  type SplitPaneType,
} from "./PaneTreeView";
import type { TerminalPaneHandle } from "./TerminalPane";

type Props = {
  tabs: Tab[];
  activeId: number;
  /** Register/unregister handle by leaf id (not tab id). */
  registerHandle: (leafId: number, handle: TerminalPaneHandle | null) => void;
  onSearchReady: (leafId: number, addon: SearchAddon) => void;
  onCwd: (leafId: number, cwd: string) => void;
  onExit: (leafId: number, code: number) => void;
  onFocusLeaf: (tabId: number, leafId: number) => void;
  /** Close a single pane (used by the per-pane header close button). */
  onClosePane: (leafId: number) => void;
  /** Split a pane from its header dropdown into a terminal/note/tasks pane. */
  onSplit: (
    leafId: number,
    type: SplitPaneType,
    direction: SplitDirection,
  ) => void;
  /** Re-dock `sourceLeafId` against `targetLeafId`'s `side`. v1 only fires for
   * the active tab (the sole pointer-interactive one), so it carries no tab id —
   * App binds it to `activeId`. */
  onMovePane: (
    sourceLeafId: number,
    targetLeafId: number,
    side: SplitSide,
  ) => void;
};

type Bundle = {
  setRef: (h: TerminalPaneHandle | null) => void;
  onSearchReady: (leafId: number, addon: SearchAddon) => void;
  onCwd: (leafId: number, cwd: string) => void;
  onExit: (leafId: number, code: number) => void;
};

/** Nearest edge of `rect` to the pointer. Diagonal quadrants pick the closer
 * axis, so corners resolve to a single side rather than flickering. */
function nearestEdge(
  rect: DOMRect,
  x: number,
  y: number,
): SplitSide {
  // Diagonal quadrants, NOT raw pixel distance: normalize to the rect and split
  // by its two diagonals. Raw distance makes a tall/narrow pane almost always
  // pick left/right (top/bottom are only hittable in a few px), so dropping
  // above/below an existing pane was effectively impossible. With quadrants the
  // top third → top, bottom third → bottom, etc., regardless of aspect ratio.
  const fx = rect.width > 0 ? (x - rect.left) / rect.width : 0.5;
  const fy = rect.height > 0 ? (y - rect.top) / rect.height : 0.5;
  if (fy < fx && fy < 1 - fx) return "top";
  if (fy > fx && fy > 1 - fx) return "bottom";
  return fx < 0.5 ? "left" : "right";
}

export function TerminalStack({
  tabs,
  activeId,
  registerHandle,
  onSearchReady,
  onCwd,
  onExit,
  onFocusLeaf,
  onClosePane,
  onSplit,
  onMovePane,
}: Props) {
  const terminals = useMemo(() => selectLiveTerminals(tabs), [tabs]);

  const registerRef = useRef(registerHandle);
  const searchReadyRef = useRef(onSearchReady);
  const cwdRef = useRef(onCwd);
  const exitRef = useRef(onExit);
  useEffect(() => {
    registerRef.current = registerHandle;
  }, [registerHandle]);
  useEffect(() => {
    searchReadyRef.current = onSearchReady;
  }, [onSearchReady]);
  useEffect(() => {
    cwdRef.current = onCwd;
  }, [onCwd]);
  useEffect(() => {
    exitRef.current = onExit;
  }, [onExit]);

  const bundles = useRef(new Map<number, Bundle>());
  const getBundle = (leafId: number): Bundle => {
    let b = bundles.current.get(leafId);
    if (!b) {
      b = {
        setRef: (h) => registerRef.current(leafId, h),
        onSearchReady: (id, addon) => searchReadyRef.current(id, addon),
        onCwd: (id, cwd) => cwdRef.current(id, cwd),
        onExit: (id, code) => exitRef.current(id, code),
      };
      bundles.current.set(leafId, b);
    }
    return b;
  };

  useEffect(() => {
    const live = new Set<number>();
    for (const t of terminals)
      for (const id of leafIds(t.paneTree)) live.add(id);
    for (const id of bundles.current.keys()) {
      if (!live.has(id)) bundles.current.delete(id);
    }
  }, [terminals]);

  // Pointer-based DnD (dnd-kit): the Tauri OS file-drop channel swallows HTML5
  // DnD, so this is the only mechanism that survives. 5px activation distance
  // matches SpaceSwitcher and leaves single-click-focus / double-click-rename
  // on the header untouched.
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );

  // The id of the leaf currently being dragged by its header (null = idle), and
  // the resolved drop target. Both live in the stack so PaneTreeView can render
  // edge zones + the indicator, and so we can gate drop overlays' pointer-events.
  const [draggingLeafId, setDraggingLeafId] = useState<number | null>(null);
  const [dropTarget, setDropTarget] = useState<PaneDropTarget>(null);
  // Pointer origin (the activator event) so we can derive live coords from delta.
  const dragOrigin = useRef<{ x: number; y: number } | null>(null);

  return (
    <div className="relative h-full w-full">
      {terminals.map((t) => {
        const tabVisible = t.id === activeId;
        // One DndContext PER TAB scopes a drag to that pane tree, which enforces
        // the within-tab v1 boundary (a pane can't be dropped into another tab).
        const onDragStart = (e: DragStartEvent) => {
          const data = e.active.data.current;
          if (data?.kind !== "pane") return;
          const src = data.leafId as number;
          const ev = e.activatorEvent as PointerEvent;
          dragOrigin.current = { x: ev.clientX, y: ev.clientY };
          setDraggingLeafId(src);
          setDropTarget(null);
        };
        const onDragMove = (e: DragMoveEvent) => {
          const over = e.over;
          const origin = dragOrigin.current;
          if (!over || !origin || over.rect == null) {
            setDropTarget(null);
            return;
          }
          const targetLeafId = over.data.current?.leafId as number | undefined;
          if (targetLeafId === undefined) {
            setDropTarget(null);
            return;
          }
          const x = origin.x + e.delta.x;
          const y = origin.y + e.delta.y;
          const r = over.rect;
          const rect = new DOMRect(r.left, r.top, r.width, r.height);
          setDropTarget({ leafId: targetLeafId, side: nearestEdge(rect, x, y) });
        };
        const reset = () => {
          setDraggingLeafId(null);
          setDropTarget(null);
          dragOrigin.current = null;
        };
        const onDragEnd = (e: DragEndEvent) => {
          const data = e.active.data.current;
          const source = data?.kind === "pane" ? (data.leafId as number) : null;
          const target = dropTarget;
          reset();
          if (source !== null && target && source !== target.leafId)
            onMovePane(source, target.leafId, target.side);
        };
        return (
          <div
            key={t.id}
            className="absolute inset-0"
            style={{
              visibility: tabVisible ? "visible" : "hidden",
              pointerEvents: tabVisible ? "auto" : "none",
            }}
            aria-hidden={!tabVisible}
          >
            <DndContext
              sensors={sensors}
              onDragStart={onDragStart}
              onDragMove={onDragMove}
              onDragEnd={onDragEnd}
              onDragCancel={reset}
            >
              <PaneTreeView
                node={t.paneTree}
                tabVisible={tabVisible}
                activeLeafId={t.activeLeafId}
                blocks={t.blocks ?? false}
                showHeaders={leafIds(t.paneTree).length > 1}
                onFocusLeaf={(leafId) => onFocusLeaf(t.id, leafId)}
                getBundle={getBundle}
                onClosePane={onClosePane}
                onSplit={onSplit}
                paneDragActive={draggingLeafId !== null}
                draggingLeafId={draggingLeafId}
                dropTarget={dropTarget}
              />
            </DndContext>
          </div>
        );
      })}
    </div>
  );
}
