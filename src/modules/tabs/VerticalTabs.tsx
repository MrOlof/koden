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
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { SpaceMeta } from "@/modules/spaces";
import {
  Cancel01Icon,
  CheckListIcon,
  CommandLineIcon,
  ComputerTerminal02Icon,
  DashboardSquare01Icon,
  Globe02Icon,
  KanbanIcon,
  Note01Icon,
  PencilEdit02Icon,
  PlusSignIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useState } from "react";
import { TabIcon, TabRenameInput, TabStatusPill } from "./TabBar";
import { TabMenuItems } from "./TabMenuItems";
import { labelFor } from "./lib/tabLabel";
import { type EditorTab, isRenamableKind, type Tab } from "./lib/useTabs";

type Props = {
  tabs: Tab[];
  activeId: number;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  /** Open a fresh tab in this tab's space, inheriting its cwd. */
  onDuplicate: (id: number) => void;
  /** Close every other tab in this tab's space. */
  onCloseOthers: (id: number) => void;
  onRename: (id: number, title: string) => void;
  onNew: () => void;
  onNewGrid: () => void;
  onNewNotes: () => void;
  onNewBoard: () => void;
  onNewTasks: () => void;
  onNewEditor: () => void;
  onNewPreview: () => void;
  onOpenDirector: () => void;
  /** All spaces — feeds the "Move to space" submenu. */
  spaces: SpaceMeta[];
  onMoveToSpace: (tabId: number, spaceId: string) => void;
};

type NewTabMenuItemsProps = {
  onNew: () => void;
  onNewGrid: () => void;
  onNewEditor: () => void;
  onNewPreview: () => void;
  onNewNotes: () => void;
  onNewBoard: () => void;
  onNewTasks: () => void;
  onOpenDirector: () => void;
};

/**
 * The "new tab" actions, shared verbatim by the "+" dropdown and the rail
 * right-click menu so the two never drift. Rendered into a DropdownMenuContent
 * by both callers.
 */
function NewTabMenuItems({
  onNew,
  onNewGrid,
  onNewEditor,
  onNewPreview,
  onNewNotes,
  onNewBoard,
  onNewTasks,
  onOpenDirector,
}: NewTabMenuItemsProps) {
  return (
    <>
      <DropdownMenuItem onSelect={onNew}>
        <HugeiconsIcon
          icon={ComputerTerminal02Icon}
          size={14}
          strokeWidth={1.75}
        />
        <span className="flex-1">Terminal</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewGrid}>
        <HugeiconsIcon
          icon={DashboardSquare01Icon}
          size={14}
          strokeWidth={1.75}
        />
        <span className="flex-1">New grid…</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewEditor}>
        <HugeiconsIcon icon={PencilEdit02Icon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Editor</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewPreview}>
        <HugeiconsIcon icon={Globe02Icon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Preview</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewNotes}>
        <HugeiconsIcon icon={Note01Icon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Notes</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewBoard}>
        <HugeiconsIcon icon={KanbanIcon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Board</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onNewTasks}>
        <HugeiconsIcon icon={CheckListIcon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Tasks</span>
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={onOpenDirector}>
        <HugeiconsIcon icon={CommandLineIcon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Director</span>
      </DropdownMenuItem>
    </>
  );
}

/**
 * Vertical tab rail for the "sidebar" layout mode — VS Code style vertical
 * workspace navigation. Renders the active space's tabs as a column.
 */
export function VerticalTabs({
  tabs,
  activeId,
  onSelect,
  onClose,
  onDuplicate,
  onCloseOthers,
  onRename,
  onNew,
  onNewGrid,
  onNewNotes,
  onNewBoard,
  onNewTasks,
  onNewEditor,
  onNewPreview,
  onOpenDirector,
  spaces,
  onMoveToSpace,
}: Props) {
  const [editingId, setEditingId] = useState<number | null>(null);
  // Cursor-anchored "new tab" menu opened by right-clicking empty rail space.
  const [newMenuOpen, setNewMenuOpen] = useState(false);
  const [newMenuPoint, setNewMenuPoint] = useState({ x: 0, y: 0 });
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: right-click affordance on the tab rail; the tabs and buttons inside are the focusable controls
    <div
      className="flex h-full min-h-0 flex-col border-r border-border/60 bg-card"
      onContextMenu={(e) => {
        // A tab's own context menu (radix) preventDefaults as the event bubbles
        // up, so only empty rail space — where nothing claimed the event — opens
        // the new-tab menu. Same items as the "+" dropdown, at the cursor.
        if (e.defaultPrevented) return;
        e.preventDefault();
        setNewMenuPoint({ x: e.clientX, y: e.clientY });
        setNewMenuOpen(true);
      }}
    >
      <div className="flex shrink-0 items-center justify-between px-2 py-1.5">
        <span className="px-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
          Tabs
        </span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-6 shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
              title="New tab"
            >
              <HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={2} />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-40">
            <NewTabMenuItems
              onNew={onNew}
              onNewGrid={onNewGrid}
              onNewEditor={onNewEditor}
              onNewPreview={onNewPreview}
              onNewNotes={onNewNotes}
              onNewBoard={onNewBoard}
              onNewTasks={onNewTasks}
              onOpenDirector={onOpenDirector}
            />
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {tabs.map((t) => {
          const isActive = t.id === activeId;
          const isPreview = t.kind === "editor" && (t as EditorTab).preview;
          const isEditing = editingId === t.id && isRenamableKind(t.kind);
          return (
            <ContextMenu key={t.id}>
              <ContextMenuTrigger asChild>
                <div
                  className={cn(
                    "group mb-0.5 flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs transition-colors",
                    isActive
                      ? "bg-foreground/[0.07] text-foreground"
                      : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground",
                  )}
                >
                  {isEditing ? (
                    <>
                      <TabIcon tab={t} />
                      <TabRenameInput
                        initial={labelFor(t)}
                        onCommit={(value) => {
                          onRename(t.id, value);
                          setEditingId(null);
                        }}
                        onCancel={() => setEditingId(null)}
                      />
                    </>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onSelect(t.id)}
                      onDoubleClick={() => {
                        if (isRenamableKind(t.kind)) setEditingId(t.id);
                      }}
                      onAuxClick={(e) => {
                        if (e.button === 1 && tabs.length > 1) {
                          e.preventDefault();
                          onClose(t.id);
                        }
                      }}
                      className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                    >
                      <TabIcon tab={t} />
                      <span className={cn("truncate", isPreview && "italic")}>
                        {labelFor(t)}
                      </span>
                      {t.kind === "editor" && t.dirty ? (
                        <span className="size-1.5 shrink-0 rounded-full bg-foreground/70" />
                      ) : (
                        <TabStatusPill tabId={t.id} />
                      )}
                    </button>
                  )}
                  {!isEditing && tabs.length > 1 ? (
                    <button
                      type="button"
                      aria-label="Close tab"
                      onClick={() => onClose(t.id)}
                      className="rounded p-0.5 opacity-0 transition-opacity hover:bg-accent group-hover:opacity-60 hover:opacity-100"
                    >
                      <HugeiconsIcon
                        icon={Cancel01Icon}
                        size={11}
                        strokeWidth={2}
                      />
                    </button>
                  ) : null}
                </div>
              </ContextMenuTrigger>
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
      </div>
      <DropdownMenu open={newMenuOpen} onOpenChange={setNewMenuOpen}>
        {/* Zero-size anchor pinned to the cursor; the menu positions off it. */}
        <DropdownMenuTrigger
          aria-hidden
          tabIndex={-1}
          className="pointer-events-none fixed size-0"
          style={{ left: newMenuPoint.x, top: newMenuPoint.y }}
        />
        <DropdownMenuContent
          align="start"
          className="min-w-40"
          onCloseAutoFocus={(e) => e.preventDefault()}
        >
          <NewTabMenuItems
            onNew={onNew}
            onNewGrid={onNewGrid}
            onNewEditor={onNewEditor}
            onNewPreview={onNewPreview}
            onNewNotes={onNewNotes}
            onNewBoard={onNewBoard}
            onNewTasks={onNewTasks}
            onOpenDirector={onOpenDirector}
          />
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
