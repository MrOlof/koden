import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
} from "@/components/ui/empty";
import { cn } from "@/lib/utils";
import { useRetryStore } from "@/modules/agents/store/retryStore";
import type { Tab } from "@/modules/tabs";
import { usePaneTitleStore } from "@/modules/terminal";
import {
  ArrowDown01Icon,
  ArrowRight01Icon,
  Cancel01Icon,
  CommandLineIcon,
  ComputerTerminal02Icon,
  Delete02Icon,
  FilterHorizontalIcon,
  HierarchySquare01Icon,
  LayoutTwoColumnIcon,
  Menu01Icon,
  ReloadIcon,
  UserGroupIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { groupRootsByTab, statusCounts } from "../lib/grouping";
import {
  formatRelativeTime,
  formatTokens,
  ROLE_META,
  STATUS_META,
} from "../lib/roleMeta";
import { TEAM_TEMPLATES, type TeamTemplate } from "../lib/templates";
import { sortAgentsForDock } from "../lib/topology";
import { AGENT_STATUSES, type Agent, type AgentStatus } from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";
import { AgentTopologyView } from "./AgentTopologyView";

type DockView = "list" | "graph";
const VIEW_KEY = "koden.agentsView";
const COLLAPSED_KEY = "koden.agentsCollapsedTabs";
const STATUS_FILTER_KEY = "koden.agentsStatusFilter";
// Stable key for the null/"Other" group inside the collapsed-tabs Set (real tab
// ids are non-negative, so -1 can never collide with one).
const OTHER_TAB_KEY = -1;

function readDockView(): DockView {
  try {
    return localStorage.getItem(VIEW_KEY) === "graph" ? "graph" : "list";
  } catch {
    return "list";
  }
}

// Persisted as number[] (tab ids; OTHER_TAB_KEY for the "Other" group). Empty =
// everything expanded. Shaped exactly like readDockView/setViewPersist below.
function readCollapsedTabs(): Set<number> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return new Set();
    return new Set(arr.filter((n): n is number => typeof n === "number"));
  } catch {
    return new Set();
  }
}

function writeCollapsedTabs(next: Set<number>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
  } catch {
    // storage may be unavailable
  }
}

// Persisted as string[] of statuses. Default (and any unreadable state) = ALL
// statuses on, which the UI treats as "no filter".
function readStatusFilter(): Set<AgentStatus> {
  try {
    const raw = localStorage.getItem(STATUS_FILTER_KEY);
    if (!raw) return new Set(AGENT_STATUSES);
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return new Set(AGENT_STATUSES);
    const valid = arr.filter((s): s is AgentStatus =>
      (AGENT_STATUSES as readonly string[]).includes(s),
    );
    return new Set(valid);
  } catch {
    return new Set(AGENT_STATUSES);
  }
}

function writeStatusFilter(next: Set<AgentStatus>) {
  try {
    localStorage.setItem(STATUS_FILTER_KEY, JSON.stringify([...next]));
  } catch {
    // storage may be unavailable
  }
}

// Where clicking an agent should jump: its own terminal, or — for a native
// subagent that has none — its parent Director's terminal (where it reports).
function resolveOpenTarget(
  agent: Agent,
  agents: Record<string, Agent>,
): { tabId: number; leafId: number } | null {
  if (agent.tabId !== null && agent.leafId !== null) {
    return { tabId: agent.tabId, leafId: agent.leafId };
  }
  if (agent.parentId) {
    const p = agents[agent.parentId];
    if (p && p.tabId !== null && p.leafId !== null) {
      return { tabId: p.tabId, leafId: p.leafId };
    }
  }
  return null;
}

type Props = {
  /** Open tabs, used to group agents under their owning tab. */
  tabs: Tab[];
  /** Activate an agent's terminal tab, when it has one. */
  onActivateAgent?: (tabId: number, leafId: number) => void;
  /** Right-click the Director: launch its live command terminal (new tab). */
  onStartDirector?: () => void;
  /** Start the Director pre-loaded with a team template's roster. */
  onStartDirectorWithTemplate?: (template: TeamTemplate) => void;
  /** Right-click the Director: add it as a split pane in the active tab. */
  onAddDirectorToTab?: () => void;
  /** Right-click an agent: launch it as a Claude Code terminal. */
  onLaunchAgent?: (agent: Agent) => void;
  /** Remove an agent from the roster. */
  onRemoveAgent?: (id: string) => void;
  /** Clear the whole roster. */
  onClearRoster?: () => void;
};

/**
 * Sidebar control center: lists every agent with live status, and is the
 * right-click command surface. Stays visible regardless of the active tab.
 */
export function AgentDock({
  tabs,
  onActivateAgent,
  onStartDirector,
  onStartDirectorWithTemplate,
  onAddDirectorToTab,
  onLaunchAgent,
  onRemoveAgent,
  onClearRoster,
}: Props) {
  const agents = useOrchestrationStore((s) => s.agents);
  const list = useMemo(() => sortAgentsForDock(Object.values(agents)), [agents]);

  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNowMs(Date.now()), 15_000);
    return () => clearInterval(t);
  }, []);

  const [view, setView] = useState<DockView>(readDockView);
  const setViewPersist = (v: DockView) => {
    setView(v);
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {
      // storage may be unavailable
    }
  };

  // Per-tab collapse state (empty = all expanded). Toggling builds a NEW Set so
  // the React compiler sees a fresh reference.
  const [collapsedTabs, setCollapsedTabs] = useState<Set<number>>(
    readCollapsedTabs,
  );
  const toggleTabCollapse = (tabId: number | null) => {
    const key = tabId ?? OTHER_TAB_KEY;
    setCollapsedTabs((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      writeCollapsedTabs(next);
      return next;
    });
  };

  // Status filter (default = all statuses on, treated as "no filter").
  const [statusFilter, setStatusFilter] = useState<Set<AgentStatus>>(
    readStatusFilter,
  );
  const toggleStatus = (status: AgentStatus, on: boolean) => {
    setStatusFilter((prev) => {
      const next = new Set(prev);
      if (on) next.add(status);
      else next.delete(status);
      writeStatusFilter(next);
      return next;
    });
  };
  const resetStatusFilter = () => {
    const next = new Set(AGENT_STATUSES);
    setStatusFilter(next);
    writeStatusFilter(next);
  };
  const clearStatusFilter = () => {
    const next = new Set<AgentStatus>();
    setStatusFilter(next);
    writeStatusFilter(next);
  };
  // One-click isolate ("solo"): show only this status. Lets you go straight to
  // e.g. just "working" without unchecking the other eight one by one.
  const onlyStatus = (status: AgentStatus) => {
    const next = new Set<AgentStatus>([status]);
    setStatusFilter(next);
    writeStatusFilter(next);
  };
  // All-on means "no filter": don't gate any row.
  const isFilterActive = statusFilter.size < AGENT_STATUSES.length;

  // Roots are the terminal agents (a native subagent always has a parent and is
  // rendered nested under it, so it is never a root here).
  const roots = useMemo(
    () => list.filter((a) => !a.parentId || !agents[a.parentId]),
    [list, agents],
  );
  // Filtering applies to ROOTS only ("show only working terminals"); nested
  // director subagents ride along with their shown parent (see render below).
  const filteredRoots = useMemo(
    () => (isFilterActive ? roots.filter((r) => statusFilter.has(r.status)) : roots),
    [roots, statusFilter, isFilterActive],
  );
  const groups = useMemo(
    () => groupRootsByTab(filteredRoots, tabs),
    [filteredRoots, tabs],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-1.5 px-3 py-2.5">
        <HugeiconsIcon
          icon={UserGroupIcon}
          size={15}
          strokeWidth={1.75}
          className="text-muted-foreground"
        />
        <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Agents
        </span>
        <span className="text-[11px] tabular-nums text-muted-foreground">
          {isFilterActive ? `${filteredRoots.length} / ${roots.length}` : list.length}
        </span>
        <div className="ml-auto flex items-center gap-0.5">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label="Filter agents by status"
                title="Filter by status"
                className={cn(
                  "rounded p-0.5 transition-colors hover:bg-accent hover:text-foreground",
                  isFilterActive
                    ? "text-foreground"
                    : "text-muted-foreground",
                )}
              >
                <HugeiconsIcon
                  icon={FilterHorizontalIcon}
                  size={13}
                  strokeWidth={isFilterActive ? 2 : 1.75}
                />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuLabel>Show statuses</DropdownMenuLabel>
              {AGENT_STATUSES.map((s) => (
                <DropdownMenuCheckboxItem
                  key={s}
                  checked={statusFilter.has(s)}
                  onCheckedChange={(v) => toggleStatus(s, v === true)}
                  onSelect={(e) => e.preventDefault()}
                  className="group"
                >
                  <span
                    className={cn(
                      "size-2 shrink-0 rounded-full",
                      STATUS_META[s].pulse && "koden-pulse",
                    )}
                    style={{ background: STATUS_META[s].dot }}
                  />
                  <span className="flex-1">{STATUS_META[s].label}</span>
                  {/* "Solo" this status in one click. stopPropagation keeps the
                      parent checkbox from also toggling. */}
                  <button
                    type="button"
                    tabIndex={-1}
                    aria-label={`Show only ${STATUS_META[s].label}`}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      onlyStatus(s);
                    }}
                    className="rounded px-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground group-hover:opacity-100"
                  >
                    only
                  </button>
                </DropdownMenuCheckboxItem>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                disabled={!isFilterActive}
                onSelect={(e) => {
                  // Keep the menu open so you can immediately tick the ones you
                  // want — only an outside click should close it.
                  e.preventDefault();
                  resetStatusFilter();
                }}
              >
                <span className="flex-1">Show all</span>
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={statusFilter.size === 0}
                onSelect={(e) => {
                  e.preventDefault();
                  clearStatusFilter();
                }}
              >
                <span className="flex-1">None</span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {onStartDirectorWithTemplate ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  title="Start Director with a team template"
                  className="mr-0.5 flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                >
                  Team
                  <HugeiconsIcon icon={ArrowDown01Icon} size={11} strokeWidth={2} />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-64">
                <DropdownMenuLabel>Start Director with a team</DropdownMenuLabel>
                {TEAM_TEMPLATES.map((t) => (
                  <DropdownMenuItem
                    key={t.id}
                    className="flex-col items-start gap-0.5"
                    onSelect={() => onStartDirectorWithTemplate(t)}
                  >
                    <span className="text-xs font-medium text-foreground">
                      {t.name}
                    </span>
                    <span className="line-clamp-2 text-[11px] text-muted-foreground">
                      {t.members.map((m) => m.name).join(" · ")}
                    </span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          {onStartDirector ? (
            <HeaderButton
              icon={CommandLineIcon}
              label="Start Director (new tab)"
              onClick={onStartDirector}
            />
          ) : null}
          {onAddDirectorToTab ? (
            <HeaderButton
              icon={LayoutTwoColumnIcon}
              label="Add Director to current tab"
              onClick={onAddDirectorToTab}
            />
          ) : null}
          {list.length > 0 && onClearRoster ? (
            <HeaderButton
              icon={Delete02Icon}
              label="Clear roster"
              destructive
              onClick={onClearRoster}
            />
          ) : null}
        </div>
      </div>

      {view === "graph" ? (
        <div className="min-h-0 flex-1 p-2">
          <AgentTopologyView onActivateAgent={onActivateAgent} />
        </div>
      ) : list.length === 0 ? (
        <Empty className="flex-1 border-none p-5">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={UserGroupIcon} />
            </EmptyMedia>
            <EmptyDescription>
              No agents yet. Start the Director from the buttons above — blank
              to spawn agents as you go, or pick a team template — then give
              it a goal.
            </EmptyDescription>
            <EmptyDescription className="text-[11px] text-muted-foreground/70">
              Dot colors: blue working · amber needs you · green done · red
              error.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {groups.map((group) => {
            const groupKey = group.tabId ?? OTHER_TAB_KEY;
            const collapsed = collapsedTabs.has(groupKey);
            return (
              <Collapsible
                key={groupKey}
                open={!collapsed}
                onOpenChange={() => toggleTabCollapse(group.tabId)}
                className="mb-1"
              >
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
                  >
                    <HugeiconsIcon
                      icon={collapsed ? ArrowRight01Icon : ArrowDown01Icon}
                      size={12}
                      strokeWidth={2}
                      className="shrink-0 text-muted-foreground"
                    />
                    <span className="truncate text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                      {group.title}
                    </span>
                    <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/70">
                      {group.agents.length}
                    </span>
                    {/* Roll-up reflects the FILTERED roots, so a collapsed group
                        is consistent with what expanding reveals. */}
                    <StatusRollup agents={group.agents} />
                  </button>
                </CollapsibleTrigger>
                <CollapsibleContent>
                  {group.agents.map((agent) => {
                    const kids = list.filter((c) => c.parentId === agent.id);
                    return (
                      <Fragment key={agent.id}>
                        <AgentRow
                          agent={agent}
                          openTarget={resolveOpenTarget(agent, agents)}
                          nowMs={nowMs}
                          onActivateAgent={onActivateAgent}
                          onStartDirector={onStartDirector}
                          onAddDirectorToTab={onAddDirectorToTab}
                          onLaunchAgent={onLaunchAgent}
                          onRemoveAgent={onRemoveAgent}
                        />
                        {kids.length > 0 ? (
                          // Subagents hang off their parent: indented with a
                          // connecting rail, rendered shorter so the hierarchy
                          // reads at a glance. They ride along with their shown
                          // parent — the status filter only gates roots.
                          // ponytail: no per-kid filtering by design.
                          <div className="mb-0.5 ml-3 border-l border-border/60 pl-1.5">
                            {kids.map((child) => (
                              <AgentRow
                                key={child.id}
                                agent={child}
                                nested
                                openTarget={resolveOpenTarget(child, agents)}
                                nowMs={nowMs}
                                onActivateAgent={onActivateAgent}
                                onStartDirector={onStartDirector}
                                onAddDirectorToTab={onAddDirectorToTab}
                                onLaunchAgent={onLaunchAgent}
                                onRemoveAgent={onRemoveAgent}
                              />
                            ))}
                          </div>
                        ) : null}
                      </Fragment>
                    );
                  })}
                </CollapsibleContent>
              </Collapsible>
            );
          })}
        </div>
      )}

      <div className="flex shrink-0 items-stretch gap-1 border-t border-border/60 bg-card/85 px-1.5 py-1">
        <ViewTab
          active={view === "list"}
          onClick={() => setViewPersist("list")}
          icon={Menu01Icon}
          label="List"
        />
        <ViewTab
          active={view === "graph"}
          onClick={() => setViewPersist("graph")}
          icon={HierarchySquare01Icon}
          label="Graph"
        />
      </div>
    </div>
  );
}

/**
 * Compact per-group status roll-up: a small STATUS_META-colored dot + count for
 * each non-idle status present, in AGENT_STATUSES order. Dots match the row dot
 * size/colors exactly so the group header reads as a summary of its rows.
 */
function StatusRollup({ agents }: { agents: Agent[] }) {
  const counts = statusCounts(agents);
  // Skip "idle": it carries no signal (a plain shell), matching the rows where
  // idle agents show no task line.
  const present = AGENT_STATUSES.filter((s) => s !== "idle" && counts[s]);
  if (present.length === 0) return null;
  return (
    <span className="ml-auto flex shrink-0 items-center gap-1.5">
      {present.map((s) => (
        <span
          key={s}
          className="flex items-center gap-0.5 text-[10px] tabular-nums text-muted-foreground/80"
          title={STATUS_META[s].label}
        >
          <span
            className={cn(
              "size-1.5 rounded-full",
              STATUS_META[s].pulse && "koden-pulse",
            )}
            style={{ background: STATUS_META[s].dot }}
          />
          {counts[s]}
        </span>
      ))}
    </span>
  );
}

function HeaderButton({
  icon,
  label,
  onClick,
  destructive,
}: {
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"];
  label: string;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => onClick()}
      className={cn(
        "rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent",
        destructive ? "hover:text-destructive" : "hover:text-foreground",
      )}
    >
      <HugeiconsIcon icon={icon} size={13} strokeWidth={1.75} />
    </button>
  );
}

function ViewTab({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"];
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "flex flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-md py-1 text-[11px] font-medium transition-colors",
        active
          ? "bg-foreground/[0.07] text-foreground"
          : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground",
      )}
    >
      <HugeiconsIcon icon={icon} size={13} strokeWidth={active ? 2 : 1.75} />
      {label}
    </button>
  );
}

function AgentRow({
  agent,
  nested,
  openTarget,
  nowMs,
  onActivateAgent,
  onStartDirector,
  onAddDirectorToTab,
  onLaunchAgent,
  onRemoveAgent,
}: {
  agent: Agent;
  /** A subagent hanging off its parent: render compact + indented. */
  nested?: boolean;
  openTarget: { tabId: number; leafId: number } | null;
  nowMs: number;
  onActivateAgent?: (tabId: number, leafId: number) => void;
  onStartDirector?: () => void;
  onAddDirectorToTab?: () => void;
  onLaunchAgent?: (agent: Agent) => void;
  onRemoveAgent?: (id: string) => void;
}) {
  const role = ROLE_META[agent.role];
  const status = STATUS_META[agent.status];
  const tokens = agent.tokens.input + agent.tokens.output;
  // Prefer the terminal's (renamable) pane title over the cwd-derived name, so
  // renaming a terminal is reflected in its agent node.
  const paneTitle = usePaneTitleStore((s) =>
    agent.leafId !== null ? s.titles[agent.leafId]?.label : undefined,
  );
  const displayName = paneTitle?.trim() || agent.name;
  // Per-tab auto-retry override: only meaningful for an agent with its own
  // terminal leaf (a claude session the retry detector can arm).
  const retryLeafId = agent.leafId;
  const autoRetry = useRetryStore((s) =>
    retryLeafId !== null ? (s.enabledByLeaf[retryLeafId] ?? false) : false,
  );
  const isLinked = agent.tabId !== null && agent.leafId !== null;
  const canOpen = openTarget !== null && !!onActivateAgent;
  const isDirector = agent.role === "director";
  const done = agent.status === "done";

  const open = () => {
    if (openTarget) onActivateAgent?.(openTarget.tabId, openTarget.leafId);
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          type="button"
          onClick={open}
          disabled={!canOpen}
          title={
            !isLinked && canOpen ? "Open the Director's terminal" : undefined
          }
          className={cn(
            "mb-0.5 flex w-full flex-col gap-0.5 rounded-md border border-transparent px-2 text-left transition-[color,background,border,opacity]",
            nested ? "py-0.5" : "py-1",
            canOpen
              ? "cursor-pointer hover:border-border/60 hover:bg-accent/50"
              : "cursor-default",
            // Finished agents fade so live ones stand out; full opacity on hover.
            done && "opacity-55 hover:opacity-100",
          )}
        >
          <div className="flex items-center gap-1.5">
            <span
              className={cn(
                "flex shrink-0 items-center justify-center rounded",
                nested ? "size-3.5" : "size-4",
              )}
              style={{ background: `${role.accent}22`, color: role.accent }}
            >
              <HugeiconsIcon
                icon={role.icon}
                size={nested ? 9 : 11}
                strokeWidth={2}
              />
            </span>
            <span
              className={cn(
                "truncate font-medium text-foreground",
                nested ? "text-[11px]" : "text-xs",
              )}
            >
              {displayName}
            </span>
            <span
              className={cn(
                "ml-auto shrink-0 rounded-full",
                nested ? "size-1.5" : "size-2",
                status.pulse && "koden-pulse",
              )}
              style={{ background: status.dot }}
              title={status.label}
            />
          </div>
          {/* The dot color already conveys state, so no "Idle"/"Working" text;
              only a real task gets a line. Subagents (nested) stay a single
              compact row. */}
          {!nested ? (
            <>
              {agent.task ? (
                <div className="truncate text-[10.5px] text-muted-foreground">
                  {agent.task}
                </div>
              ) : null}
              <div className="flex items-center gap-2 text-[9.5px] text-muted-foreground/80">
                {agent.config.model ? (
                  <span className="font-mono">{agent.config.model}</span>
                ) : null}
                {tokens > 0 ? (
                  <span className="tabular-nums">
                    {formatTokens(tokens)} tok
                  </span>
                ) : null}
                <span className="ml-auto tabular-nums">
                  {formatRelativeTime(agent.lastActivityAt, nowMs)}
                </span>
              </div>
            </>
          ) : null}
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent
        className="min-w-44"
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        {isDirector ? (
          <>
            <ContextMenuItem onSelect={() => onStartDirector?.()}>
              <HugeiconsIcon
                icon={CommandLineIcon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">
                {isLinked ? "Open live command" : "Start live command"}
              </span>
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => onAddDirectorToTab?.()}>
              <HugeiconsIcon
                icon={LayoutTwoColumnIcon}
                size={14}
                strokeWidth={1.75}
              />
              <span className="flex-1">Add to current tab</span>
            </ContextMenuItem>
          </>
        ) : isLinked ? (
          <ContextMenuItem onSelect={open}>
            <HugeiconsIcon
              icon={ComputerTerminal02Icon}
              size={14}
              strokeWidth={1.75}
            />
            <span className="flex-1">Open terminal</span>
          </ContextMenuItem>
        ) : openTarget ? (
          <ContextMenuItem onSelect={open}>
            <HugeiconsIcon
              icon={ComputerTerminal02Icon}
              size={14}
              strokeWidth={1.75}
            />
            <span className="flex-1">View in Director's terminal</span>
          </ContextMenuItem>
        ) : (
          <ContextMenuItem onSelect={() => onLaunchAgent?.(agent)}>
            <HugeiconsIcon
              icon={ComputerTerminal02Icon}
              size={14}
              strokeWidth={1.75}
            />
            <span className="flex-1">Launch in terminal</span>
          </ContextMenuItem>
        )}
        {retryLeafId !== null ? (
          <>
            <ContextMenuSeparator />
            <ContextMenuCheckboxItem
              checked={autoRetry}
              onCheckedChange={(v) =>
                useRetryStore.getState().setEnabled(retryLeafId, v === true)
              }
              onSelect={(e) => e.preventDefault()}
            >
              <HugeiconsIcon icon={ReloadIcon} size={14} strokeWidth={1.75} />
              <span className="flex-1">Auto-retry on rate limit</span>
            </ContextMenuCheckboxItem>
          </>
        ) : null}
        <ContextMenuSeparator />
        <ContextMenuItem
          className="text-destructive focus:text-destructive"
          onSelect={() => onRemoveAgent?.(agent.id)}
        >
          <HugeiconsIcon icon={Cancel01Icon} size={14} strokeWidth={1.75} />
          <span className="flex-1">Remove</span>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
