import { Button } from "@/components/ui/button";
import { WindowControls } from "@/components/WindowControls";
import {
  fmtShortcut,
  IS_MAC,
  MOD_KEY,
  USE_CUSTOM_WINDOW_CONTROLS,
} from "@/lib/platform";
import { NotificationBell } from "@/modules/agents";
import { useChatStore } from "@/modules/ai";
import { BrainHeaderMenu } from "@/modules/brain";
import type { SpaceMeta } from "@/modules/spaces";
import type { Tab } from "@/modules/tabs";
import { TabBar } from "@/modules/tabs";
import {
  Brain02Icon,
  CommandIcon,
  Settings01Icon,
  SidebarLeftIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  SearchInline,
  type SearchInlineHandle,
  type SearchTarget,
} from "./SearchInline";

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
  onClose: (id: number) => void;
  /** Open a fresh tab in this tab's space, inheriting its cwd. */
  onDuplicate: (id: number) => void;
  /** Close every other tab in this tab's space. */
  onCloseOthers: (id: number) => void;
  /** Promote a preview (transient) tab to persistent. */
  onPin: (id: number) => void;
  /** Set a terminal tab's custom label; empty string resets to default. */
  onRename: (id: number, title: string) => void;
  /** All spaces — feeds the tab "Move to space" submenu. */
  spaces: SpaceMeta[];
  onMoveToSpace: (tabId: number, spaceId: string) => void;
  onToggleSidebar: () => void;
  onOpenCommandPalette: () => void;
  /** Open the Koden Brain (search / memory / librarian). */
  onOpenBrain: () => void;
  /** Open the Koden Brain landing on the MEMORY view (ADR-020) — target of the
   *  bell's Librarian memory-activity entries. */
  onOpenBrainMemory: () => void;
  /** Open the Koden Brain Map (knowledge-graph view). */
  onOpenBrainMap: () => void;
  /** Open the Library, the Librarian's read-only wiki. */
  onOpenLibrary: () => void;
  onActivateAgent: (tabId: number, leafId: number) => void;
  onActivateLocalAgent: () => void;
  onOpenSettings: () => void;
  spaceSwitcher: ReactNode;
  searchTarget: SearchTarget;
  searchRef: RefObject<SearchInlineHandle | null>;
  /** Hide the horizontal tab strip (sidebar layout renders tabs vertically). */
  hideTabs?: boolean;
};

const COMPACT_WIDTH = 720;

export function Header({
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
  onClose,
  onDuplicate,
  onCloseOthers,
  onPin,
  onRename,
  spaces,
  onMoveToSpace,
  onToggleSidebar,
  onOpenCommandPalette,
  onOpenBrain,
  onOpenBrainMemory,
  onOpenBrainMap,
  onOpenLibrary,
  onActivateAgent,
  onActivateLocalAgent,
  onOpenSettings,
  spaceSwitcher,
  searchTarget,
  searchRef,
  hideTabs,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  const openMini = useChatStore((s) => s.openMini);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      setCompact(w < COMPACT_WIDTH);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const settingsButton = (
    <Button
      variant="ghost"
      size="icon"
      className="size-7 shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
      onClick={onOpenSettings}
      title="Settings"
    >
      <HugeiconsIcon icon={Settings01Icon} size={15} strokeWidth={1.75} />
    </Button>
  );

  return (
    <div
      ref={rootRef}
      data-tauri-drag-region
      className={`flex h-10 shrink-0 items-center gap-2 border-b border-border/60 bg-card select-none ${
        IS_MAC ? "pr-2 pl-20" : "pr-0 pl-2"
      }`}
    >
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          onClick={onToggleSidebar}
          title={`Toggle sidebar (${fmtShortcut(MOD_KEY, "B")})`}
          variant="ghost"
          size="icon-sm"
          className="shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <HugeiconsIcon icon={SidebarLeftIcon} size={18} strokeWidth={1.75} />
        </Button>

        <Button
          size="icon-sm"
          variant="ghost"
          onClick={onOpenCommandPalette}
          title={`Command palette (${fmtShortcut(MOD_KEY, "P")})`}
          className="shrink-0 gap-1.5 rounded-md px-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <HugeiconsIcon icon={CommandIcon} size={14} strokeWidth={1.75} />
        </Button>

        {!IS_MAC && (
          <NotificationBell
            onActivate={onActivateAgent}
            onActivateLocal={onActivateLocalAgent}
            onOpenBrain={onOpenBrainMemory}
          />
        )}
        {!IS_MAC && (
          <BrainHeaderMenu
            onOpenBrain={onOpenBrain}
            onOpenBrainMap={onOpenBrainMap}
            onOpenLibrary={onOpenLibrary}
            onOpenSettings={onOpenSettings}
          />
        )}
      </div>

      {!IS_MAC && <span className="mx-1 h-full w-px shrink-0 bg-border/70" />}

      {IS_MAC && <span className="mr-1 h-full w-px shrink-0 bg-border/70" />}

      <div
        className="flex min-w-0 flex-1 items-center gap-2"
        data-tauri-drag-region
      >
        {spaceSwitcher}
        {hideTabs ? null : (
          <TabBar
            tabs={tabs}
            activeId={activeId}
            onSelect={onSelect}
            onNew={onNew}
            onNewGrid={onNewGrid}
            onNewBlock={onNewBlock}
            onNewPrivate={onNewPrivate}
            onNewPreview={onNewPreview}
            onNewEditor={onNewEditor}
            onNewGitGraph={onNewGitGraph}
            onNewNotes={onNewNotes}
            onNewBoard={onNewBoard}
            onNewTasks={onNewTasks}
            onOpenDirector={onOpenDirector}
            onClose={onClose}
            onDuplicate={onDuplicate}
            onCloseOthers={onCloseOthers}
            onPin={onPin}
            onRename={onRename}
            spaces={spaces}
            onMoveToSpace={onMoveToSpace}
            compact={compact}
          />
        )}
        <div data-tauri-drag-region className="h-full min-w-2 flex-1" />
      </div>

      <Button
        size="sm"
        variant="outline"
        onClick={openMini}
        title={`Librarian — chat about your code & projects (${fmtShortcut(MOD_KEY, "I")})`}
        className="h-7 shrink-0 gap-1.5 rounded-md px-2 text-[12px]"
      >
        <HugeiconsIcon icon={Brain02Icon} size={14} strokeWidth={1.75} />
        {compact ? null : "Librarian"}
      </Button>

      <SearchInline ref={searchRef} target={searchTarget} compact={compact} />

      {IS_MAC && (
        <>
          <NotificationBell
            onActivate={onActivateAgent}
            onActivateLocal={onActivateLocalAgent}
            onOpenBrain={onOpenBrainMemory}
          />
          <BrainHeaderMenu
            onOpenBrain={onOpenBrain}
            onOpenBrainMap={onOpenBrainMap}
            onOpenLibrary={onOpenLibrary}
            onOpenSettings={onOpenSettings}
          />
          {settingsButton}
        </>
      )}

      {!IS_MAC && settingsButton}

      {USE_CUSTOM_WINDOW_CONTROLS && (
        <>
          <span className="ml-1 h-5 w-px shrink-0 bg-border/60" />
          <WindowControls />
        </>
      )}
    </div>
  );
}
