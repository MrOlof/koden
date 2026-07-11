import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { clipboardWriteText } from "@/lib/clipboard";
import { fmtShortcut, MOD_KEY } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  type PaneColorPalette,
  setPaneColorMode,
  setPaneColorPalette,
} from "@/modules/settings/store";
import { NotePane } from "@/modules/workspace-docs/NotesStack";
import { TaskPane } from "@/modules/workspace-docs/TasksStack";
import {
  Cancel01Icon,
  CheckListIcon,
  MoreVerticalIcon,
  Note01Icon,
  Search01Icon,
  SparklesIcon,
  TerminalIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import type { SearchAddon } from "@xterm/addon-search";
import {
  DropIndicator,
  type Edge,
  useDraggable,
  useDroppable,
} from "@/modules/dnd";
import { Fragment, type ReactNode, useEffect, useRef, useState } from "react";
import { useTerminalDropStore } from "./lib/dropStore";
import { getLiveSlotForLeaf } from "./lib/rendererPool";
import { basenameOf, usePaneTitleStore } from "./lib/paneTitles";
import { paneColorAt } from "./lib/paneAutoColor";
import { leafIds, type PaneNode, type SplitSide } from "./lib/panes";
import { TerminalHistoryPopover } from "./TerminalHistoryPopover";
import { TerminalPane, type TerminalPaneHandle } from "./TerminalPane";

/** While a pane drag is in flight, which leaf is hovered + the nearest edge. */
export type PaneDropTarget = { leafId: number; side: SplitSide } | null;

/** Auto-color palettes offered inline in the pane menu. */
const PALETTES = ["muted", "vibrant", "pastel"] as const;

/** The pane type to open. */
export type SplitPaneType = "terminal" | "note" | "tasks";
/** Where the new pane lands relative to the source pane. */
export type SplitDirection = SplitSide;

type LeafBundle = {
  setRef: (h: TerminalPaneHandle | null) => void;
  onSearchReady: (leafId: number, addon: SearchAddon) => void;
  onCwd: (leafId: number, cwd: string) => void;
  onExit: (leafId: number, code: number) => void;
};

type Props = {
  node: PaneNode;
  tabVisible: boolean;
  activeLeafId: number;
  blocks: boolean;
  /** Show a per-pane title bar (used when the tab is split into 2+ panes). */
  showHeaders: boolean;
  onFocusLeaf: (leafId: number) => void;
  getBundle: (leafId: number) => LeafBundle;
  /** Close a pane from its header (when provided). */
  onClosePane?: (leafId: number) => void;
  /** Open a new pane of `type` in `direction` relative to `leafId`. */
  onSplit?: (
    leafId: number,
    type: SplitPaneType,
    direction: SplitDirection,
  ) => void;
  /** True while any pane in this tab is being header-dragged. Gates drop-zone
   * pointer-events so overlays never block terminal text/resize when idle. */
  paneDragActive?: boolean;
  /** The leaf currently being dragged (hidden from itself as a drop target). */
  draggingLeafId?: number | null;
  /** Resolved hover target + edge, for rendering the drop indicator. */
  dropTarget?: PaneDropTarget;
};

export function PaneTreeView(props: Props) {
  const { node } = props;
  if (node.kind === "leaf") {
    return <PaneLeaf {...props} node={node} />;
  }

  return (
    <ResizablePanelGroup
      orientation={node.dir === "row" ? "horizontal" : "vertical"}
    >
      {node.children.map((child, i) => (
        // Keyed by the subtree's first leaf, not the node id: when a leaf is
        // split in place, the replacing split node gets a fresh id and would
        // otherwise remount the surviving pane.
        <Fragment key={leafIds(child)[0]}>
          {i > 0 && <ResizableHandle />}
          <ResizablePanel id={`pane-${child.id}`} minSize="10%">
            <PaneTreeView {...props} node={child} />
          </ResizablePanel>
        </Fragment>
      ))}
    </ResizablePanelGroup>
  );
}

function PaneLeaf({
  node,
  tabVisible,
  activeLeafId,
  blocks,
  onFocusLeaf,
  getBundle,
  onClosePane,
  onSplit,
  paneDragActive = false,
  draggingLeafId = null,
  dropTarget = null,
}: Props & { node: Extract<PaneNode, { kind: "leaf" }> }) {
  const focused = node.id === activeLeafId;
  const isNote = node.content === "note";
  const isTasks = node.content === "tasks";

  // The header is the drag handle; the content wrapper is the drop target. Both
  // key off the leaf id, so the moving leaf object (and its PTY) survives.
  const drag = useDraggable({
    id: node.id,
    data: { kind: "pane", leafId: node.id },
  });
  const drop = useDroppable({
    id: node.id,
    data: { kind: "pane", leafId: node.id },
  });

  // Don't show a landing zone on the pane being dragged itself.
  const isDropTarget =
    paneDragActive &&
    draggingLeafId !== node.id &&
    dropTarget?.leafId === node.id;

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: focus-sync wrapper, not a control; the focusable terminal/header live inside
    <div
      // dnd-kit tracks the draggable by THIS node ref; without it the
      // PointerSensor never starts (the header listeners fire into a null node).
      ref={drag.setNodeRef}
      onMouseDownCapture={() => {
        if (!focused) onFocusLeaf?.(node.id);
      }}
      // Catches focus from Tab, programmatic focus, or any path that
      // skips mousedown — keeps activeLeafId in sync with DOM focus.
      onFocus={() => {
        if (!focused) onFocusLeaf?.(node.id);
      }}
      data-pane-leaf={node.id}
      className={cn(
        "relative flex h-full w-full flex-col",
        // Focused pane (incl. the single unsplit pane) carries a 1px spruce
        // top hairline. Pseudo-element so it never shifts the xterm grid.
        focused &&
          "before:pointer-events-none before:absolute before:inset-x-0 before:top-0 before:z-10 before:h-px before:bg-primary before:content-['']",
        drag.isDragging && "opacity-60",
      )}
    >
      <PaneHeader
        leafId={node.id}
        cwd={node.cwd}
        kind={isNote ? "note" : isTasks ? "tasks" : "terminal"}
        focused={focused}
        onClose={onClosePane ? () => onClosePane(node.id) : undefined}
        onSplit={
          onSplit
            ? (type, direction) => onSplit(node.id, type, direction)
            : undefined
        }
        dragListeners={drag.listeners}
        dragAttributes={drag.attributes}
        setDragHandleRef={drag.setActivatorNodeRef}
      />
      <PaneContextMenu
        leafId={node.id}
        kind={isNote ? "note" : isTasks ? "tasks" : "terminal"}
        // Merge the context-menu's native-listener ref onto the drop target so
        // right-click is caught on the same node dnd uses, no extra DOM layer.
        contentRef={drop.setNodeRef}
        onSplit={
          onSplit
            ? (type, direction) => onSplit(node.id, type, direction)
            : undefined
        }
      >
        {isNote ? (
          <NotePane docId={node.docId ?? `pane-${node.id}`} embedded />
        ) : isTasks ? (
          <TaskPane listId={node.docId ?? `pane-${node.id}`} embedded />
        ) : (
          <>
            <TerminalPane
              leafId={node.id}
              visible={tabVisible}
              focused={focused}
              initialCwd={node.cwd}
              blocks={blocks ?? false}
              ref={getBundle?.(node.id).setRef}
              onSearchReady={getBundle?.(node.id).onSearchReady}
              onCwd={getBundle?.(node.id).onCwd}
              onExit={getBundle?.(node.id).onExit}
            />
            <DropOverlay leafId={node.id} />
          </>
        )}
        {/* Pane-drag landing overlay. Kept mounted only during an active pane
          drag and always pointer-events-none, so it never blocks terminal
          text selection or the resize gutters when idle. */}
        {paneDragActive && draggingLeafId !== node.id ? (
          <div
            aria-hidden
            className={cn(
              "pointer-events-none absolute inset-0 z-20 rounded-md transition-colors",
              isDropTarget && "bg-primary/10 ring-1 ring-inset ring-primary/40",
            )}
          >
            {isDropTarget ? (
              <DropIndicator edge={dropTarget.side as Edge} />
            ) : null}
          </div>
        ) : null}
      </PaneContextMenu>
    </div>
  );
}

/** Right-click access path to the same options as the header dots menu.
 *
 * Why this is NOT a Radix <ContextMenuTrigger>: the xterm grid is rendered into
 * a host div that rendererPool.ts creates with document.createElement and
 * appends imperatively — it lives OUTSIDE React's fiber tree. React 19 routes
 * synthetic events by walking fibers up from the event target, so a contextmenu
 * fired on xterm's canvas/helper-textarea never reaches a React onContextMenu
 * on any ancestor (the target has no fiber). That's why the Radix trigger's
 * handler never ran, nothing called preventDefault, and WebView2's native menu
 * won. (The header dots menu works because its button IS React-rendered.)
 *
 * Fix: bind a NATIVE capture-phase contextmenu listener on the pane content
 * node — native DOM events bubble regardless of fibers. It preventDefaults
 * (kills the WebView2 menu) and opens a controlled DropdownMenu anchored to a
 * fixed-position virtual element at the cursor. Capture phase also runs before
 * xterm's own (bubble) contextmenu listener, so we win the event cleanly.
 *
 * xterm tradeoff: right-click no longer falls through to any native/xterm
 * paste-on-right-click — the user wants the pane menu here, and paste stays
 * available via the keybinding. */
function PaneContextMenu({
  leafId,
  kind,
  onSplit,
  contentRef,
  children,
}: {
  leafId: number;
  kind: SplitPaneType;
  onSplit?: (type: SplitPaneType, direction: SplitDirection) => void;
  /** dnd-kit drop ref; merged onto the content node so both bind to one div. */
  contentRef: (node: HTMLElement | null) => void;
  children: ReactNode;
}) {
  const entry = usePaneTitleStore((s) => s.titles[leafId]);
  const setPaneColor = usePaneTitleStore((s) => s.setPaneColor);
  const paneColorTerminal = usePreferencesStore((s) => s.paneColorTerminal);
  const paneColorNotes = usePreferencesStore((s) => s.paneColorNotes);
  const paneColorTask = usePreferencesStore((s) => s.paneColorTask);
  const locked = entry?.locked ?? false;
  const accent = entry?.color;
  const defaultAccent =
    kind === "note"
      ? paneColorNotes
      : kind === "tasks"
        ? paneColorTask
        : paneColorTerminal;
  const titleColor = accent ?? defaultAccent;

  const [open, setOpen] = useState(false);
  // Cursor point the menu anchors to. State (not a ref) so the compiler treats
  // the new position as a reactive input and re-renders the anchor at it.
  const [point, setPoint] = useState({ x: 0, y: 0 });
  // Whether the terminal had a selection when the menu opened — gates the
  // "Ask Librarian about selection" row (same precondition as the Mod+J
  // shortcut; App disables that binding when nothing is selected).
  const [hasSelection, setHasSelection] = useState(false);
  const nodeRef = useRef<HTMLDivElement | null>(null);

  // Native capture listener: fires for events over the foreign xterm DOM that
  // React's synthetic system can't see, and runs before xterm's own handler.
  // Mounted unconditionally (hooks can't be conditional under the React
  // compiler); the handler early-returns when there's no split handler.
  useEffect(() => {
    // Bind to the whole pane leaf (header + content), not just the content
    // node, so right-clicking the title bar also opens the pane menu — not only
    // the terminal body. Falls back to the content node if the wrapper isn't found.
    const node =
      nodeRef.current?.closest<HTMLElement>("[data-pane-leaf]") ??
      nodeRef.current;
    if (!node) return;
    const onContextMenu = (e: MouseEvent) => {
      if (!onSplit) return; // single-pane tab: leave right-click to xterm
      // Right-clicking a live terminal selection copies it (classic terminal
      // behavior) instead of opening the pane menu — but only over the terminal
      // body (nodeRef = the content div; the header is a sibling) and only when
      // text is actually selected. An empty-area right-click, or a right-click
      // on the header, still opens the pane menu.
      // ponytail: notes/tasks have no renderer slot, so getLiveSlotForLeaf is
      // null for them and this is terminal-only by construction.
      const content = nodeRef.current;
      if (content && e.target instanceof Node && content.contains(e.target)) {
        const slot = getLiveSlotForLeaf(leafId);
        const sel = slot?.term.hasSelection() ? slot.term.getSelection() : "";
        if (sel) {
          e.preventDefault();
          e.stopPropagation();
          void clipboardWriteText(sel);
          slot?.term.clearSelection();
          return;
        }
      }
      e.preventDefault();
      e.stopPropagation();
      setPoint({ x: e.clientX, y: e.clientY });
      // Header right-clicks keep the terminal selection alive (the copy path
      // above only claims content-area clicks), so snapshot it here.
      setHasSelection(
        getLiveSlotForLeaf(leafId)?.term.hasSelection() ?? false,
      );
      setOpen(true);
    };
    node.addEventListener("contextmenu", onContextMenu, { capture: true });
    return () =>
      node.removeEventListener("contextmenu", onContextMenu, { capture: true });
  }, [onSplit, leafId]);

  // Merge dnd's drop ref with our own node ref onto the single content div.
  const setRef = (node: HTMLDivElement | null) => {
    nodeRef.current = node;
    contentRef(node);
  };

  return (
    <div ref={setRef} className="relative min-h-0 flex-1">
      {children}
      {onSplit ? (
        <DropdownMenu open={open} onOpenChange={setOpen}>
          {/* Zero-size anchor pinned to the cursor; the menu positions off it. */}
          <DropdownMenuTrigger
            aria-hidden
            tabIndex={-1}
            className="pointer-events-none fixed size-0"
            style={{ left: point.x, top: point.y }}
          />
          <DropdownMenuContent
            align="start"
            className="min-w-40"
            onCloseAutoFocus={(e) => e.preventDefault()}
          >
            {kind === "terminal" ? (
              <>
                <DropdownMenuItem
                  disabled={!hasSelection}
                  onSelect={() =>
                    // App-level handler (askFromSelection) listens for this;
                    // same window-event decoupling as file attach.
                    window.dispatchEvent(
                      new CustomEvent("koden:ai-ask-selection"),
                    )
                  }
                >
                  <HugeiconsIcon
                    icon={SparklesIcon}
                    size={14}
                    strokeWidth={1.75}
                  />
                  <span className="flex-1">
                    Ask Librarian about selection
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {fmtShortcut(MOD_KEY, "J")}
                  </span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
              </>
            ) : null}
            <PaneMenuItems
              parts={DROPDOWN_PARTS}
              locked={locked}
              accent={titleColor}
              onSplit={onSplit}
              onSetColor={(color) => setPaneColor(leafId, color)}
            />
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}

function PaneHeader({
  leafId,
  cwd,
  kind,
  focused,
  onClose,
  onSplit,
  dragListeners,
  dragAttributes,
  setDragHandleRef,
}: {
  leafId: number;
  cwd?: string;
  kind: SplitPaneType;
  focused: boolean;
  onClose?: () => void;
  onSplit?: (type: SplitPaneType, direction: SplitDirection) => void;
  dragListeners?: ReturnType<typeof useDraggable>["listeners"];
  dragAttributes?: ReturnType<typeof useDraggable>["attributes"];
  setDragHandleRef?: ReturnType<typeof useDraggable>["setActivatorNodeRef"];
}) {
  const entry = usePaneTitleStore((s) => s.titles[leafId]);
  const renamePane = usePaneTitleStore((s) => s.renamePane);
  const setPaneColor = usePaneTitleStore((s) => s.setPaneColor);
  const paneColorTerminal = usePreferencesStore((s) => s.paneColorTerminal);
  const paneColorNotes = usePreferencesStore((s) => s.paneColorNotes);
  const paneColorTask = usePreferencesStore((s) => s.paneColorTask);
  // Gates the terminal-history search button (legacy "command minimap" pref key,
  // relabeled "Terminal command history" in settings — see GeneralSection).
  const historyEnabled = usePreferencesStore((s) => s.commandMinimapEnabled);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);
  // Empty string (e.g. a color-only entry on a fresh terminal) falls back to cwd.
  const label = entry?.label || basenameOf(cwd);
  const locked = entry?.locked ?? false;
  // The dot uses the explicit per-pane color only, so it keeps its focus cue.
  const accent = entry?.color;
  // The title text falls back to the per-type default color from settings.
  const defaultAccent =
    kind === "note"
      ? paneColorNotes
      : kind === "tasks"
        ? paneColorTask
        : paneColorTerminal;
  const titleColor = accent ?? defaultAccent;

  return (
    // The header bar is the pane's drag handle. dnd-kit's PointerSensor (5px
    // activation) means a plain click still focuses and a double-click still
    // renames; the rename input, close button, and PaneMenu trigger each
    // stopPropagation, so they keep working. Resize handles live on the gutters,
    // not here, so the two never collide.
    <div
      ref={setDragHandleRef}
      {...dragAttributes}
      {...dragListeners}
      className={cn(
        "group relative flex h-6 shrink-0 cursor-grab items-center justify-center border-b border-border/50 px-6 text-[11px] active:cursor-grabbing",
        focused ? "bg-card text-foreground" : "bg-card/60 text-muted-foreground",
      )}
    >
      <span
        className={cn(
          "absolute left-2 top-1/2 size-1.5 -translate-y-1/2 rounded-full",
          !accent && (focused ? "bg-primary" : "bg-muted-foreground/40"),
        )}
        style={accent ? { background: accent } : undefined}
      />
      {editing ? (
        <input
          ref={inputRef}
          defaultValue={entry?.label ?? ""}
          placeholder={basenameOf(cwd)}
          aria-label="Rename pane"
          className="min-w-0 max-w-full rounded-sm bg-background px-1 text-center font-mono text-[11px] text-foreground outline-none ring-1 ring-ring"
          // Pointer-down stop keeps dnd-kit's PointerSensor (on the header) from
          // starting a drag while selecting text in the rename field.
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          onBlur={(e) => {
            renamePane(leafId, e.target.value);
            setEditing(false);
          }}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") {
              renamePane(leafId, e.currentTarget.value);
              setEditing(false);
            } else if (e.key === "Escape") setEditing(false);
          }}
        />
      ) : (
        <button
          type="button"
          disabled={locked}
          title={locked ? label : "Double-click to rename"}
          onDoubleClick={() => {
            if (!locked) setEditing(true);
          }}
          className={cn(
            // 12px / medium / slight tracking keeps a tinted label legible on
            // dark bg (small thin glyphs are the hardest contrast case).
            "max-w-full truncate text-center font-mono text-xs font-medium tracking-[0.012em]",
            !locked && "cursor-text",
          )}
          style={titleColor ? { color: titleColor } : undefined}
        >
          {label}
        </button>
      )}
      {kind === "terminal" && historyEnabled ? (
        // Search/history trigger, sitting just LEFT of the "⋮" pane menu
        // (right-8 / 2rem). A small muted pill (icon + "Search" label) so it
        // reads as a search affordance, not a bare glyph; it grows leftward
        // from its right edge, staying clear of the ⋮ menu and ✕ close. Same
        // opacity/hover treatment as the close and menu icons.
        <Popover open={historyOpen} onOpenChange={setHistoryOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              aria-label="Search terminal history"
              title="Search terminal history"
              // Pointer-down stop keeps the header's drag sensor from claiming
              // the click as a drag start.
              onPointerDown={(e) => e.stopPropagation()}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
              className="absolute right-14 top-1/2 flex -translate-y-1/2 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground opacity-40 transition-opacity hover:bg-accent hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
            >
              <HugeiconsIcon icon={Search01Icon} size={12} strokeWidth={2} />
              Search
            </button>
          </PopoverTrigger>
          <PopoverContent
            align="end"
            className="w-80 gap-0 rounded-2xl p-1.5"
            onOpenAutoFocus={(e) => {
              // Let the cmdk input grab focus itself (it has autoFocus); this
              // stops Radix from focusing the content wrapper first.
              e.preventDefault();
            }}
            onPointerDown={(e) => e.stopPropagation()}
          >
            <TerminalHistoryPopover
              leafId={leafId}
              onClose={() => setHistoryOpen(false)}
            />
          </PopoverContent>
        </Popover>
      ) : null}
      {onClose ? (
        <button
          type="button"
          aria-label="Close pane"
          title="Close pane"
          // Pointer-down stop keeps the header's drag sensor from claiming the
          // close click as a drag start.
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onClose();
          }}
          className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground opacity-40 transition-opacity hover:bg-accent hover:text-foreground group-hover:opacity-100"
        >
          <HugeiconsIcon icon={Cancel01Icon} size={13} strokeWidth={2} />
        </button>
      ) : null}
      {onSplit ? (
        <PaneMenu
          locked={locked}
          accent={titleColor}
          onSplit={onSplit}
          onSetColor={(color) => setPaneColor(leafId, color)}
        />
      ) : null}
    </div>
  );
}

const SPLIT_TYPES: ReadonlyArray<{
  type: SplitPaneType;
  label: string;
  icon: typeof TerminalIcon;
}> = [
  { type: "terminal", label: "Terminal", icon: TerminalIcon },
  { type: "note", label: "Note", icon: Note01Icon },
  { type: "tasks", label: "Task", icon: CheckListIcon },
];

const SPLIT_DIRECTIONS: ReadonlyArray<{
  direction: SplitDirection;
  label: string;
}> = [
  { direction: "left", label: "Left" },
  { direction: "right", label: "Right" },
  { direction: "top", label: "Top" },
  { direction: "bottom", label: "Bottom" },
];

/** The menu primitives that render the split + title-color items. Both the
 * header dots menu and the pane right-click menu now mount the same Radix
 * DropdownMenu (the right-click path opens a controlled one at the cursor —
 * see PaneContextMenu), so a single parts bundle keeps the logic in one place.
 * ponytail: this used to also carry a ContextMenu-parts variant for a Radix
 * <ContextMenuTrigger>; that path was dropped because the trigger never fired
 * over xterm's out-of-fiber DOM, so the union collapsed to dropdown parts. */
type MenuParts = {
  Label: typeof DropdownMenuLabel;
  Item: typeof DropdownMenuItem;
  Separator: typeof DropdownMenuSeparator;
  Sub: typeof DropdownMenuSub;
  SubTrigger: typeof DropdownMenuSubTrigger;
  SubContent: typeof DropdownMenuSubContent;
  RadioGroup: typeof DropdownMenuRadioGroup;
  RadioItem: typeof DropdownMenuRadioItem;
};

const DROPDOWN_PARTS: MenuParts = {
  Label: DropdownMenuLabel,
  Item: DropdownMenuItem,
  Separator: DropdownMenuSeparator,
  Sub: DropdownMenuSub,
  SubTrigger: DropdownMenuSubTrigger,
  SubContent: DropdownMenuSubContent,
  RadioGroup: DropdownMenuRadioGroup,
  RadioItem: DropdownMenuRadioItem,
};

/** The split-type/direction submenus + the title-color submenu, rendered with
 * whichever menu primitive bundle is passed in. Both the header dots dropdown
 * and the pane right-click menu mount this, so the two stay in lockstep. */
function PaneMenuItems({
  parts,
  locked,
  accent,
  onSplit,
  onSetColor,
}: {
  parts: MenuParts;
  locked: boolean;
  accent?: string;
  onSplit: (type: SplitPaneType, direction: SplitDirection) => void;
  onSetColor: (color: string) => void;
}) {
  const {
    Label,
    Item,
    Separator,
    Sub,
    SubTrigger,
    SubContent,
    RadioGroup,
    RadioItem,
  } = parts;
  const colorRef = useRef<HTMLInputElement>(null);
  const paneColorMode = usePreferencesStore((s) => s.paneColorMode);
  const paneColorPalette = usePreferencesStore((s) => s.paneColorPalette);
  // Radio value collapses {mode, palette} into one choice: "off" = manual.
  const autoValue = paneColorMode === "automatic" ? paneColorPalette : "off";
  return (
    <>
      <Label>Split into</Label>
      {SPLIT_TYPES.map(({ type, label, icon }) => (
        <Sub key={type}>
          <SubTrigger>
            <HugeiconsIcon icon={icon} size={14} strokeWidth={1.75} />
            <span className="flex-1">{label}</span>
          </SubTrigger>
          <SubContent>
            {SPLIT_DIRECTIONS.map(({ direction, label: dirLabel }) => (
              <Item key={direction} onSelect={() => onSplit(type, direction)}>
                {dirLabel}
              </Item>
            ))}
          </SubContent>
        </Sub>
      ))}
      {!locked ? (
        <>
          <Separator />
          <Sub>
            <SubTrigger>
              <span
                aria-hidden
                className="size-3.5 rounded-full ring-1 ring-border"
                style={{ background: accent ?? "currentColor" }}
              />
              <span className="flex-1">Title color</span>
            </SubTrigger>
            <SubContent>
              <Item
                // Don't close the menu on select; hand off to the native picker.
                onSelect={(e) => {
                  e.preventDefault();
                  colorRef.current?.click();
                }}
              >
                <span
                  aria-hidden
                  className="size-3.5 rounded-full ring-1 ring-border"
                  style={{ background: accent ?? "currentColor" }}
                />
                <span className="flex-1">Custom…</span>
                <input
                  ref={colorRef}
                  type="color"
                  aria-label="Pane title color"
                  defaultValue={accent ?? "#9aa5b1"}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => onSetColor(e.target.value)}
                  className="pointer-events-none absolute size-0 opacity-0"
                />
              </Item>
              <Separator />
              <Label>Auto-color new panes</Label>
              <RadioGroup
                value={autoValue}
                onValueChange={(v) => {
                  if (v === "off") {
                    void setPaneColorMode("manual");
                  } else {
                    void setPaneColorMode("automatic");
                    void setPaneColorPalette(v as PaneColorPalette);
                  }
                }}
              >
                <RadioItem value="off">Off (manual)</RadioItem>
                {PALETTES.map((p) => (
                  <RadioItem key={p} value={p}>
                    <span
                      aria-hidden
                      className="size-3.5 rounded-full ring-1 ring-border"
                      style={{ background: paneColorAt(p, 0, 40) }}
                    />
                    <span className="flex-1 capitalize">{p}</span>
                  </RadioItem>
                ))}
              </RadioGroup>
            </SubContent>
          </Sub>
        </>
      ) : null}
    </>
  );
}

function PaneMenu({
  locked,
  accent,
  onSplit,
  onSetColor,
}: {
  locked: boolean;
  accent?: string;
  onSplit: (type: SplitPaneType, direction: SplitDirection) => void;
  onSetColor: (color: string) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Pane options"
          title="Split pane"
          // Pointer-down stop keeps the header's drag sensor from swallowing the
          // menu open.
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          className="absolute right-8 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground opacity-40 transition-opacity hover:bg-accent hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
        >
          <HugeiconsIcon icon={MoreVerticalIcon} size={13} strokeWidth={2} />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="min-w-40"
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <PaneMenuItems
          parts={DROPDOWN_PARTS}
          locked={locked}
          accent={accent}
          onSplit={onSplit}
          onSetColor={onSetColor}
        />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function DropOverlay({ leafId }: { leafId: number }) {
  const active = useTerminalDropStore((s) => s.targetLeafId === leafId);
  if (!active) return null;
  return (
    <div className="pointer-events-none absolute inset-2 grid place-items-center rounded-lg border border-primary/45 bg-background/70 text-xs font-medium text-foreground shadow-lg backdrop-blur-sm">
      Drop file path here
    </div>
  );
}
