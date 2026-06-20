import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { toast } from "sonner";
import { installTestBus } from "@/dev/testBus";
import { getLaunchDir } from "@/lib/launchDir";
import { getDefaultFolder } from "@/modules/settings/store";
import { usePresence } from "@/lib/usePresence";
import { quoteShellArg } from "@/lib/shellQuote";
import { useZoom } from "@/lib/useZoom";
import { AgentNotificationsBridge, RetryBridge, UsageBridge } from "@/modules/agents";
import { useUsageStore } from "@/modules/agents/store/usageStore";
import {
  AgentRunBridge,
  AiMiniWindow,
  LocalAgentNotificationsBridge,
  useAiBootstrap,
  useAiLiveBridge,
  useChatStore,
} from "@/modules/ai";
import { AiComposerProvider } from "@/modules/ai/lib/composer";
import { native } from "@/modules/ai/lib/native";
import {
  CommandPalette,
  createCommandItems,
} from "@/modules/command-palette";
import {
  NewEditorDialog,
  useEditorFileSync,
  type EditorPaneHandle,
} from "@/modules/editor";
import { FileExplorer, type FileExplorerHandle } from "@/modules/explorer";
import type { GitHistorySearchHandle } from "@/modules/git-history";
import {
  Header,
  type SearchInlineHandle,
  type SearchTarget,
} from "@/modules/header";
import type { PreviewPaneHandle } from "@/modules/preview";
import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { isMarkdownPath } from "@/lib/utils";
import {
  useGlobalShortcuts,
  type ShortcutHandlers,
  type ShortcutId,
} from "@/modules/shortcuts";
import {
  SidebarRail,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  useSidebarPanel,
} from "@/modules/sidebar";
import {
  SourceControlPanel,
  useSourceControlContext,
} from "@/modules/source-control";
import { StatusBar } from "@/modules/statusbar";
import {
  GridDialog,
  type TerminalTab,
  useLayoutMode,
  useTabs,
  useTabStatusStore,
  useWindowTitle,
  useWorkspaceCwd,
  VerticalTabs,
} from "@/modules/tabs";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  clearFocusedTerminal,
  disposeSession,
  findLeaf,
  findLeafCwd,
  hasLeaf,
  leafIdForPty,
  leafIds,
  navigateFocusedBlocks,
  nextPaneColor,
  respawnSession,
  sideToSplit,
  type SplitPaneType,
  type SplitSide,
  submitToLeaf,
  type TerminalPaneHandle,
  usePaneTitleStore,
  useTerminalFileDrop,
  whenSessionReady,
  writeToSession,
} from "@/modules/terminal";
import {
  SpaceSwitcher,
  useSpaces,
  useSpacePersistence,
  useSpacesBoot,
} from "@/modules/spaces";
import { DEFAULT_SPACE_ID } from "@/modules/tabs/lib/useTabs";
import { ThemeProvider, useThemeFileEditing } from "@/modules/theme";
import { UpdaterDialog } from "@/modules/updater";
import { useWorkspaceEnvStore } from "@/modules/workspace";
import {
  type Agent,
  AgentBusBridge,
  AgentDock,
  AGENT_ROLES,
  type AgentRole,
  type DirectorCommand,
  DirectorBusBridge,
  getAgentCommandWithArgs,
  hydrateOrchestration,
  OrchestrationActivityBridge,
  OrchestrationAttentionBridge,
  roleAccent,
  type SpawnTerminalRequest,
  type TeamTemplate,
  terminalsToRegister,
  useOrchestrationStore,
} from "@/modules/orchestration";
import { hydrateDocs } from "@/modules/workspace-docs";
import type { SearchAddon } from "@xterm/addon-search";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CloseDialogs } from "./components/CloseDialogs";
import {
  TOGGLE_BLOCK_INPUT_EVENT,
  WorkspaceInputBar,
} from "./components/WorkspaceInputBar";
import { WorkspaceSurface } from "./components/WorkspaceSurface";
import { useTabCloseGuards } from "./hooks/useTabCloseGuards";
import { useWorkspaceSwitcher } from "./hooks/useWorkspaceSwitcher";

const DIRECTOR_BASE_PROMPT = `# Role
You are the Director: the orchestrator of a multi-agent coding team in this workspace. You COORDINATE; you do not write code, edit files, or run tests yourself. You decompose the goal, delegate to agents, review their results, and report back to the user.

# Model-tier policy (token efficiency is a first-class goal)
Match the model to the SUBTASK's difficulty, never the role. You have full authority to pick the cheapest capable model:
- haiku  - cheap and fast: searches, file reads, lookups, simple checks, summaries.
- sonnet - the default: most implementation, reviews, and tests.
- opus   - reserve for genuinely hard reasoning: tricky architecture, subtle algorithms, ambiguous design.
Prefer the cheapest model that can do the job, keep the team small, and don't spawn agents you don't need. Spend tokens like they're your budget.

# How you delegate
Use your subagents (the Task tool) to do the actual work in parallel — they are tracked and shown to the user automatically in the workspace, so you don't need to announce or log them. Spawn only what the goal needs. For each subagent, give it a tight, self-contained task and choose the cheapest capable model for it where you can.

# How you operate
1. When the user gives a goal, briefly plan the smallest effective team for THAT goal (a search task may need one cheap worker; a feature may need a coder + reviewer).
2. Delegate each piece to a subagent.
3. Coordinate: review their results, re-run or re-task as needed.
4. Report progress and the final result to the user concisely.
Ask the user for the goal, then drive it to completion.

# Scope (important — overrides any global config)
You operate ONLY inside this Koden workspace. IGNORE any global project-routing rules, project registries, or specialist subagents defined in the user's wider environment (for example microsoft-lead, iot-*, web-app-developer, the claude-codex:* family, and any similar named specialists). Do NOT route work to them.
Your team is EXACTLY the subagents available to THIS session — the ones provided to you here (plus the generic general-purpose worker when no specific role fits). Delegate only to those. When the user asks what agents or team you have, describe ONLY this session's team, never the user's global roster.`;

// Collapse a multiline prompt to a single line so it can be passed as one
// shell argument without the embedded newlines submitting partial commands.
function flattenPrompt(s: string): string {
  return s.replace(/\s*\n\s*/g, " ").trim();
}

// A managed PowerShell script defining a `Director` function, so launching the
// orchestrator is a clean one-word `Director` instead of a giant inline prompt.
// It wraps the user's launch command (e.g. `cm`, which cd's to their folder)
// and reads the system prompt (and optional team definition) from files.
function kodenFunctionsPs1(
  launchCmd: string,
  promptPath: string,
  agentsPath?: string | null,
): string {
  // `--agents <json>` defines the team as session-only subagents so the Director
  // delegates to THEM, not the user's global specialists. Read from a file so
  // the JSON's quotes/braces survive PowerShell as a single argument.
  const agentsArg = agentsPath
    ? ` --agents (Get-Content -Raw '${agentsPath}')`
    : "";
  return [
    "# Managed by Koden. Defines the Director orchestrator launcher.",
    "function Director {",
    `  ${launchCmd} --model opus --append-system-prompt (Get-Content -Raw '${promptPath}')${agentsArg} @args`,
    "}",
    "",
  ].join("\n");
}

// Stable subagent-type slug for a roster member (matches the live subagent back
// to its planned roster node). e.g. "QA" -> "qa", "Best Coder" -> "best-coder".
function memberSlug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

const MODEL_BY_ROLE: Record<string, string> = {
  director: "opus",
  architect: "opus",
  coder: "sonnet",
  reviewer: "sonnet",
  auditor: "haiku",
  qa: "sonnet",
  devops: "sonnet",
  worker: "sonnet",
};

// Builds the `--agents` JSON: each template member becomes a real session-only
// subagent keyed by its slug, so `subagent_type: "architect"` resolves to OUR
// agent rather than a global specialist of a similar name.
function teamAgentsJson(template: TeamTemplate): string {
  const agents: Record<
    string,
    { description: string; prompt: string; model: string }
  > = {};
  for (const m of template.members) {
    agents[memberSlug(m.name)] = {
      description: m.task,
      prompt: `You are the ${m.name} (${m.role}) on a focused team coordinated by the Director inside the Koden workspace. Your responsibility: ${m.task} Do exactly the task you are handed, stay strictly in scope, and return a concise result to the Director. Do not delegate to other agents.`,
      model: MODEL_BY_ROLE[m.role] ?? "sonnet",
    };
  }
  return JSON.stringify(agents);
}

// System prompt for a spawned worker so it reports into the same bus (status +
// messages), making its work and coordination visible and director-routable.
function agentSystemPrompt(
  name: string,
  role: string,
  busPath: string | null,
): string {
  const base = `# You are "${name}", a ${role} agent
You are part of a team coordinated by the Director. Do the task in the user prompt, then stop. Stay within your task; don't expand scope.`;
  if (!busPath) return base;
  return `${base}

# Reporting (so the Director and user can see you)
Append ONE JSON object per line to the team bus with your Bash tool: echo '<json>' >> ${quoteShellArg(busPath)}
- When you start:    {"cmd":"status","agent":"${name}","status":"working"}
- To report up or to a teammate: {"cmd":"message","from":"${name}","to":"Director","text":"<short update>"}
- If blocked:        {"cmd":"status","agent":"${name}","status":"blocked"}  plus a message explaining why
- When finished:     {"cmd":"message","from":"${name}","to":"Director","text":"<result summary>"} then {"cmd":"status","agent":"${name}","status":"done"}
Keep messages short.`;
}

// Appended to the Director's system prompt when launched from a team template,
// so it starts already knowing its standing roster and delegates accordingly.
function teamRosterPrompt(template: TeamTemplate): string {
  const roster = template.members
    .map((m) => `- ${m.name} (${m.role}): ${m.task}`)
    .join("\n");
  return `

# Your standing team: ${template.name}
You have a pre-assigned crew. When the user gives you a goal, delegate the work across these roles using your Task subagents (matching each to the cheapest capable model):
${roster}
You don't have to use every role for every goal — pick the ones the task needs.`;
}

export default function App() {
  const {
    tabs,
    activeId,
    setActiveId,
    allocId,
    replaceTabs,
    moveTabToSpace,
    reorderTab,
    newTabInSpace,
    removeTabsForSpace,
    markBooted,
    setActiveSpaceForNewTabs,
    newTab,
    newBlockTab,
    newAgentTab,
    newGridTab,
    newPrivateTab,
    openFileTab,
    pinTab,
    newPreviewTab,
    newMarkdownTab,
    newNotesTab,
    newBoardTab,
    newTasksTab,
    openOrchestrationTab,
    setMarkdownView,
    openAiDiffTab,
    closeAiDiffTab,
    openGitDiffTab,
    openCommitHistoryTab,
    openCommitFileDiffTab,
    closeTab,
    duplicateTab,
    closeOthersInSpace,
    updateTab,
    selectByIndex,
    setLeafCwd,
    focusPane,
    focusNextPaneInTab,
    splitActivePane,
    movePane,
    addNotePane,
    addTasksPane,
    closeActivePane,
    closePaneByLeaf,
    resetWorkspace,
  } = useTabs(
    getDefaultFolder() || getLaunchDir()
      ? { cwd: getDefaultFolder() || getLaunchDir() }
      : undefined,
  );

  // Mirror `tabs` into a ref so callbacks scheduled with `setTimeout`
  // (e.g. cdInNewTab) read the latest pane state instead of a stale closure.
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;

  const activeTerminalTab = useMemo(() => {
    const t = tabs.find((x) => x.id === activeId);
    return t && t.kind === "terminal" ? t : null;
  }, [tabs, activeId]);
  const activeLeafId = activeTerminalTab?.activeLeafId ?? null;

  const searchAddons = useRef<Map<number, SearchAddon>>(new Map());
  const [activeSearchAddon, setActiveSearchAddon] =
    useState<SearchAddon | null>(null);
  const searchInlineRef = useRef<SearchInlineHandle | null>(null);
  const terminalRefs = useRef<Map<number, TerminalPaneHandle>>(new Map());
  const editorRefs = useRef<Map<number, EditorPaneHandle>>(new Map());
  const previewRefs = useRef<Map<number, PreviewPaneHandle>>(new Map());
  const [activeEditorHandle, setActiveEditorHandle] =
    useState<EditorPaneHandle | null>(null);
  const [gitHistoryHandle, setGitHistoryHandle] =
    useState<GitHistorySearchHandle | null>(null);
  const { zoomIn, zoomOut, zoomReset } = useZoom();
  useTerminalFileDrop();
  const explorerRef = useRef<FileExplorerHandle>(null);

  // Drives session disposal off the pane tree, not React lifecycles —
  // split/unsplit re-mount components but the leaf is still live.
  const liveLeavesRef = useRef<Set<number>>(new Set());

  const clearWorkspaceState = useCallback(() => {
    for (const id of liveLeavesRef.current) disposeSession(id);
    searchAddons.current.clear();
    terminalRefs.current.clear();
    editorRefs.current.clear();
    previewRefs.current.clear();
    setActiveSearchAddon(null);
    setActiveEditorHandle(null);
  }, []);

  const workspaceEnv = useWorkspaceEnvStore((s) => s.env);
  const setWorkspaceEnv = useWorkspaceEnvStore((s) => s.setEnv);
  const { home, launchCwd, launchCwdResolved, switchWorkspace } =
    useWorkspaceSwitcher({
      tabsRef,
      workspaceEnv,
      setWorkspaceEnv,
      resetWorkspace,
      clearWorkspaceState,
    });

  // Absolute path of the Director command bus, home-rooted so it's stable
  // regardless of where `cm` cd's a session. Used by both the Director and the
  // workers it spawns (so they report status/messages to the same bus).
  const busPath = home ? `${home}/.koden/director-bus.jsonl` : null;
  // Per-pane agent status bus: every Koden claude/codex session appends
  // working/attention/finished here tagged with its pty id. Watched always (not
  // only while a Director runs) so any terminal's status reaches the dock.
  const agentBusPath = home ? `${home}/.koden/agent-bus.jsonl` : null;

  // Ensures ~/.koden exists and is authorized for Koden fs writes. Order
  // matters: authorize home first (it exists and canonicalizes), then create
  // the dir under it, then authorize the dir itself. Returns the dir or null.
  const ensureKodenDir = useCallback(async (): Promise<string | null> => {
    if (!home) return null;
    const dir = `${home}/.koden`;
    try {
      await native.workspaceAuthorize(home);
      await native.createDir(dir).catch(() => {});
      await native.workspaceAuthorize(dir).catch(() => {});
      return dir;
    } catch {
      return null;
    }
  }, [home]);

  const activeSpaceId = useSpaces((s) => s.activeId);
  const spacesHydrated = useSpaces((s) => s.hydrated);
  // Reactive spaces list so the tab "Move to space" submenu re-renders as
  // spaces are added/renamed/removed (getState() snapshots wouldn't).
  const spaces = useSpaces((s) => s.spaces);

  useSpacesBoot({
    ready: launchCwdResolved,
    launchCwd,
    home,
    allocId,
    replaceTabs,
    markBooted,
    setActiveSpaceForNewTabs,
  });

  useSpacePersistence({
    tabs,
    activeId,
    activeSpaceId: activeSpaceId ?? DEFAULT_SPACE_ID,
    enabled: spacesHydrated,
  });

  const prevSpaceRef = useRef(activeSpaceId);
  useEffect(() => {
    if (!spacesHydrated || !activeSpaceId) return;
    setActiveSpaceForNewTabs(activeSpaceId);
    const prev = prevSpaceRef.current;
    prevSpaceRef.current = activeSpaceId;
    if (prev === null || prev === activeSpaceId) return;
    const inSpace = tabsRef.current.filter((t) => t.spaceId === activeSpaceId);
    if (inSpace.length === 0) return;
    // Keep the active tab if it already belongs to the newly active space (a
    // cross-space jump set it explicitly); else fall to the space's last tab.
    if (inSpace.some((t) => t.id === activeId)) return;
    setActiveId(inSpace[inSpace.length - 1].id);
  }, [
    activeSpaceId,
    activeId,
    spacesHydrated,
    setActiveSpaceForNewTabs,
    setActiveId,
  ]);

  const [switcherOpen, setSwitcherOpen] = useState(false);

  const spaceTabs = useMemo(
    () => tabs.filter((t) => t.spaceId === (activeSpaceId ?? DEFAULT_SPACE_ID)),
    [tabs, activeSpaceId],
  );

  const { layoutMode, toggleLayoutMode } = useLayoutMode();

  const {
    sidebarRef,
    sidebarWidthRef,
    sidebarView,
    persistSidebarView,
    toggleSidebar,
    cycleSidebarView,
    persistSidebarWidth,
    toggleExplorerFocus,
  } = useSidebarPanel(explorerRef, layoutMode);

  // Mirror the primary sidebar's collapsed state so the always-visible vertical
  // rail (sidebar mode) can drop its active highlight when the column is closed.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  const [newEditorOpen, setNewEditorOpen] = useState(false);
  const [newGridOpen, setNewGridOpen] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [paletteInitialMode, setPaletteInitialMode] = useState<
    "commands" | "content"
  >("commands");
  const openCommandPalette = useCallback(
    (mode: "commands" | "content" = "commands") => {
      setPaletteInitialMode(mode);
      setCommandPaletteOpen(true);
    },
    [],
  );
  const miniOpen = useChatStore((s) => s.mini.open);
  const miniPresence = usePresence(miniOpen, 200);
  const openMini = useChatStore((s) => s.openMini);
  const focusInput = useChatStore((s) => s.focusInput);
  const openPanel = useChatStore((s) => s.openPanel);
  const panelOpen = useChatStore((s) => s.panelOpen);
  const setLive = useChatStore((s) => s.setLive);
  const respondToApproval = useChatStore((s) => s.respondToApproval);

  const linkOrchestrationTerminal = useOrchestrationStore((s) => s.linkTerminal);
  const setOrchestrationStatus = useOrchestrationStore((s) => s.setStatus);
  const agentCount = useOrchestrationStore((s) => Object.keys(s.agents).length);

  useEffect(() => {
    void hydrateOrchestration();
    void hydrateDocs().then(({ recovered }) => {
      if (recovered) {
        toast.warning("Recovered notes from backup", {
          description:
            "The workspace docs file was damaged (likely a power loss). Restored from the most recent backup.",
        });
      }
    });
  }, []);

  // Start each session with a fresh agent status bus so the reader never
  // replays a previous run's (now dead-pty) events.
  useEffect(() => {
    if (agentBusPath) void native.writeFile(agentBusPath, "").catch(() => {});
  }, [agentBusPath]);

  // Drive per-tab status pills from terminal agent signals (working / waiting
  // for approval / done / exited). Maps the signal's pty to its tab.
  useEffect(() => {
    const un = listen<{ id: number; kind: string }>(
      "koden:agent-signal",
      (e) => {
        const leafId = leafIdForPty(e.payload.id);
        if (leafId === null) return;
        const tab = tabsRef.current.find(
          (t): t is TerminalTab =>
            t.kind === "terminal" && hasLeaf(t.paneTree, leafId),
        );
        if (!tab) return;
        // Surface ANY terminal running a coding agent in the Agents panel (not
        // just the Director): register a node the first time this leaf goes
        // active. Status thereafter is driven by OrchestrationActivityBridge.
        if (e.payload.kind === "started" || e.payload.kind === "working") {
          const orch = useOrchestrationStore.getState();
          const known = Object.values(orch.agents).some(
            (a) => a.leafId === leafId,
          );
          if (!known) {
            const cwd = findLeafCwd(tab.paneTree, leafId);
            const base = cwd?.split(/[\\/]/).filter(Boolean).pop();
            const id = orch.spawn({
              role: "worker",
              name: base || "Agent",
              task: "Running",
              leafId,
              tabId: tab.id,
            });
            orch.setStatus(id, "working");
          }
        }
        const store = useTabStatusStore.getState();
        switch (e.payload.kind) {
          case "attention":
            store.escalate(tab.id, "waiting");
            break;
          case "started":
          case "working":
            store.escalate(tab.id, "working");
            break;
          case "finished":
            store.escalate(tab.id, "done");
            break;
          case "exited":
            store.clear(tab.id);
            break;
        }
      },
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Clear a tab's status pill once you switch to it (you've seen it).
  useEffect(() => {
    useTabStatusStore.getState().clear(activeId);
  }, [activeId]);

  const { hasComposer, keysLoaded } = useAiBootstrap();

  const activeTab = tabs.find((t) => t.id === activeId);
  const isTerminalTab = activeTab?.kind === "terminal";
  const isBlockTab = activeTerminalTab?.blocks === true;
  const isEditorTab = activeTab?.kind === "editor";
  const isGitHistoryTab = activeTab?.kind === "git-history";

  useEditorFileSync({ tabs, tabsRef, editorRefs });
  useThemeFileEditing({ tabsRef, openFileTab });

  // A configured default folder wins over the launch dir / OS home as the base
  // the explorer roots at and new terminals open in (Settings → General).
  const defaultFolder = usePreferencesStore((s) => s.defaultFolder);
  // Default accent colors for newly created note/task panes (Settings → General).
  const paneColorNotes = usePreferencesStore((s) => s.paneColorNotes);
  const paneColorTask = usePreferencesStore((s) => s.paneColorTask);
  const paneColorMode = usePreferencesStore((s) => s.paneColorMode);
  const paneColorPalette = usePreferencesStore((s) => s.paneColorPalette);
  const prefsHydrated = usePreferencesStore((s) => s.hydrated);
  const usageGuardEnabled = usePreferencesStore((s) => s.usageGuardEnabled);
  // Switching auto-color palette/mode recolors EXISTING panes too, not just
  // future spawns. Skip the initial mount/restore so saved per-pane colors
  // aren't clobbered on boot; only a genuine user change recolors. setPaneColor
  // no-ops on locked (Director/agent) panes, so those keep their role accent.
  const autoColorPrev = useRef<{
    mode: typeof paneColorMode;
    palette: typeof paneColorPalette;
  } | null>(null);
  useEffect(() => {
    // Wait for prefs to hydrate from disk: the store starts at DEFAULT values
    // and flips to stored values in one async set. Capturing the baseline only
    // AFTER hydration means the default->stored transition (and the
    // spaces-restore boot race) can't clobber restored per-pane colors.
    if (!prefsHydrated) return;
    const prev = autoColorPrev.current;
    autoColorPrev.current = { mode: paneColorMode, palette: paneColorPalette };
    if (prev === null) return;
    if (prev.mode === paneColorMode && prev.palette === paneColorPalette) return;
    if (paneColorMode !== "automatic") return;
    const setPaneColor = usePaneTitleStore.getState().setPaneColor;
    for (const t of tabsRef.current) {
      if (t.kind !== "terminal") continue;
      for (const id of leafIds(t.paneTree)) {
        setPaneColor(id, nextPaneColor(paneColorPalette));
      }
    }
  }, [prefsHydrated, paneColorMode, paneColorPalette]);

  // Auto-color brand-new tabs. The split handlers assign a generated color at
  // split time, but a fresh tab's initial terminal leaf is created deeper (in
  // newTab / newTabInSpace / the + button) without one, so it fell back to the
  // plain per-type default. This fills in any terminal leaf that has NO color
  // yet (and isn't locked) whenever automatic mode is on, covering new tabs and
  // restores uniformly without touching already-colored panes. It writes to
  // usePaneTitleStore (read via getState, not subscribed), not to tabs, so it
  // can't loop. Splits, notes/tasks panes, and the Director already carry a
  // color (or are locked), so the no-color/locked guard skips them.
  useEffect(() => {
    if (paneColorMode !== "automatic") return;
    const { titles, setPaneColor } = usePaneTitleStore.getState();
    for (const t of tabsRef.current) {
      if (t.kind !== "terminal") continue;
      for (const id of leafIds(t.paneTree)) {
        const entry = titles[id];
        if (entry?.color || entry?.locked) continue;
        setPaneColor(id, nextPaneColor(paneColorPalette));
      }
    }
  }, [tabs, paneColorMode, paneColorPalette]);
  const { explorerRoot, inheritedCwdForNewTab } = useWorkspaceCwd(
    activeTab,
    tabs,
    defaultFolder.trim() || launchCwd || home,
  );

  useWindowTitle(activeTab, explorerRoot);

  useEffect(() => {
    setActiveSearchAddon(
      activeLeafId !== null
        ? (searchAddons.current.get(activeLeafId) ?? null)
        : null,
    );
    setActiveEditorHandle(editorRefs.current.get(activeId) ?? null);
  }, [activeId, activeLeafId]);

  const handleSearchReady = useCallback(
    (leafId: number, addon: SearchAddon) => {
      searchAddons.current.set(leafId, addon);
      if (leafId === activeLeafId) setActiveSearchAddon(addon);
    },
    [activeLeafId],
  );

  const disposeTab = useCallback(
    (id: number) => {
      // Terminal-leaf-keyed maps (terminalRefs/searchAddons) are pruned by
      // the effect below as the pane tree changes; only the tab-id-keyed
      // handles need explicit cleanup here.
      editorRefs.current.delete(id);
      previewRefs.current.delete(id);
      closeTab(id);
    },
    [closeTab],
  );

  const {
    pendingCloseTab,
    pendingTerminalCloseTab,
    pendingDeleteTabs,
    handleClose,
    confirmClose,
    cancelClose,
    confirmTerminalClose,
    cancelTerminalClose,
    confirmDeleteClose,
    cancelDeleteClose,
    handlePathDeleted,
  } = useTabCloseGuards({ tabs, disposeTab });

  useEffect(() => {
    const live = new Set<number>();
    for (const t of tabs) {
      if (t.kind === "terminal") {
        for (const id of leafIds(t.paneTree)) live.add(id);
      }
    }
    for (const id of liveLeavesRef.current) {
      if (!live.has(id)) {
        disposeSession(id);
        // A terminal closed: drop the agent it backed. Closing the DIRECTOR
        // also tears down its team (roster + native subagents have no terminals
        // of their own and would otherwise linger as orphans) — but leave other
        // standalone agent terminals untouched.
        const store = useOrchestrationStore.getState();
        const agent = Object.values(store.agents).find((a) => a.leafId === id);
        // Removing a terminal also tears down the native subagents it spawned
        // (they have no terminal of their own and would otherwise orphan) — the
        // Director and any plain terminal are handled the same way now.
        if (agent) store.removeWithChildren(agent.id);
        else store.removeByLeaf(id);
        usePaneTitleStore.getState().clearPaneTitle(id);
      }
    }
    // Surface every running terminal in the Agents panel the moment it opens —
    // not only once it emits an agent signal. claude only emits its first OSC 777
    // when you submit a prompt, so a freshly-opened session used to be invisible
    // (the root of "the agent list is always empty unless the Director runs"). A
    // plain shell now shows as an idle node named by its cwd; a coding agent's
    // OSC 777 upgrades its status live via OrchestrationActivityBridge, and the
    // teardown above removes it when the pane closes. The `owned` check skips
    // leaves already claimed by an agent (e.g. the Director), so there is no
    // double-registration; cold (restored, not-yet-opened) tabs are excluded.
    {
      const orch = useOrchestrationStore.getState();
      const owned = new Set<number>();
      for (const a of Object.values(orch.agents)) {
        if (a.leafId !== null) owned.add(a.leafId);
      }
      for (const seed of terminalsToRegister(tabs, owned)) {
        const agentId = orch.spawn({
          role: "worker",
          name: seed.name,
          task: null,
          leafId: seed.leafId,
          tabId: seed.tabId,
          // A terminal session's model is unknown to Koden (it's whatever the
          // user ran — `cm`/opus, plain claude, codex, a shell). Don't claim a
          // role-default model the dock would show as a wrong "sonnet".
          config: { model: "" },
        });
        orch.setStatus(agentId, "idle");
      }
    }
    liveLeavesRef.current = live;
    for (const k of [...terminalRefs.current.keys()])
      if (!live.has(k)) terminalRefs.current.delete(k);
    for (const k of [...searchAddons.current.keys()])
      if (!live.has(k)) searchAddons.current.delete(k);
  }, [tabs]);

  const cycleTab = useCallback(
    (delta: 1 | -1) => {
      const scoped = tabsRef.current.filter(
        (t) => t.spaceId === (activeSpaceId ?? DEFAULT_SPACE_ID),
      );
      if (scoped.length < 2) return;
      const idx = scoped.findIndex((t) => t.id === activeId);
      const nextIdx = (idx + delta + scoped.length) % scoped.length;
      setActiveId(scoped[nextIdx].id);
    },
    [activeId, activeSpaceId, setActiveId],
  );

  const cycleSpace = useCallback((delta: 1 | -1) => {
    const { spaces, activeId: sid, setActive } = useSpaces.getState();
    if (spaces.length < 2) return;
    const idx = spaces.findIndex((s) => s.id === sid);
    const next = (idx + delta + spaces.length) % spaces.length;
    setActive(spaces[next].id);
  }, []);

  const captureActiveSelection = useCallback((): string | null => {
    const t = tabs.find((x) => x.id === activeId);
    if (!t) return null;
    if (t.kind === "terminal") {
      const lid = t.activeLeafId;
      return terminalRefs.current.get(lid)?.getSelection() ?? null;
    }
    if (t.kind === "editor") {
      return editorRefs.current.get(activeId)?.getSelection() ?? null;
    }
    return null;
  }, [tabs, activeId]);

  const togglePanelAndFocus = useCallback(() => {
    if (!hasComposer) {
      void openSettingsWindow("models");
      return;
    }
    if (panelOpen) {
      useChatStore.getState().closePanel();
    } else {
      openPanel();
      focusInput(null);
    }
  }, [hasComposer, panelOpen, openPanel, focusInput]);

  const attachSelection = useChatStore((s) => s.attachSelection);

  const handleAttachFileToAgent = useCallback(
    (path: string) => {
      if (!hasComposer) {
        void openSettingsWindow("models");
        return;
      }
      // Dispatch a window event the composer listens for. Same pattern as
      // selections — keeps file-explorer decoupled from the AI module.
      window.dispatchEvent(
        new CustomEvent<string>("koden:ai-attach-file", { detail: path }),
      );
      openPanel();
      focusInput(null);
    },
    [hasComposer, openPanel, focusInput],
  );

  const askFromSelection = useCallback(() => {
    if (!hasComposer) {
      void openSettingsWindow("models");
      return;
    }
    const selection = captureActiveSelection();
    if (!selection || !selection.trim()) {
      focusInput(null);
      return;
    }
    const source: "terminal" | "editor" =
      activeTab?.kind === "editor" ? "editor" : "terminal";
    attachSelection(selection, source);
  }, [
    hasComposer,
    captureActiveSelection,
    focusInput,
    attachSelection,
    activeTab,
  ]);

  const openNewTab = useCallback(() => {
    newTab(inheritedCwdForNewTab());
  }, [newTab, inheritedCwdForNewTab]);

  const openNewPrivateTab = useCallback(() => {
    newPrivateTab(inheritedCwdForNewTab());
  }, [newPrivateTab, inheritedCwdForNewTab]);

  const openNewBlockTab = useCallback(() => {
    newBlockTab(inheritedCwdForNewTab());
  }, [newBlockTab, inheritedCwdForNewTab]);

  // Builds the grid tab, then once each pane's PTY is up (whenSessionReady gates
  // the no-op-if-not-ready submit), types the launch command into every pane in
  // parallel so the whole swarm starts together.
  const handleCreateGrid = useCallback(
    (rows: number, cols: number, launchCmd: string) => {
      const { leafIds: gridLeafIds } = newGridTab(
        rows,
        cols,
        inheritedCwdForNewTab(),
      );
      const cmd = launchCmd.trim();
      if (cmd) {
        void Promise.all(
          gridLeafIds.map(async (id) => {
            await whenSessionReady(id);
            submitToLeaf(id, cmd);
          }),
        );
      }
      setNewGridOpen(false);
    },
    [newGridTab, inheritedCwdForNewTab],
  );

  const openDirector = useCallback(
    () => openOrchestrationTab("director"),
    [openOrchestrationTab],
  );

  // Director "run in terminal": open an agent terminal tab and link the
  // orchestration record to it so the dock/topology can activate it.
  const handleSpawnTerminalAgent = useCallback(
    (req: SpawnTerminalRequest) => {
      // Usage-guard soft gate: near the 5-hour wall, don't start new agents
      // (the proactive pause's actual consumer). Manual/user actions are
      // unaffected; this only blocks fresh subagent spawns while paused.
      if (usageGuardEnabled && useUsageStore.getState().pauseActive) {
        toast.warning(
          "Usage guard: paused near the 5-hour limit — not starting a new agent.",
        );
        return;
      }
      const name =
        useOrchestrationStore.getState().agents[req.agentId]?.name ?? req.role;
      const { tabId, leafId } = newAgentTab(inheritedCwdForNewTab(), name);
      linkOrchestrationTerminal(req.agentId, { leafId, tabId });
      usePaneTitleStore
        .getState()
        .setPaneTitle(leafId, name, false, roleAccent(req.role));
      setOrchestrationStatus(req.agentId, "working");
      // Launch Claude Code in the agent's terminal: model via `--model`, a
      // worker system prompt (identity + bus-reporting protocol) so the agent
      // reports its status/messages back into the shared bus, and the task as
      // the initial prompt.
      const task = req.task.trim();
      const model = req.model.trim();
      const workerPrompt = agentSystemPrompt(name, req.role, busPath);
      void (async () => {
        let promptArg = quoteShellArg(flattenPrompt(workerPrompt));
        const dir = await ensureKodenDir();
        if (dir && busPath) {
          try {
            const promptPath = `${dir}/agent-${req.agentId}.txt`;
            await native.writeFile(promptPath, workerPrompt);
            promptArg = `"$(Get-Content -Raw ${quoteShellArg(promptPath)})"`;
          } catch {
            promptArg = quoteShellArg(flattenPrompt(workerPrompt));
          }
        }
        const parts = [getAgentCommandWithArgs()];
        if (model) parts.push("--model", model);
        parts.push("--append-system-prompt", promptArg);
        if (task) parts.push(quoteShellArg(task));
        await whenSessionReady(leafId);
        submitToLeaf(leafId, parts.join(" "));
      })();
    },
    [
      newAgentTab,
      inheritedCwdForNewTab,
      linkOrchestrationTerminal,
      setOrchestrationStatus,
      busPath,
      ensureKodenDir,
    ],
  );


  const sendCd = useCallback(
    (path: string) => {
      if (activeLeafId === null) return;
      const term = terminalRefs.current.get(activeLeafId);
      if (!term) return;
      term.write(`cd ${quoteShellArg(path)}\r`);
      term.focus();
    },
    [activeLeafId, usageGuardEnabled],
  );

  const cdInNewTab = useCallback(
    (path: string) => {
      const tabId = newTab(path);
      setTimeout(() => {
        const tab = tabsRef.current.find((x) => x.id === tabId);
        if (!tab || tab.kind !== "terminal") return;
        const t = terminalRefs.current.get(tab.activeLeafId);
        if (!t) return;
        t.write(`cd ${quoteShellArg(path)}\r`);
        t.focus();
      }, 80);
    },
    [newTab],
  );

  const handleOpenFile = useCallback(
    (path: string, pin?: boolean) => {
      // Markdown opens in its rendered view by default; a per-tab toggle flips
      // it to the raw editor. Other files default to preview (pin=false);
      // explicit actions like context-menu "Open" pass pin=true to persist.
      if (isMarkdownPath(path)) newMarkdownTab(path);
      else openFileTab(path, pin ?? false);
    },
    [openFileTab, newMarkdownTab],
  );

  const handlePathRenamed = useCallback(
    (from: string, to: string) => {
      for (const t of tabs) {
        if (t.kind !== "editor") continue;
        if (t.path === from) {
          const i = to.lastIndexOf("/");
          updateTab(t.id, { path: to, title: i === -1 ? to : to.slice(i + 1) });
        } else if (t.path.startsWith(`${from}/`)) {
          const suffix = t.path.slice(from.length);
          const newPath = `${to}${suffix}`;
          const i = newPath.lastIndexOf("/");
          updateTab(t.id, {
            path: newPath,
            title: i === -1 ? newPath : newPath.slice(i + 1),
          });
        }
      }
    },
    [tabs, updateTab],
  );

  const activeTerminalLeafCwd =
    activeTab?.kind === "terminal"
      ? (findLeafCwd(activeTab.paneTree, activeTab.activeLeafId) ??
        activeTab.cwd ??
        null)
      : null;

  const activeFilePath = (() => {
    if (activeTab?.kind === "editor") return activeTab.path;
    if (activeTab?.kind === "git-diff") {
      if (/^([A-Za-z]:|\/|\\)/.test(activeTab.path)) return activeTab.path;
      const root = activeTab.repoRoot.replace(/[\\/]+$/, "");
      const rel = activeTab.path.replace(/^[\\/]+/, "");
      return `${root}/${rel}`;
    }
    if (activeTab?.kind === "git-commit-file") {
      const root = activeTab.repoRoot.replace(/[\\/]+$/, "");
      const rel = activeTab.path.replace(/^[\\/]+/, "");
      return `${root}/${rel}`;
    }
    return null;
  })();
  const explorerActiveFilePath =
    activeTab?.kind === "editor" || activeTab?.kind === "markdown"
      ? activeTab.path
      : null;
  const { sourceControl, toggleSourceControl, openGitGraphFromContext } =
    useSourceControlContext({
      activeTab,
      tabs,
      activeTerminalLeafCwd,
      explorerRoot,
      launchCwd,
      launchCwdResolved,
      home,
      sidebarView,
      cycleSidebarView,
      openCommitHistoryTab,
    });
  const explorerGitDecorations = usePreferencesStore(
    (s) => s.explorerGitDecorations,
  );

  const openPreviewTab = useCallback(
    (url: string) => {
      const id = newPreviewTab(url);
      // Focus the address bar if the URL is empty so the user can type.
      if (!url) {
        setTimeout(() => previewRefs.current.get(id)?.focusAddressBar(), 0);
      }
      return id;
    },
    [newPreviewTab],
  );


  // Pick a fresh leaf accent. Automatic mode draws a generated color from the
  // chosen palette so each new pane is distinct; manual mode uses the per-type
  // default (terminals get none so they keep their plain status dot).
  const paneColorFor = useCallback(
    (kind: "terminal" | "note" | "tasks"): string | undefined => {
      if (paneColorMode === "automatic") return nextPaneColor(paneColorPalette);
      if (kind === "note") return paneColorNotes;
      if (kind === "tasks") return paneColorTask;
      return undefined;
    },
    [paneColorMode, paneColorPalette, paneColorNotes, paneColorTask],
  );

  // Add a docs-backed note pane beside the active terminal pane, titling it so
  // its header reads "Notes" (renamable) and stands out with the notes accent.
  const addNotePaneToActiveTab = useCallback(
    (dir: "row" | "col" = "row") => {
      const t = tabsRef.current.find((x) => x.id === activeId);
      if (!t || t.kind !== "terminal") return;
      const added = addNotePane(activeId, dir);
      if (added)
        usePaneTitleStore
          .getState()
          .setPaneTitle(added.leafId, "Notes", false, paneColorFor("note"));
    },
    [activeId, addNotePane, paneColorFor],
  );

  // Add a docs-backed tasks pane beside the active terminal pane, titled "Tasks"
  // (renamable) with the checklist accent.
  const addTasksPaneToActiveTab = useCallback(
    (dir: "row" | "col" = "row") => {
      const t = tabsRef.current.find((x) => x.id === activeId);
      if (!t || t.kind !== "terminal") return;
      const added = addTasksPane(activeId, dir);
      if (added)
        usePaneTitleStore
          .getState()
          .setPaneTitle(added.leafId, "Tasks", false, paneColorFor("tasks"));
    },
    [activeId, addTasksPane, paneColorFor],
  );

  const splitActivePaneInActiveTab = useCallback(
    (dir: "row" | "col") => {
      const t = tabsRef.current.find((x) => x.id === activeId);
      if (!t || t.kind !== "terminal") return;
      // Split into the same kind as the focused pane: a note splits into another
      // note, a tasks pane into another tasks pane, a terminal into a terminal.
      const leaf = findLeaf(t.paneTree, t.activeLeafId);
      if (leaf?.content === "note") {
        addNotePaneToActiveTab(dir);
        return;
      }
      if (leaf?.content === "tasks") {
        addTasksPaneToActiveTab(dir);
        return;
      }
      const newLeafId = splitActivePane(activeId, dir);
      const color = paneColorFor("terminal");
      if (newLeafId !== null && color)
        usePaneTitleStore.getState().setPaneTitle(newLeafId, "", false, color);
    },
    [
      activeId,
      splitActivePane,
      addNotePaneToActiveTab,
      addTasksPaneToActiveTab,
      paneColorFor,
    ],
  );

  // Per-pane header dropdown: split the clicked leaf in a 4-way direction into a
  // terminal, note, or tasks pane. The split fns operate on the tab's active
  // leaf, so focus the clicked leaf first to land the split on the right pane.
  const handlePaneSplit = useCallback(
    (leafId: number, type: SplitPaneType, side: SplitSide) => {
      const t = tabsRef.current.find((x) => x.id === activeId);
      if (!t || t.kind !== "terminal") return;
      if (t.activeLeafId !== leafId) focusPane(t.id, leafId);
      const { dir, before } = sideToSplit(side);
      if (type === "note") {
        const added = addNotePane(activeId, dir, before);
        if (added)
          usePaneTitleStore
            .getState()
            .setPaneTitle(added.leafId, "Notes", false, paneColorFor("note"));
      } else if (type === "tasks") {
        const added = addTasksPane(activeId, dir, before);
        if (added)
          usePaneTitleStore
            .getState()
            .setPaneTitle(added.leafId, "Tasks", false, paneColorFor("tasks"));
      } else {
        // In manual mode terminal panes keep their cwd-basename header and plain
        // status dot (no entry). In automatic mode they get an empty-label color
        // entry so the dot/title pick up the generated accent; the empty label
        // still falls back to the cwd basename in PaneHeader.
        const newLeafId = splitActivePane(activeId, dir, before);
        const color = paneColorFor("terminal");
        if (newLeafId !== null && color)
          usePaneTitleStore
            .getState()
            .setPaneTitle(newLeafId, "", false, color);
      }
    },
    [
      activeId,
      focusPane,
      addNotePane,
      addTasksPane,
      splitActivePane,
      paneColorFor,
    ],
  );

  const handleCloseTabOrPane = useCallback(() => {
    const t = tabsRef.current.find((x) => x.id === activeId);
    if (t?.kind === "terminal" && leafIds(t.paneTree).length > 1) {
      closeActivePane(activeId);
      return;
    }
    void handleClose(activeId);
  }, [activeId, closeActivePane, handleClose]);

  const [zenMode, setZenMode] = useState(false);

  const shortcutHandlers = useMemo<ShortcutHandlers>(
    () => ({
      "commandPalette.open": () => openCommandPalette("commands"),
      "commandPalette.content": () => openCommandPalette("content"),
      "tab.new": openNewTab,
      "tab.newBlock": openNewBlockTab,
      "tab.newPrivate": openNewPrivateTab,
      "tab.newPreview": () => openPreviewTab(""),
      "tab.newEditor": () => setNewEditorOpen(true),
      "tab.close": handleCloseTabOrPane,
      "tab.next": () => cycleTab(1),
      "tab.prev": () => cycleTab(-1),
      "tab.selectByIndex": (e) => selectByIndex(parseInt(e.key, 10) - 1),
      "space.next": () => cycleSpace(1),
      "space.prev": () => cycleSpace(-1),
      "space.overview": () => setSwitcherOpen(true),
      "pane.splitRight": () => splitActivePaneInActiveTab("row"),
      "pane.splitDown": () => splitActivePaneInActiveTab("col"),
      "pane.addNote": () => addNotePaneToActiveTab("row"),
      "pane.focusNext": () => focusNextPaneInTab(activeId, 1),
      "pane.focusPrev": () => focusNextPaneInTab(activeId, -1),
      "pane.source": toggleSourceControl,
      "terminal.clear": () => {
        clearFocusedTerminal();
      },
      "terminal.toggleInput": () =>
        window.dispatchEvent(new CustomEvent(TOGGLE_BLOCK_INPUT_EVENT)),
      "blocks.prev": () => navigateFocusedBlocks(-1),
      "blocks.next": () => navigateFocusedBlocks(1),
      "search.focus": () => searchInlineRef.current?.focus(),
      "ai.toggle": togglePanelAndFocus,
      "ai.askSelection": askFromSelection,
      "settings.open": () => void openSettingsWindow(),
      "sidebar.toggle": toggleSidebar,
      "explorer.focus": toggleExplorerFocus,
      "view.zoomIn": zoomIn,
      "view.zoomOut": zoomOut,
      "view.zoomReset": zoomReset,
      "view.zenMode": () => setZenMode((v) => !v),
      "editor.undo": () => editorRefs.current.get(activeId)?.undo(),
      "editor.redo": () => editorRefs.current.get(activeId)?.redo(),
    }),
    [
      activeId,
      openCommandPalette,
      cycleTab,
      cycleSpace,
      handleCloseTabOrPane,
      openNewTab,
      openNewBlockTab,
      openNewPrivateTab,
      openPreviewTab,
      selectByIndex,
      splitActivePaneInActiveTab,
      addNotePaneToActiveTab,
      focusNextPaneInTab,
      toggleSourceControl,
      togglePanelAndFocus,
      askFromSelection,
      toggleSidebar,
      toggleExplorerFocus,
      zoomIn,
      zoomOut,
      zoomReset,
    ],
  );

  const shortcutsDisabled = useCallback(
    (id: ShortcutId, e: KeyboardEvent) => {
      if (id === "editor.undo" || id === "editor.redo") {
        return activeTab?.kind !== "editor";
      }
      if (id === "ai.askSelection") {
        const target =
          (e.target as HTMLElement | null) ?? document.activeElement;
        const inTerminal = !!(target as HTMLElement | null)?.closest?.(
          ".xterm",
        );
        if (!inTerminal) return false;
        const sel = captureActiveSelection();
        return !sel || !sel.trim();
      }
      if (id === "terminal.clear") {
        // Only intercept ⌘K while a terminal is focused; elsewhere let the key
        // fall through (we never preventDefault when disabled).
        const target =
          (e.target as HTMLElement | null) ?? document.activeElement;
        return !(target as HTMLElement | null)?.closest?.(".xterm");
      }
      if (
        id === "terminal.toggleInput" ||
        id === "blocks.prev" ||
        id === "blocks.next"
      ) {
        return !(activeTab?.kind === "terminal" && activeTab.blocks === true);
      }
      if (id === "sidebar.toggle") {
        // Ctrl+B is also Claude Code's "run in background" key. While a terminal
        // is focused, let Ctrl+B reach the shell/Claude instead of toggling the
        // sidebar. Ctrl+Shift+B (second binding) still toggles it from anywhere.
        const target =
          (e.target as HTMLElement | null) ?? document.activeElement;
        const inTerminal = !!(target as HTMLElement | null)?.closest?.(
          ".xterm",
        );
        // Only defer the plain (no-shift) Ctrl/⌘+B binding; the Shift variant
        // is the always-on toggle and is never claimed by the terminal.
        return inTerminal && !e.shiftKey;
      }
      return false;
    },
    [activeTab],
  );

  useGlobalShortcuts(shortcutHandlers, { isDisabled: shortcutsDisabled });

  const registerTerminalHandle = useCallback(
    (leafId: number, h: TerminalPaneHandle | null) => {
      if (h) terminalRefs.current.set(leafId, h);
      else terminalRefs.current.delete(leafId);
    },
    [],
  );

  const registerEditorHandle = useCallback(
    (id: number, h: EditorPaneHandle | null) => {
      if (h) {
        editorRefs.current.set(id, h);
        const line = pendingGotoLine.current.get(id);
        if (line != null) {
          pendingGotoLine.current.delete(id);
          h.gotoLine(line);
        }
      } else {
        editorRefs.current.delete(id);
      }
      if (id === activeId) setActiveEditorHandle(h);
    },
    [activeId],
  );

  const registerPreviewHandle = useCallback(
    (id: number, h: PreviewPaneHandle | null) => {
      if (h) previewRefs.current.set(id, h);
      else previewRefs.current.delete(id);
    },
    [],
  );

  const handlePreviewUrl = useCallback(
    (id: number, url: string) => updateTab(id, { url }),
    [updateTab],
  );

  const subagentCounterRef = useRef(0);
  const authorizedCwds = useRef(new Set<string>());
  const handleTerminalCwd = useCallback(
    (leafId: number, cwd: string) => {
      setLeafCwd(leafId, cwd);
      if (cwd && !authorizedCwds.current.has(cwd)) {
        authorizedCwds.current.add(cwd);
        native.workspaceAuthorize(cwd).catch(() => {
          authorizedCwds.current.delete(cwd);
        });
      }
    },
    [setLeafCwd],
  );

  const handleFocusLeaf = useCallback(
    (tabId: number, leafId: number) => focusPane(tabId, leafId),
    [focusPane],
  );

  const onActivateAgent = useCallback(
    (tabId: number, leafId: number) => {
      setActiveId(tabId);
      focusPane(tabId, leafId);
    },
    [setActiveId, focusPane],
  );

  const onActivateLocalAgent = useCallback(() => {
    openPanel();
    focusInput(null);
  }, [openPanel, focusInput]);

  // Prepare the bus (authorize + create dir + clear), build the system prompt,
  // and launch Claude Code as the orchestrator in the given leaf.
  const launchDirectorInLeaf = useCallback(
    async (leafId: number, template?: TeamTemplate) => {
      // Install the Claude Code hooks (incl. subagent lifecycle) before the
      // session starts, so the Director's subagents surface in real time.
      await invoke("agent_enable_claude_hooks").catch(() => {});
      const prompt = template
        ? DIRECTOR_BASE_PROMPT + teamRosterPrompt(template)
        : DIRECTOR_BASE_PROMPT;
      // Fallback: inline flattened prompt. When the bus is available we write a
      // `Director` PowerShell function to a managed file and just run `Director`
      // (clean one-word command, screen cleared of the dot-source line).
      let command = `${getAgentCommandWithArgs()} --model opus --append-system-prompt ${quoteShellArg(
        flattenPrompt(prompt),
      )}`;
      const dir = await ensureKodenDir();
      if (dir && busPath) {
        try {
          await native.writeFile(busPath, "");
          const promptPath = `${dir}/director-prompt.txt`;
          const ps1Path = `${dir}/koden.ps1`;
          await native.writeFile(promptPath, prompt);
          // A template defines the team as real session subagents via --agents.
          let agentsPath: string | null = null;
          if (template) {
            agentsPath = `${dir}/director-agents.json`;
            await native.writeFile(agentsPath, teamAgentsJson(template));
          }
          await native.writeFile(
            ps1Path,
            kodenFunctionsPs1(getAgentCommandWithArgs(), promptPath, agentsPath),
          );
          command = `. ${quoteShellArg(ps1Path)}; Clear-Host; Director`;
        } catch {
          command = `${getAgentCommandWithArgs()} --model opus --append-system-prompt ${quoteShellArg(
            flattenPrompt(prompt),
          )}`;
        }
      }
      await whenSessionReady(leafId);
      submitToLeaf(leafId, command);
    },
    [busPath, ensureKodenDir],
  );

  // Right-click Director → start (or open) its live command terminal. An
  // optional team template pre-loads its roster (as nodes + in the Director's
  // prompt) so the Director starts with a known crew instead of an empty team.
  const startDirectorCommand = useCallback(
    (template?: TeamTemplate) => {
      const store = useOrchestrationStore.getState();
      // Reuse only a LIVE Director (one with a terminal); just focus it.
      const live = Object.values(store.agents).find(
        (a) => a.role === "director" && a.tabId !== null && a.leafId !== null,
      );
      if (live && live.tabId !== null && live.leafId !== null) {
        onActivateAgent(live.tabId, live.leafId);
        return;
      }
      // No live Director: clear any stale Director team (but keep standalone
      // agent terminals) so the team reflects THIS launch.
      const staleDirector = Object.values(store.agents).find(
        (a) => a.role === "director",
      );
      if (staleDirector) store.removeWithChildren(staleDirector.id);
      const directorId = store.spawn({
        role: "director",
        name: "Director",
        task: "Coordinating workspace",
      });
      const { tabId, leafId } = newAgentTab(inheritedCwdForNewTab(), "Director");
      linkOrchestrationTerminal(directorId, { leafId, tabId });
      usePaneTitleStore
        .getState()
        .setPaneTitle(leafId, "Director", true, roleAccent("director"));
      setOrchestrationStatus(directorId, "idle");
      // Show the chosen team's roster immediately as idle "planned crew" nodes.
      // As the Director delegates, each live subagent claims an idle slot (see
      // handleDirectorCommand), so the planned team visibly comes alive.
      if (template) {
        for (const m of template.members) {
          const id = store.spawn({
            role: m.role,
            name: m.name,
            task: m.task,
            parentId: directorId,
          });
          store.setStatus(id, "idle");
        }
      }
      void launchDirectorInLeaf(leafId, template);
    },
    [
      newAgentTab,
      inheritedCwdForNewTab,
      linkOrchestrationTerminal,
      setOrchestrationStatus,
      onActivateAgent,
      launchDirectorInLeaf,
    ],
  );

  // Add the Director as a split pane inside the active terminal tab instead of
  // a new tab. Falls back to a new tab when the active tab can't be split.
  const addDirectorToActiveTab = useCallback(() => {
    const store = useOrchestrationStore.getState();
    // Reuse a live Director if one exists (don't duplicate it into a new pane).
    const live = Object.values(store.agents).find(
      (a) => a.role === "director" && a.tabId !== null && a.leafId !== null,
    );
    if (live && live.tabId !== null && live.leafId !== null) {
      onActivateAgent(live.tabId, live.leafId);
      return;
    }
    const active = tabsRef.current.find((t) => t.id === activeId);
    if (!active || active.kind !== "terminal") {
      startDirectorCommand();
      return;
    }
    const newLeafId = splitActivePane(activeId, "row");
    if (newLeafId === null) {
      startDirectorCommand();
      return;
    }
    // Fresh start: clear any stale Director team (keep standalone agents).
    const staleDirector = Object.values(store.agents).find(
      (a) => a.role === "director",
    );
    if (staleDirector) store.removeWithChildren(staleDirector.id);
    const directorId = store.spawn({
      role: "director",
      name: "Director",
      task: "Coordinating workspace",
    });
    linkOrchestrationTerminal(directorId, { leafId: newLeafId, tabId: activeId });
    usePaneTitleStore
      .getState()
      .setPaneTitle(newLeafId, "Director", true, roleAccent("director"));
    setOrchestrationStatus(directorId, "idle");
    void launchDirectorInLeaf(newLeafId);
  }, [
    activeId,
    splitActivePane,
    linkOrchestrationTerminal,
    setOrchestrationStatus,
    startDirectorCommand,
    launchDirectorInLeaf,
    onActivateAgent,
  ]);

  // Materialize a Director bus command into visible orchestration state.
  const handleDirectorCommand = useCallback(
    (cmd: DirectorCommand) => {
      const store = useOrchestrationStore.getState();
      const directorId =
        Object.values(store.agents).find((a) => a.role === "director")?.id ??
        null;
      const byName = (name?: string) =>
        name
          ? (Object.values(store.agents).find((a) => a.name === name)?.id ??
            null)
          : null;

      // Any tool/subagent activity means the Director is actively orchestrating
      // — keep it "working" even across tool-answer resumes that don't re-fire
      // UserPromptSubmit. Its own Stop hook returns it to idle when the turn ends.
      if (cmd.cmd === "director-active") {
        if (directorId) store.setStatus(directorId, "working");
        return;
      }
      if (directorId && (cmd.cmd === "subagent-start" || cmd.cmd === "subagent-stop")) {
        store.setStatus(directorId, "working");
      }

      if (cmd.cmd === "subagent-start") {
        // First, try to claim an idle template roster slot so the planned team
        // visibly activates as the Director delegates. Otherwise spawn a fresh
        // node for this native subagent (it has no Koden terminal of its own).
        const idleSlots = Object.values(store.agents)
          .filter(
            (a) =>
              a.parentId === directorId &&
              a.role !== "director" &&
              a.leafId === null &&
              a.status === "idle",
          )
          .sort((a, b) => a.createdAt - b.createdAt);
        // Prefer the slot whose slug matches the subagent_type (exact role),
        // else fall back to the oldest idle slot.
        const slot =
          (cmd.agentType
            ? idleSlots.find((a) => memberSlug(a.name) === cmd.agentType)
            : undefined) ?? idleSlots[0];
        if (slot) {
          if (cmd.name?.trim()) store.setTask(slot.id, cmd.name.trim());
          store.setStatus(slot.id, "working");
          return;
        }
        subagentCounterRef.current += 1;
        const id = store.spawn({
          role: "worker",
          name: cmd.name?.trim() || `Subagent ${subagentCounterRef.current}`,
          task: cmd.name?.trim() || "Working",
          parentId: directorId,
          config: { model: "sonnet" },
          native: true,
        });
        store.setStatus(id, "working");
        return;
      }
      if (cmd.cmd === "subagent-stop") {
        // SubagentStop carries no identity, so retire the oldest still-running
        // terminal-less child of the Director (claimed slot or native subagent;
        // they complete roughly FIFO).
        const oldest = Object.values(store.agents)
          .filter(
            (a) =>
              a.parentId === directorId &&
              a.role !== "director" &&
              a.leafId === null &&
              a.status === "working",
          )
          .sort((a, b) => a.createdAt - b.createdAt)[0];
        if (oldest) store.setStatus(oldest.id, "done");
        return;
      }

      if (cmd.cmd === "spawn") {
        const role = (AGENT_ROLES as readonly string[]).includes(cmd.role ?? "")
          ? (cmd.role as AgentRole)
          : "worker";
        const model =
          cmd.model && ["opus", "sonnet", "haiku"].includes(cmd.model)
            ? cmd.model
            : "sonnet";
        const id = store.spawn({
          role,
          name: cmd.name,
          task: cmd.task,
          parentId: directorId,
          config: { model },
        });
        handleSpawnTerminalAgent({ agentId: id, role, task: cmd.task, model });
      } else if (cmd.cmd === "message") {
        const fromId = byName(cmd.from) ?? directorId;
        if (!fromId) return;
        const kinds = [
          "delegation",
          "handoff",
          "decision",
          "review",
          "audit",
          "approval",
        ];
        store.logFlow({
          kind:
            cmd.kind && kinds.includes(cmd.kind)
              ? (cmd.kind as "delegation")
              : "message",
          fromId,
          toId: cmd.to && cmd.to !== "all" ? byName(cmd.to) : null,
          summary: cmd.text,
        });
      } else if (cmd.cmd === "status") {
        const id = byName(cmd.agent);
        const valid = [
          "spawning",
          "idle",
          "working",
          "reviewing",
          "waiting",
          "blocked",
          "done",
          "error",
        ];
        if (id && valid.includes(cmd.status))
          store.setStatus(id, cmd.status as Agent["status"]);
      }
    },
    [handleSpawnTerminalAgent],
  );

  const directorLive = useOrchestrationStore((s) =>
    Object.values(s.agents).some(
      (a) => a.role === "director" && a.leafId !== null,
    ),
  );

  const clearRoster = useCallback(() => {
    useOrchestrationStore.getState().reset();
  }, []);

  const launchAgentTerminal = useCallback(
    (agent: Agent) => {
      if (agent.tabId !== null && agent.leafId !== null) {
        onActivateAgent(agent.tabId, agent.leafId);
        return;
      }
      handleSpawnTerminalAgent({
        agentId: agent.id,
        role: agent.role,
        task: agent.task ?? "",
        model: agent.config.model,
      });
    },
    [onActivateAgent, handleSpawnTerminalAgent],
  );

  const removeAgent = useCallback((id: string) => {
    useOrchestrationStore.getState().remove(id);
  }, []);

  const handleLeafExit = useCallback(
    (leafId: number, _code: number) => {
      const all = tabsRef.current;
      const tab = all.find(
        (t) => t.kind === "terminal" && hasLeaf(t.paneTree, leafId),
      );
      if (!tab || tab.kind !== "terminal") return;
      const isLast =
        leafIds(tab.paneTree).length === 1 &&
        all.filter((t) => t.kind === "terminal").length === 1;
      if (isLast) {
        void respawnSession(leafId, tab.cwd);
      } else {
        closePaneByLeaf(leafId);
      }
    },
    [closePaneByLeaf],
  );

  const handleEditorDirty = useCallback(
    (id: number, dirty: boolean) => updateTab(id, { dirty }),
    [updateTab],
  );

  const handleRenameTab = useCallback(
    (id: number, title: string) => updateTab(id, { customTitle: title.trim() }),
    [updateTab],
  );

  const searchTarget = useMemo<SearchTarget>(() => {
    if (isTerminalTab && activeLeafId !== null && activeSearchAddon)
      return {
        kind: "terminal",
        addon: activeSearchAddon,
        focus: () => terminalRefs.current.get(activeLeafId)?.focus(),
      };
    if (isEditorTab && activeEditorHandle)
      return {
        kind: "editor",
        handle: activeEditorHandle,
        focus: () => activeEditorHandle.focus(),
      };
    if (isGitHistoryTab && gitHistoryHandle)
      return {
        kind: "git-history",
        handle: gitHistoryHandle,
        focus: () => {},
      };
    return null;
  }, [
    isTerminalTab,
    isEditorTab,
    isGitHistoryTab,
    activeLeafId,
    activeSearchAddon,
    activeEditorHandle,
    gitHistoryHandle,
  ]);

  const activeCwd = activeTerminalLeafCwd;

  const handleNewSpace = useCallback(() => {
    const { spaces, create, setActive } = useSpaces.getState();
    const meta = create({
      name: `Space ${spaces.length + 1}`,
      root: activeCwd ?? home ?? null,
      env: workspaceEnv,
    });
    setActiveSpaceForNewTabs(meta.id);
    newTab(activeCwd ?? undefined);
    setActive(meta.id);
    return meta.id;
  }, [activeCwd, home, workspaceEnv, newTab, setActiveSpaceForNewTabs]);

  const handleDeleteSpace = useCallback(
    (id: string) => {
      useSpaces.getState().remove(id);
      removeTabsForSpace(id);
    },
    [removeTabsForSpace],
  );

  const handleMoveTab = useCallback(
    (tabId: number, targetSpaceId: string) => {
      if (moveTabToSpace(tabId, targetSpaceId)) {
        useSpaces.getState().setActive(targetSpaceId);
      }
    },
    [moveTabToSpace],
  );

  const handleReorderTab = useCallback(
    (tabId: number, targetTabId: number, edge: "top" | "bottom") => {
      if (reorderTab(tabId, targetTabId, edge)) {
        const target = tabsRef.current.find((x) => x.id === targetTabId);
        if (target) useSpaces.getState().setActive(target.spaceId);
      }
    },
    [reorderTab],
  );

  const handleNewTabInSpace = useCallback(
    (spaceId: string) => {
      const root = useSpaces.getState().spaces.find((s) => s.id === spaceId)
        ?.root;
      newTabInSpace(spaceId, root ?? undefined);
    },
    [newTabInSpace],
  );

  const jumpToTab = useCallback(
    (tabId: number) => {
      const t = tabsRef.current.find((x) => x.id === tabId);
      if (!t) return;
      setActiveId(tabId);
      useSpaces.getState().setActive(t.spaceId);
      setSwitcherOpen(false);
    },
    [setActiveId],
  );

  const spaceSwitcher = (
    <SpaceSwitcher
      open={switcherOpen}
      onOpenChange={setSwitcherOpen}
      tabs={tabs}
      onNewSpace={() => void handleNewSpace()}
      onDeleteSpace={handleDeleteSpace}
      onNewTabInSpace={handleNewTabInSpace}
      onJumpTab={jumpToTab}
      onCloseTab={handleClose}
      onMoveTabToSpace={handleMoveTab}
      onReorderTab={handleReorderTab}
      onReorderSpaces={(ids) => useSpaces.getState().reorder(ids)}
    />
  );

  const commandPaletteItems = useMemo(
    () =>
      commandPaletteOpen
        ? createCommandItems({
            tabs,
            activeId,
            searchTarget,
            explorerRoot,
            home,
            openNewTab,
            openNewBlock: openNewBlockTab,
            openNewPrivate: openNewPrivateTab,
            openNewEditor: () => setNewEditorOpen(true),
            openNewPreview: () => openPreviewTab(""),
            openNewNotes: () => newNotesTab(),
            openNewBoard: () => newBoardTab(),
            openNewTasks: () => newTasksTab(),
            openDirector,
            openAgentTopology: () => openOrchestrationTab("agent-topology"),
            openMessageFlow: () => openOrchestrationTab("message-flow"),
            openGitGraph: openGitGraphFromContext,
            toggleSourceControl,
            closeActiveTabOrPane: handleCloseTabOrPane,
            splitPaneRight: () => splitActivePaneInActiveTab("row"),
            splitPaneDown: () => splitActivePaneInActiveTab("col"),
            addTerminalPane: (dir) => splitActivePaneInActiveTab(dir),
            addNotePane: (dir) => addNotePaneToActiveTab(dir),
            addTasksPane: (dir) => addTasksPaneToActiveTab(dir),
            focusSearch: () => searchInlineRef.current?.focus(),
            focusExplorerSearch: () => explorerRef.current?.focusSearch(),
            toggleSidebar,
            toggleLayout: toggleLayoutMode,
            toggleAi: togglePanelAndFocus,
            askAiSelection: askFromSelection,
            openSettings: () => void openSettingsWindow(),
            openKeyboardShortcuts: () => void openSettingsWindow("shortcuts"),
            spaces: useSpaces.getState().spaces,
            activeSpaceId,
            openSpacesOverview: () => setSwitcherOpen(true),
            newSpace: () => void handleNewSpace(),
            switchSpace: (id) => useSpaces.getState().setActive(id),
          })
        : [],
    [
      commandPaletteOpen,
      tabs,
      activeId,
      searchTarget,
      explorerRoot,
      home,
      openNewTab,
      openNewBlockTab,
      openNewPrivateTab,
      openPreviewTab,
      openGitGraphFromContext,
      toggleSourceControl,
      handleCloseTabOrPane,
      splitActivePaneInActiveTab,
      addNotePaneToActiveTab,
      addTasksPaneToActiveTab,
      toggleSidebar,
      togglePanelAndFocus,
      askFromSelection,
      activeSpaceId,
      handleNewSpace,
      newNotesTab,
      newBoardTab,
      newTasksTab,
      openDirector,
      openOrchestrationTab,
      toggleLayoutMode,
    ],
  );

  // DEV-only: expose the autonomous test-harness control bus
  // (window.__KODEN_TEST__). Vite's import.meta.env.DEV dead-code elimination
  // strips this from production builds. See src/dev/testBus.ts and
  // .memory/test-harness-design-2026-06-20.md.
  useEffect(() => {
    if (!import.meta.env?.DEV || typeof window === "undefined") return;
    installTestBus({
      commandItems: commandPaletteItems,
      shortcutHandlers,
      setPaletteOpen: setCommandPaletteOpen,
      tabs,
      activeId,
      newGridTab,
      reorderTab,
      duplicateTab,
      moveTabToSpace,
      commandOverrides: {
        "theme.pick": () => {
          throw new Error(
            "theme.pick is a palette mode-switch; use __KODEN_TEST__.settings.setThemeId(id)",
          );
        },
        "search.content": () => {
          throw new Error(
            "search.content is a palette mode-switch; drive via the palette DOM input",
          );
        },
        "history.open": () => {
          throw new Error(
            "history.open is a palette mode-switch; drive via the palette DOM input",
          );
        },
      },
    });
  }, [
    commandPaletteItems,
    shortcutHandlers,
    tabs,
    activeId,
    newGridTab,
    reorderTab,
    duplicateTab,
    moveTabToSpace,
  ]);

  const pendingGotoLine = useRef<Map<number, number>>(new Map());
  const openContentHit = useCallback(
    (path: string, line: number) => {
      const id = openFileTab(path, true);
      if (id == null) return;
      const h = editorRefs.current.get(id);
      if (h) h.gotoLine(line);
      else pendingGotoLine.current.set(id, line);
    },
    [openFileTab],
  );

  const insertHistoryCommand = useMemo(
    () =>
      isTerminalTab && activeLeafId !== null
        ? (cmd: string) => {
            writeToSession(activeLeafId, cmd);
            terminalRefs.current.get(activeLeafId)?.focus();
          }
        : null,
    [isTerminalTab, activeLeafId],
  );

  useAiLiveBridge({
    setLive,
    activeId,
    tabs,
    explorerRoot,
    launchCwd,
    home,
    openPreviewTab,
    newAgentTab,
    terminalRefs,
  });

  const shell = (
    <ThemeProvider>
      <TooltipProvider>
        <div className="relative flex h-screen flex-col overflow-hidden bg-background text-foreground">
          {!zenMode && (
            <Header
              tabs={spaceTabs}
              activeId={activeId}
              onSelect={setActiveId}
              onNew={openNewTab}
              onNewGrid={() => setNewGridOpen(true)}
              onNewBlock={openNewBlockTab}
              onNewPrivate={openNewPrivateTab}
              onNewPreview={() => openPreviewTab("")}
              onNewEditor={() => setNewEditorOpen(true)}
              onNewGitGraph={openGitGraphFromContext}
              onNewNotes={() => newNotesTab()}
              onNewBoard={() => newBoardTab()}
              onNewTasks={() => newTasksTab()}
              onOpenDirector={openDirector}
              onClose={handleClose}
              onDuplicate={duplicateTab}
              onCloseOthers={closeOthersInSpace}
              onPin={pinTab}
              onRename={handleRenameTab}
              spaces={spaces}
              onMoveToSpace={handleMoveTab}
              onToggleSidebar={toggleSidebar}
              onOpenCommandPalette={() => openCommandPalette("commands")}
              onActivateAgent={onActivateAgent}
              onActivateLocalAgent={onActivateLocalAgent}
              onOpenSettings={() => void openSettingsWindow()}
              spaceSwitcher={spaceSwitcher}
              searchTarget={searchTarget}
              searchRef={searchInlineRef}
              hideTabs={layoutMode === "sidebar"}
            />
          )}

          <main className="zoom-content flex min-h-0 flex-1 flex-row">
            {/* Sidebar mode: a slim always-visible activity rail (Files |
                Source Control). It lives OUTSIDE the collapsible primary column
                so it stays reachable to re-expand Files/SC after collapsing. */}
            {layoutMode === "sidebar" && (
              <SidebarRail
                orientation="vertical"
                collapsed={sidebarCollapsed}
                activeView={sidebarView}
                onSelectView={cycleSidebarView}
                changedCount={sourceControl.changedCount}
                agentCount={agentCount}
                layoutMode={layoutMode}
              />
            )}
            <ResizablePanelGroup
              orientation="horizontal"
              className="min-h-0 flex-1"
            >
              <ResizablePanel
                id="sidebar"
                panelRef={sidebarRef}
                defaultSize={`${sidebarWidthRef.current}px`}
                minSize={`${SIDEBAR_MIN_WIDTH}px`}
                maxSize={`${SIDEBAR_MAX_WIDTH}px`}
                collapsible
                collapsedSize={0}
                onResize={(size) => {
                  setSidebarCollapsed(size.inPixels <= 0);
                  if (size.inPixels > 0) persistSidebarWidth(size.inPixels);
                }}
              >
                <div className="flex h-full min-h-0 flex-col border-r border-border/60 bg-card">
                  <div key={sidebarView} className="min-h-0 flex-1 koden-panel-in">
                    {sidebarView === "explorer" ? (
                      <FileExplorer
                        ref={explorerRef}
                        rootPath={explorerRoot}
                        gitStatus={
                          explorerGitDecorations ? sourceControl.status : null
                        }
                        activeFilePath={explorerActiveFilePath}
                        onOpenFile={handleOpenFile}
                        onPathRenamed={handlePathRenamed}
                        onPathDeleted={handlePathDeleted}
                        onRevealInTerminal={cdInNewTab}
                        onAttachToAgent={handleAttachFileToAgent}
                      />
                    ) : sidebarView === "agents" ? (
                      <AgentDock
                        tabs={tabs}
                        onActivateAgent={onActivateAgent}
                        onStartDirector={() => startDirectorCommand()}
                        onStartDirectorWithTemplate={startDirectorCommand}
                        onAddDirectorToTab={addDirectorToActiveTab}
                        onLaunchAgent={launchAgentTerminal}
                        onRemoveAgent={removeAgent}
                        onClearRoster={clearRoster}
                      />
                    ) : (
                      <SourceControlPanel
                        open
                        sourceControl={sourceControl}
                        onOpenDiff={openGitDiffTab}
                        onOpenGitGraph={openGitGraphFromContext}
                        onOpenFile={handleOpenFile}
                      />
                    )}
                  </div>
                  {layoutMode === "top" && (
                    <SidebarRail
                      activeView={sidebarView}
                      onSelectView={persistSidebarView}
                      changedCount={sourceControl.changedCount}
                      agentCount={agentCount}
                      layoutMode={layoutMode}
                    />
                  )}
                </div>
              </ResizablePanel>
              <ResizableHandle withHandle />
              {layoutMode === "sidebar" && (
                <>
                  <ResizablePanel
                    id="vertical-tabs"
                    defaultSize="220px"
                    minSize="160px"
                    maxSize="380px"
                  >
                    <ResizablePanelGroup
                      orientation="vertical"
                      className="h-full"
                    >
                      <ResizablePanel
                        id="vtabs-list"
                        defaultSize="55%"
                        minSize="20%"
                      >
                        <VerticalTabs
                          tabs={spaceTabs}
                          activeId={activeId}
                          onSelect={setActiveId}
                          onClose={handleClose}
                          onDuplicate={duplicateTab}
                          onCloseOthers={closeOthersInSpace}
                          onRename={handleRenameTab}
                          onNew={openNewTab}
                          onNewGrid={() => setNewGridOpen(true)}
                          onNewNotes={() => newNotesTab()}
                          onNewBoard={() => newBoardTab()}
                          onNewTasks={() => newTasksTab()}
                          onNewEditor={() => setNewEditorOpen(true)}
                          onNewPreview={() => openPreviewTab("")}
                          onOpenDirector={openDirector}
                          spaces={spaces}
                          onMoveToSpace={handleMoveTab}
                        />
                      </ResizablePanel>
                      <ResizableHandle withHandle />
                      <ResizablePanel
                        id="vtabs-agents"
                        defaultSize="45%"
                        minSize="15%"
                        collapsible
                        collapsedSize={0}
                      >
                        <div className="h-full min-h-0 border-r border-t border-border/60 bg-card">
                          <AgentDock
                            tabs={tabs}
                            onActivateAgent={onActivateAgent}
                            onStartDirector={() => startDirectorCommand()}
                            onStartDirectorWithTemplate={startDirectorCommand}
                            onAddDirectorToTab={addDirectorToActiveTab}
                            onLaunchAgent={launchAgentTerminal}
                            onRemoveAgent={removeAgent}
                            onClearRoster={clearRoster}
                          />
                        </div>
                      </ResizablePanel>
                    </ResizablePanelGroup>
                  </ResizablePanel>
                  <ResizableHandle withHandle />
                </>
              )}
              <ResizablePanel id="workspace" defaultSize="78%" minSize="30%">
                <div className="flex h-full min-h-0 flex-col">
                  <div className="relative min-h-0 flex-1">
                    <WorkspaceSurface
                      tabs={tabs}
                      activeId={activeId}
                      activeTab={activeTab}
                      registerTerminalHandle={registerTerminalHandle}
                      onSearchReady={handleSearchReady}
                      onCwd={handleTerminalCwd}
                      onExit={handleLeafExit}
                      onFocusLeaf={handleFocusLeaf}
                      onClosePane={closePaneByLeaf}
                      onSplit={handlePaneSplit}
                      onMovePane={(source, target, side) =>
                        movePane(activeId, source, target, side)
                      }
                      registerEditorHandle={registerEditorHandle}
                      onEditorDirtyChange={handleEditorDirty}
                      onEditorCloseTab={disposeTab}
                      registerPreviewHandle={registerPreviewHandle}
                      onPreviewUrlChange={handlePreviewUrl}
                      onAiDiffAccept={(id) => respondToApproval(id, true)}
                      onAiDiffReject={(id) => respondToApproval(id, false)}
                      onOpenCommitFile={openCommitFileDiffTab}
                      onGitHistorySearchHandle={setGitHistoryHandle}
                      onSetMarkdownView={setMarkdownView}
                      onActivateAgent={onActivateAgent}
                      onSpawnTerminalAgent={handleSpawnTerminalAgent}
                    />
                  </div>

                  <WorkspaceInputBar
                    isBlockTab={isBlockTab}
                    isTerminalTab={isTerminalTab}
                    activeLeafId={activeLeafId}
                    cwd={activeCwd}
                    home={home}
                    hasComposer={hasComposer}
                    panelOpen={panelOpen}
                    keysLoaded={keysLoaded}
                    onConnect={() => void openSettingsWindow("models")}
                  />
                </div>
              </ResizablePanel>
            </ResizablePanelGroup>
          </main>

          {!zenMode && (
            <StatusBar
              cwd={activeCwd}
              filePath={activeFilePath}
              home={home}
              onCd={sendCd}
              onWorkspaceChange={switchWorkspace}
              onOpenMini={openMini}
              hasComposer={hasComposer}
              privateActive={
                activeTab?.kind === "terminal" && activeTab.private === true
              }
            />
          )}

          <AgentNotificationsBridge
            tabs={tabs}
            activeId={activeId}
            onActivate={onActivateAgent}
          />
          <RetryBridge />
          <UsageBridge />
          <OrchestrationActivityBridge />
          <OrchestrationAttentionBridge />
          <AgentBusBridge busPath={agentBusPath} />
          <DirectorBusBridge
            busPath={directorLive ? busPath : null}
            onCommand={handleDirectorCommand}
          />
          <Toaster position="bottom-right" />

          {hasComposer ? (
            <>
              <AgentRunBridge
                openAiDiffTab={openAiDiffTab}
                closeAiDiffTab={closeAiDiffTab}
              />
              <LocalAgentNotificationsBridge />
            </>
          ) : null}

          {hasComposer && miniPresence.mounted ? (
            <AiMiniWindow state={miniPresence.state} />
          ) : null}

          <CommandPalette
            open={commandPaletteOpen}
            onOpenChange={setCommandPaletteOpen}
            initialMode={paletteInitialMode}
            commandItems={commandPaletteItems}
            workspaceRoot={explorerRoot}
            onOpenContentHit={openContentHit}
            insertCommand={insertHistoryCommand}
          />

          <NewEditorDialog
            open={newEditorOpen}
            onOpenChange={setNewEditorOpen}
            rootPath={explorerRoot ?? home}
            onCreated={(path) => openFileTab(path)}
          />

          <GridDialog
            open={newGridOpen}
            onOpenChange={setNewGridOpen}
            onConfirm={handleCreateGrid}
          />

          <UpdaterDialog />

          <CloseDialogs
            tabs={tabs}
            pendingCloseTab={pendingCloseTab}
            onCancelClose={cancelClose}
            onConfirmClose={confirmClose}
            pendingTerminalCloseTab={pendingTerminalCloseTab}
            onCancelTerminalClose={cancelTerminalClose}
            onConfirmTerminalClose={confirmTerminalClose}
            pendingDeleteTabs={pendingDeleteTabs}
            onCancelDeleteClose={cancelDeleteClose}
            onConfirmDeleteClose={confirmDeleteClose}
          />
        </div>
      </TooltipProvider>
    </ThemeProvider>
  );

  return <AiComposerProvider>{shell}</AiComposerProvider>;
}
