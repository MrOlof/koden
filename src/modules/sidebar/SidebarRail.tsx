import { cn } from "@/lib/utils";
import type { LayoutMode } from "@/modules/tabs/lib/useLayoutMode";
import {
  FolderGitTwoIcon,
  FolderTreeIcon,
  UserGroupIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import type { SidebarViewId } from "./types";

export const SIDEBAR_RAIL_HEIGHT = 36;
// Width of the slim vertical activity rail shown in sidebar layout mode. It
// stays mounted outside the collapsible Files/Source-Control column so the user
// can always click to expand it back.
export const SIDEBAR_RAIL_WIDTH = 40;

type RailItem = {
  id: SidebarViewId;
  label: string;
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"];
  badge?: number;
};

type Props = {
  activeView: SidebarViewId;
  onSelectView: (view: SidebarViewId) => void;
  changedCount: number;
  agentCount: number;
  layoutMode: LayoutMode;
  /**
   * "horizontal" (default): the original strip at the bottom of the primary
   * sidebar, used in "top" layout mode.
   * "vertical": a slim always-visible activity rail at the far left, used in
   * "sidebar" layout mode where the primary column itself collapses away.
   */
  orientation?: "horizontal" | "vertical";
  /**
   * When the primary sidebar column is collapsed, no rail item is "active"
   * (clicking one expands the column to that view).
   */
  collapsed?: boolean;
};

export function SidebarRail({
  activeView,
  onSelectView,
  changedCount,
  agentCount,
  layoutMode,
  orientation = "horizontal",
  collapsed = false,
}: Props) {
  // In sidebar mode Agents live exclusively in the Tabs+Agents column, so the
  // rail only offers Files and Source Control.
  const items: RailItem[] = [
    { id: "explorer", label: "Files", icon: FolderTreeIcon },
    {
      id: "source-control",
      label: "Source Control",
      icon: FolderGitTwoIcon,
      badge: changedCount,
    },
    ...(layoutMode === "sidebar"
      ? []
      : [
          {
            id: "agents" as const,
            label: "Agents",
            icon: UserGroupIcon,
            badge: agentCount,
          },
        ]),
  ];

  const isVertical = orientation === "vertical";

  return (
    <div
      style={isVertical ? { width: SIDEBAR_RAIL_WIDTH } : { height: SIDEBAR_RAIL_HEIGHT }}
      className={cn(
        "flex shrink-0 gap-1 bg-card/85 backdrop-blur",
        isVertical
          ? "h-full flex-col items-stretch border-r border-border/60 px-1.5 py-2"
          : "items-stretch border-t border-border/60 px-1.5 py-1",
      )}
    >
      {items.map((item) => {
        const isActive = !collapsed && item.id === activeView;
        const showBadge = !!item.badge && item.badge > 0;
        return (
          <button
            key={item.id}
            type="button"
            aria-label={item.label}
            title={isVertical ? item.label : undefined}
            aria-pressed={isActive}
            onClick={() => onSelectView(item.id)}
            className={cn(
              "group relative flex cursor-pointer items-center justify-center rounded-md font-medium outline-none transition-colors duration-[var(--dur-base)]",
              "focus-visible:ring-2 focus-visible:ring-primary/40",
              isVertical
                ? "h-9 w-full"
                : "flex-1 gap-1.5 text-[11px]",
              isActive
                ? "bg-foreground/[0.07] text-foreground dark:bg-foreground/[0.09]"
                : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground",
            )}
          >
            <HugeiconsIcon
              icon={item.icon}
              size={isVertical ? 17 : 14}
              strokeWidth={isActive ? 2 : 1.75}
              className="shrink-0 transition-[stroke-width] duration-[var(--dur-base)]"
            />
            {!isVertical ? <span>{item.label}</span> : null}
            {showBadge ? (
              <span
                className={cn(
                  "inline-flex h-4 min-w-4 items-center justify-center rounded-full border border-border/60 bg-card px-1 text-[9px] font-semibold leading-none tabular-nums text-muted-foreground/95",
                  isVertical &&
                    "absolute -right-0.5 -top-0.5 h-3.5 min-w-3.5 px-0.5",
                )}
              >
                {item.badge! > 99 ? "99+" : item.badge}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
