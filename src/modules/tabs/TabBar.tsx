import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { fmtShortcut, MOD_KEY, SHIFT_KEY } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { BrainTabIcon } from "@/modules/brain";
import { fileIconUrl } from "@/modules/explorer/lib/iconResolver";
import {
  BookOpen01Icon,
  Cancel01Icon,
  CheckListIcon,
  Clock01Icon,
  CommandLineIcon,
  ComputerTerminal02Icon,
  DashboardSquare01Icon,
  GitBranchIcon,
  GitCompareIcon,
  Globe02Icon,
  HierarchySquare01Icon,
  Home01Icon,
  IncognitoIcon,
  KanbanIcon,
  MessageMultiple01Icon,
  Note01Icon,
  PencilEdit02Icon,
  PlusSignIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { STATUS_META } from "@/modules/orchestration/lib/roleMeta";
import type { SpaceMeta } from "@/modules/spaces";
import { isRenamableKind } from "./lib/useTabs";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { labelFor } from "./lib/tabLabel";
import { TabMenuItems } from "./TabMenuItems";
import { type TabStatus, useTabStatusStore } from "./lib/tabStatus";
import type { EditorTab, Tab } from "./lib/useTabs";

type Props = {
  tabs: Tab[];
  activeId: number;
  onSelect: (id: number) => void;
  onNew: () => void;
  onNewGrid: () => void;
  onNewBlock: () => void;
  onNewPrivate: () => void;
  onNewPreview: () => void;
  onNewEditor: () => void;
  onNewGitGraph: () => void;
  onNewNotes: () => void;
  onNewBoard: () => void;
  onNewTasks: () => void;
  onOpenDirector: () => void;
  /** Open the launcher ("New tab: choose") in the active space. */
  onOpenLauncher: () => void;
  onClose: (id: number) => void;
  /** Open a fresh tab in this tab's space, inheriting its cwd. */
  onDuplicate: (id: number) => void;
  /** Close every other tab in this tab's space. */
  onCloseOthers: (id: number) => void;
  /** Pin (promote) a preview tab to persistent on double-click. */
  onPin: (id: number) => void;
  /** Set a terminal tab's custom label; empty string resets to default. */
  onRename: (id: number, title: string) => void;
  /** All spaces — feeds the "Move to space" submenu. */
  spaces: SpaceMeta[];
  onMoveToSpace: (tabId: number, spaceId: string) => void;
  compact?: boolean;
};

export function TabBar({
  tabs,
  activeId,
  onSelect,
  onNew,
  onNewGrid,
  onNewBlock,
  onNewPrivate,
  onNewPreview,
  onNewEditor,
  onNewGitGraph,
  onNewNotes,
  onNewBoard,
  onNewTasks,
  onOpenDirector,
  onOpenLauncher,
  onClose,
  onDuplicate,
  onCloseOthers,
  onPin,
  onRename,
  spaces,
  onMoveToSpace,
  compact,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [editingId, setEditingId] = useState<number | null>(null);

  // Play the enter animation only for tabs opened after the first paint, never
  // the restored set and never on switch/reorder (triggers are keyed, so they
  // don't remount then). The ref is seeded with the initial ids on first render.
  const seenRef = useRef<Set<number> | null>(null);
  const firstRender = seenRef.current === null;
  let seen = seenRef.current;
  if (seen === null) {
    seen = new Set(tabs.map((t) => t.id));
    seenRef.current = seen;
  }
  useEffect(() => {
    seenRef.current = new Set(tabs.map((t) => t.id));
  }, [tabs]);

  // Single shared pill slides to the active tab instead of each tab toggling
  // its own background. Measured relative to the list (its offsetParent) so it
  // scrolls with the strip for free; transform/width only, no layout on siblings.
  const [pill, setPill] = useState<{ left: number; width: number } | null>(
    null,
  );
  const [pillReady, setPillReady] = useState(false);

  const measurePill = useCallback(() => {
    const el = listRef.current?.querySelector<HTMLElement>(
      '[data-tab-active="true"]',
    );
    setPill(el ? { left: el.offsetLeft, width: el.offsetWidth } : null);
  }, []);

  useLayoutEffect(() => {
    measurePill();
  }, [measurePill, activeId, tabs]);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const ro = new ResizeObserver(measurePill);
    ro.observe(list);
    return () => ro.disconnect();
  }, [measurePill]);

  // Hold the transition off until the pill is first placed, so it never slides
  // in from the origin on mount.
  useEffect(() => {
    if (pill && !pillReady) {
      const id = requestAnimationFrame(() => setPillReady(true));
      return () => cancelAnimationFrame(id);
    }
  }, [pill, pillReady]);

  // Horizontal wheel scroll without holding shift.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      if (el.scrollWidth <= el.clientWidth) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Keep the active tab visible after selection / open.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const active = el.querySelector<HTMLElement>(`[data-tab-id="${activeId}"]`);
    active?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeId]);

  return (
    <div
      ref={scrollRef}
      data-tauri-drag-region
      className="min-w-0 shrink overflow-x-auto [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      <div className="flex w-max items-center gap-0.5">
        <Tabs
          value={String(activeId)}
          onValueChange={(v) => onSelect(Number(v))}
        >
          <TabsList
            ref={listRef}
            className="relative h-7 w-max gap-0.5 bg-transparent p-0"
          >
            <span
              aria-hidden
              className="pointer-events-none absolute bottom-0 left-0 h-px bg-primary"
              style={
                pill
                  ? {
                      width: pill.width,
                      transform: `translateX(${pill.left}px)`,
                      transitionProperty: pillReady
                        ? "transform, width"
                        : "none",
                      transitionDuration: "var(--dur-base)",
                      transitionTimingFunction: "var(--ease-premium)",
                    }
                  : { opacity: 0 }
              }
            />
            {tabs.map((t) => {
              const isPreview = t.kind === "editor" && (t as EditorTab).preview;
              const isActive = t.id === activeId;
              const isNew = !firstRender && !seen.has(t.id);

              // While renaming, render a non-button cell so the <input> is not
              // nested inside the trigger <button> (invalid HTML, and WebKit
              // blocks focus/selection on inputs inside buttons).
              if (editingId === t.id && isRenamableKind(t.kind)) {
                return (
                  <div
                    key={t.id}
                    data-tab-id={t.id}
                    className={cn(
                      "flex h-7 shrink-0 items-center gap-1.5 rounded-md bg-accent text-xs text-foreground",
                      compact ? "px-1.5" : "px-2",
                    )}
                  >
                    <TabIcon tab={t} />
                    <TabRenameInput
                      initial={labelFor(t)}
                      onCommit={(value) => {
                        onRename(t.id, value);
                        setEditingId(null);
                      }}
                      onCancel={() => setEditingId(null)}
                    />
                  </div>
                );
              }

              const trigger = (
                <TabsTrigger
                  key={t.id}
                  value={String(t.id)}
                  data-tab-id={t.id}
                  data-tab-active={isActive ? "true" : undefined}
                  onDoubleClick={() => {
                    if (isPreview) onPin(t.id);
                    else if (isRenamableKind(t.kind)) setEditingId(t.id);
                  }}
                  onAuxClick={(e) => {
                    if (e.button === 1 && tabs.length > 1) {
                      e.preventDefault();
                      e.stopPropagation();
                      onClose(t.id);
                    }
                  }}
                  onMouseDown={(e) => {
                    if (e.button === 1) e.preventDefault();
                  }}
                  className={cn(
                    "group relative z-[1] h-7 shrink-0 justify-between gap-1.5 rounded-md bg-transparent text-xs transition-colors data-active:bg-transparent dark:data-active:bg-transparent",
                    isNew && "koden-tab-in",
                    isActive
                      ? "text-foreground dark:text-foreground"
                      : "text-muted-foreground hover:text-foreground/80 dark:text-muted-foreground",
                    compact
                      ? "px-1.5!"
                      : tabs.length === 1
                        ? "px-2!"
                        : "ps-2! pe-1!",
                  )}
                >
                  <span
                    className={cn(
                      "flex items-center gap-1.5 truncate",
                      compact ? "max-w-48" : "max-w-80",
                    )}
                  >
                    <TabIcon tab={t} />
                    {/* Preview tabs use italic to signal the transient state,
                        matching the visual convention from VSCode. */}
                    <span
                      className={cn(
                        "truncate font-mono tracking-[0.01em]",
                        isPreview && "italic",
                      )}
                    >
                      {labelFor(t)}
                    </span>
                    {t.kind === "editor" && t.dirty ? (
                      <span
                        aria-label="Unsaved changes"
                        className="size-1.5 shrink-0 rounded-full bg-foreground/70"
                      />
                    ) : (
                      <TabStatusPill tabId={t.id} />
                    )}
                  </span>
                  {tabs.length > 1 && (
                    <span
                      role="button"
                      aria-label="Close tab"
                      onClick={(e) => {
                        e.stopPropagation();
                        onClose(t.id);
                      }}
                      className="rounded p-0.5 opacity-0 transition-opacity hover:bg-accent hover:opacity-100 group-hover:opacity-60"
                    >
                      <HugeiconsIcon
                        icon={Cancel01Icon}
                        size={11}
                        strokeWidth={2}
                      />
                    </span>
                  )}
                </TabsTrigger>
              );

              return (
                <ContextMenu key={t.id}>
                  <ContextMenuTrigger asChild>{trigger}</ContextMenuTrigger>
                  <ContextMenuContent
                    className="min-w-44"
                    onCloseAutoFocus={(e) => e.preventDefault()}
                  >
                    <TabMenuItems
                      tab={t}
                      tabCount={tabs.length}
                      onRename={() => setEditingId(t.id)}
                      onDuplicate={() => onDuplicate(t.id)}
                      onNew={onNew}
                      onClose={() => onClose(t.id)}
                      onCloseOthers={() => onCloseOthers(t.id)}
                      spaces={spaces}
                      onMoveToSpace={(sid) => onMoveToSpace(t.id, sid)}
                    />
                  </ContextMenuContent>
                </ContextMenu>
              );
            })}
          </TabsList>
        </Tabs>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
              title={`New tab (${fmtShortcut(MOD_KEY, "T")})`}
            >
              <HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={2} />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="min-w-44"
            onCloseAutoFocus={(e) => e.preventDefault()}
          >
            <DropdownMenuItem onSelect={() => onOpenLauncher()}>
              <HugeiconsIcon icon={Home01Icon} size={14} strokeWidth={1.75} />
              <span className="flex-1">New tab: choose…</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, "N")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onNew()}>
              <HugeiconsIcon
                icon={ComputerTerminal02Icon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Terminal</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, "T")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewGrid()}>
              <HugeiconsIcon
                icon={DashboardSquare01Icon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">New grid…</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewBlock()}>
              <HugeiconsIcon
                icon={ComputerTerminal02Icon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Blocks</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, SHIFT_KEY, "T")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewPrivate()}>
              <HugeiconsIcon
                icon={IncognitoIcon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Privacy</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, "R")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewEditor()}>
              <HugeiconsIcon
                icon={PencilEdit02Icon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Editor</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, "E")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewPreview()}>
              <HugeiconsIcon icon={Globe02Icon} size={14} strokeWidth={1.75} />
              <span className="flex-1">Preview</span>
              <span className="text-xs text-muted-foreground">
                {fmtShortcut(MOD_KEY, SHIFT_KEY, "O")}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewGitGraph()}>
              <HugeiconsIcon
                icon={GitBranchIcon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Git Graph</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewNotes()}>
              <HugeiconsIcon icon={Note01Icon} size={14} strokeWidth={1.75} />
              <span className="flex-1">Notes</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewBoard()}>
              <HugeiconsIcon icon={KanbanIcon} size={14} strokeWidth={1.75} />
              <span className="flex-1">Board</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onNewTasks()}>
              <HugeiconsIcon icon={CheckListIcon} size={14} strokeWidth={1.75} />
              <span className="flex-1">Tasks</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onOpenDirector()}>
              <HugeiconsIcon
                icon={CommandLineIcon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Director</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

// Derived from the orchestration STATUS_META (single source of truth for
// status hues/labels) so the tab dot never drifts from the agent-dock dot.
// Tabs only surface the four roll-up states from tabStatus.ts.
const TAB_STATUS_META: Record<
  TabStatus,
  { color: string; pulse: boolean; label: string }
> = {
  working: {
    color: STATUS_META.working.dot,
    pulse: STATUS_META.working.pulse,
    label: STATUS_META.working.label,
  },
  waiting: {
    color: STATUS_META.waiting.dot,
    pulse: STATUS_META.waiting.pulse,
    label: STATUS_META.waiting.label,
  },
  done: {
    color: STATUS_META.done.dot,
    pulse: STATUS_META.done.pulse,
    label: STATUS_META.done.label,
  },
  error: {
    color: STATUS_META.error.dot,
    pulse: STATUS_META.error.pulse,
    label: STATUS_META.error.label,
  },
};

export function TabStatusPill({ tabId }: { tabId: number }) {
  const status = useTabStatusStore((s) => s.statuses[tabId]);
  if (!status) return null;
  const m = TAB_STATUS_META[status];
  return (
    <span
      aria-label={m.label}
      title={m.label}
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        m.pulse && "koden-pulse",
      )}
      style={{ background: m.color }}
    />
  );
}

export function TabIcon({ tab }: { tab: Tab }) {
  if (tab.kind === "editor" || tab.kind === "markdown") {
    const url = fileIconUrl(tab.title);
    return url ? <img src={url} alt="" className="size-3.5 shrink-0" /> : null;
  }
  if (tab.kind === "preview") {
    return (
      <HugeiconsIcon
        icon={Globe02Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "ai-diff") {
    return (
      <HugeiconsIcon
        icon={GitCompareIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "terminal" && tab.private) {
    return (
      <HugeiconsIcon
        icon={IncognitoIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "git-diff" || tab.kind === "git-commit-file") {
    return (
      <HugeiconsIcon
        icon={GitCompareIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "git-history") {
    return (
      <HugeiconsIcon
        icon={Clock01Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "notes") {
    return (
      <HugeiconsIcon
        icon={Note01Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "board") {
    return (
      <HugeiconsIcon
        icon={KanbanIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "tasks") {
    return (
      <HugeiconsIcon
        icon={CheckListIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "director") {
    return (
      <HugeiconsIcon
        icon={CommandLineIcon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "brain") {
    return <BrainTabIcon />;
  }
  if (tab.kind === "library") {
    return (
      <HugeiconsIcon
        icon={BookOpen01Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "brain-map") {
    return (
      <HugeiconsIcon
        icon={HierarchySquare01Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "launcher") {
    return (
      <HugeiconsIcon
        icon={Home01Icon}
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  if (tab.kind === "agent-topology" || tab.kind === "message-flow") {
    return (
      <HugeiconsIcon
        icon={
          tab.kind === "agent-topology"
            ? HierarchySquare01Icon
            : MessageMultiple01Icon
        }
        size={14}
        strokeWidth={2}
        className="shrink-0"
      />
    );
  }
  return (
    <HugeiconsIcon
      icon={ComputerTerminal02Icon}
      size={14}
      strokeWidth={2}
      className="shrink-0"
    />
  );
}

export function TabRenameInput({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  // Guards against a trailing blur re-resolving an edit that Enter/Escape
  // already finished (Escape must never commit).
  const done = useRef(false);

  useEffect(() => {
    // Focus on the next frame so it runs after the context menu restores focus
    // to its trigger when closing; a synchronous focus would be stolen.
    const raf = requestAnimationFrame(() => {
      ref.current?.focus();
      ref.current?.select();
    });
    return () => cancelAnimationFrame(raf);
  }, []);

  const finish = (fn: () => void) => {
    if (done.current) return;
    done.current = true;
    fn();
  };

  // explicit = the user pressed Enter, which pins even the unchanged label. A
  // plain blur with no change must not freeze the cwd-derived default into a
  // custom title.
  const commit = (value: string, explicit: boolean) => {
    if (!explicit && value.trim() === initial.trim()) finish(onCancel);
    else finish(() => onCommit(value));
  };

  return (
    <input
      ref={ref}
      defaultValue={initial}
      aria-label="Rename tab"
      className={cn(
        "w-28 min-w-0 rounded-sm bg-background px-1 text-xs text-foreground",
        "outline-none ring-1 ring-border focus:ring-ring",
      )}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") commit(e.currentTarget.value, true);
        else if (e.key === "Escape") finish(onCancel);
      }}
      onBlur={(e) => {
        // Switching windows/apps blurs the input; keep the edit open instead
        // of resolving it on the way out.
        if (!document.hasFocus()) return;
        commit(e.currentTarget.value, false);
      }}
    />
  );
}
