import {
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import type { SpaceMeta } from "@/modules/spaces";
import {
  Cancel01Icon,
  CancelCircleIcon,
  Copy01Icon,
  FolderTransferIcon,
  PencilEdit02Icon,
  PlusSignIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { isRenamableKind, type Tab } from "./lib/useTabs";

type Props = {
  tab: Tab;
  /** Number of tabs in this tab's space — gates Close / Close others. */
  tabCount: number;
  /** Begin the inline rename for this tab (sets the host's editingId). */
  onRename: () => void;
  onDuplicate: () => void;
  onNew: () => void;
  onClose: () => void;
  onCloseOthers: () => void;
  /** All spaces; the "Move to space" submenu lists every space but this one. */
  spaces: SpaceMeta[];
  onMoveToSpace: (spaceId: string) => void;
};

/**
 * The shared tab-level context-menu body. Rendered as the children of a
 * <ContextMenuContent> by both TabBar (horizontal strip) and VerticalTabs
 * (sidebar rail) so the two tab surfaces offer identical right-click actions.
 *
 * ponytail: this is a flat fragment of menu items, not a wrapper around
 * ContextMenuContent — each host owns its own Content so it keeps control of
 * positioning props (min-width, onCloseAutoFocus, etc.).
 */
export function TabMenuItems({
  tab,
  tabCount,
  onRename,
  onDuplicate,
  onNew,
  onClose,
  onCloseOthers,
  spaces,
  onMoveToSpace,
}: Props) {
  const isLastInSpace = tabCount <= 1;
  const otherSpaces = spaces.filter((s) => s.id !== tab.spaceId);
  return (
    <>
      {isRenamableKind(tab.kind) && (
        <ContextMenuItem onSelect={onRename}>
          <HugeiconsIcon icon={PencilEdit02Icon} size={14} strokeWidth={1.75} />
          <span className="flex-1">Rename</span>
        </ContextMenuItem>
      )}
      <ContextMenuItem onSelect={onDuplicate}>
        <HugeiconsIcon icon={Copy01Icon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Duplicate</span>
      </ContextMenuItem>
      <ContextMenuItem onSelect={onNew}>
        <HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={1.75} />
        <span className="flex-1">New tab</span>
      </ContextMenuItem>
      {otherSpaces.length > 0 && (
        <ContextMenuSub>
          <ContextMenuSubTrigger>
            <HugeiconsIcon
              icon={FolderTransferIcon}
              size={14}
              strokeWidth={1.75}
            />
            <span className="flex-1">Move to space</span>
          </ContextMenuSubTrigger>
          <ContextMenuSubContent className="min-w-40">
            {otherSpaces.map((s) => (
              <ContextMenuItem key={s.id} onSelect={() => onMoveToSpace(s.id)}>
                <span className="flex-1 truncate">{s.name}</span>
              </ContextMenuItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
      )}
      <ContextMenuSeparator />
      {!isLastInSpace && (
        <ContextMenuItem onSelect={onClose}>
          <HugeiconsIcon icon={Cancel01Icon} size={14} strokeWidth={1.75} />
          <span className="flex-1">Close</span>
        </ContextMenuItem>
      )}
      <ContextMenuItem disabled={isLastInSpace} onSelect={onCloseOthers}>
        <HugeiconsIcon icon={CancelCircleIcon} size={14} strokeWidth={1.75} />
        <span className="flex-1">Close others</span>
      </ContextMenuItem>
    </>
  );
}
