import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import {
  Cancel01Icon,
  ComputerTerminal02Icon,
  Settings02Icon,
  Tick02Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useMemo, useState } from "react";
import type { PlannedAgent } from "@/modules/ai/lib/teamPlanner";
import { getAgentCommand, setAgentCommand } from "../lib/agentCommand";
import { MODEL_ALIASES, defaultConfigForRole, roleBlurb } from "../lib/roles";
import { ROLE_META, STATUS_META, formatTokens } from "../lib/roleMeta";
import { TEAM_TEMPLATES, type TeamTemplate } from "../lib/templates";
import { countActive, sortAgentsForDock, totalTokens } from "../lib/topology";
import {
  AGENT_ROLES,
  AGENT_STATUSES,
  type Agent,
  type AgentRole,
} from "../lib/types";
import { useOrchestrationStore } from "../store/orchestrationStore";

export type SpawnTerminalRequest = {
  agentId: string;
  role: AgentRole;
  task: string;
  model: string;
};

type Props = {
  /** Spawn a terminal coding-agent bound to the orchestration agent. */
  onSpawnTerminal?: (req: SpawnTerminalRequest) => void;
  /** Activate an agent's linked terminal tab. */
  onActivateAgent?: (tabId: number, leafId: number) => void;
};

export function DirectorView({ onSpawnTerminal, onActivateAgent }: Props) {
  const agents = useOrchestrationStore((s) => s.agents);
  const spawn = useOrchestrationStore((s) => s.spawn);

  const list = useMemo(() => sortAgentsForDock(Object.values(agents)), [agents]);
  const director = list.find((a) => a.role === "director") ?? null;

  // Every Director workspace has a single root director agent.
  useEffect(() => {
    if (Object.values(agents).some((a) => a.role === "director")) return;
    spawn({ role: "director", name: "Director", task: "Coordinating workspace" });
  }, [agents, spawn]);

  const applyTemplate = (template: TeamTemplate) => {
    const directorId =
      director?.id ??
      spawn({ role: "director", name: "Director", task: "Coordinating workspace" });
    for (const m of template.members) {
      spawn({
        role: m.role,
        name: m.name,
        task: m.task,
        parentId: directorId,
      });
    }
  };

  const tokens = totalTokens(list);

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-y-auto p-1">
      <header className="flex flex-wrap items-center gap-3 rounded-lg border border-border/60 bg-card/40 px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-md bg-primary/15 text-primary">
            <HugeiconsIcon icon={ROLE_META.director.icon} size={16} strokeWidth={2} />
          </span>
          <div>
            <div className="text-sm font-semibold text-foreground">Director</div>
            <div className="text-[11px] text-muted-foreground">
              Spawn, assign, route, review and approve
            </div>
          </div>
        </div>
        <div className="ml-auto flex items-center gap-4 text-xs text-muted-foreground">
          <Stat label="Agents" value={String(list.length)} />
          <Stat label="Active" value={String(countActive(list))} />
          <Stat label="Tokens" value={formatTokens(tokens.input + tokens.output)} />
        </div>
      </header>

      <AgentCommandField />

      <GoalPlanner onSpawnTerminal={onSpawnTerminal} />

      {list.length > 0 ? (
        <div className="flex flex-col gap-2">
          <div className="px-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            Team
          </div>
          {list.map((agent) => (
            <AgentCard
              key={agent.id}
              agent={agent}
              onSpawnTerminal={onSpawnTerminal}
              onActivateAgent={onActivateAgent}
            />
          ))}
        </div>
      ) : null}

      <Collapsible>
        <CollapsibleTrigger className="flex w-full items-center gap-1.5 rounded-md px-1 py-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground">
          Manual spawn & templates
        </CollapsibleTrigger>
        <CollapsibleContent className="mt-2 flex flex-col gap-3">
          <div className="rounded-lg border border-border/60 bg-card/40 p-3">
            <div className="mb-2 text-xs font-semibold text-foreground">
              Team templates
            </div>
            <div className="flex flex-col gap-2">
              {TEAM_TEMPLATES.map((t) => (
                <div key={t.id} className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-medium text-foreground">
                      {t.name}
                    </div>
                    <div className="text-[11px] text-muted-foreground">
                      {t.description}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    variant="secondary"
                    className="h-7 shrink-0 text-xs"
                    onClick={() => applyTemplate(t)}
                  >
                    Apply
                  </Button>
                </div>
              ))}
            </div>
          </div>

          <SpawnForm
            directorId={director?.id ?? null}
            onSpawn={(req) => {
              const id = spawn({
                role: req.role,
                name: req.name || undefined,
                task: req.task || null,
                parentId: director?.id ?? null,
                config: { model: req.model },
              });
              if (req.runInTerminal && onSpawnTerminal) {
                onSpawnTerminal({
                  agentId: id,
                  role: req.role,
                  task: req.task,
                  model: req.model,
                });
              }
            }}
          />
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}

type PlanRow = PlannedAgent & { key: string };

function GoalPlanner({
  onSpawnTerminal,
}: {
  onSpawnTerminal?: (req: SpawnTerminalRequest) => void;
}) {
  const spawn = useOrchestrationStore((s) => s.spawn);
  const [goal, setGoal] = useState("");
  const [planning, setPlanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reasoning, setReasoning] = useState<string | null>(null);
  const [rows, setRows] = useState<PlanRow[]>([]);
  const [autoLaunch, setAutoLaunch] = useState(true);

  const runPlan = async () => {
    const g = goal.trim();
    if (!g || planning) return;
    setPlanning(true);
    setError(null);
    try {
      const { planTeam } = await import("@/modules/ai/lib/teamPlanner");
      const plan = await planTeam(g);
      setReasoning(plan.reasoning);
      setRows(plan.agents.map((a) => ({ ...a, key: crypto.randomUUID() })));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Planning failed.");
      setReasoning(null);
      setRows([]);
    } finally {
      setPlanning(false);
    }
  };

  const ensureDirectorId = (): string => {
    const existing = Object.values(useOrchestrationStore.getState().agents).find(
      (a) => a.role === "director",
    );
    return (
      existing?.id ??
      spawn({ role: "director", name: "Director", task: "Coordinating workspace" })
    );
  };

  const approve = () => {
    if (rows.length === 0) return;
    const directorId = ensureDirectorId();
    for (const a of rows) {
      const id = spawn({
        role: a.role as AgentRole,
        name: a.name,
        task: a.task,
        parentId: directorId,
        config: { model: a.model },
      });
      if (autoLaunch && onSpawnTerminal) {
        onSpawnTerminal({
          agentId: id,
          role: a.role as AgentRole,
          task: a.task,
          model: a.model,
        });
      }
    }
    setRows([]);
    setReasoning(null);
    setGoal("");
  };

  const hasPlan = rows.length > 0 || reasoning !== null;

  return (
    <div className="rounded-lg border border-border/60 bg-card/40 p-3">
      <div className="mb-2 text-xs font-semibold text-foreground">
        Delegate a goal
      </div>
      <Textarea
        value={goal}
        onChange={(e) => setGoal(e.target.value)}
        placeholder="Describe what the team should accomplish. The Director picks the roles and models; you approve before anything spawns."
        rows={2}
        className="resize-none text-xs"
      />
      <div className="mt-2 flex items-center justify-between gap-2">
        <span className="text-[11px] text-muted-foreground">
          The Director plans, you approve.
        </span>
        <Button
          size="sm"
          className="h-7 text-xs"
          disabled={!goal.trim() || planning}
          onClick={() => void runPlan()}
        >
          {planning ? "Planning…" : "Plan team"}
        </Button>
      </div>

      {error ? (
        <p className="mt-2 text-[11px] text-destructive">{error}</p>
      ) : null}

      {hasPlan ? (
        <div className="mt-3 border-t border-border/50 pt-3">
          {reasoning ? (
            <p className="mb-2 text-[11px] text-muted-foreground">{reasoning}</p>
          ) : null}
          <div className="flex flex-col gap-1.5">
            {rows.map((a, i) => {
              const role = ROLE_META[a.role];
              return (
                <div key={a.key} className="flex items-center gap-2">
                  <span
                    className="flex size-5 shrink-0 items-center justify-center rounded"
                    style={{ background: `${role.accent}22`, color: role.accent }}
                  >
                    <HugeiconsIcon icon={role.icon} size={12} strokeWidth={2} />
                  </span>
                  <span className="w-20 shrink-0 truncate text-xs font-medium text-foreground">
                    {a.name}
                  </span>
                  <AliasSelect
                    value={a.model}
                    onChange={(m) =>
                      setRows((prev) =>
                        prev.map((r, j) => (j === i ? { ...r, model: m } : r)),
                      )
                    }
                  />
                  <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                    {a.task}
                  </span>
                  <button
                    type="button"
                    aria-label="Remove from plan"
                    className="shrink-0 rounded p-1 text-muted-foreground hover:text-destructive"
                    onClick={() =>
                      setRows((prev) => prev.filter((_, j) => j !== i))
                    }
                  >
                    <HugeiconsIcon icon={Cancel01Icon} size={12} strokeWidth={2} />
                  </button>
                </div>
              );
            })}
          </div>
          <div className="mt-3 flex items-center justify-between gap-2">
            <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted-foreground">
              <input
                type="checkbox"
                checked={autoLaunch}
                onChange={(e) => setAutoLaunch(e.target.checked)}
                className="size-3.5 accent-[var(--primary)]"
              />
              Launch each as a Claude Code terminal
            </label>
            <div className="flex gap-1.5">
              <Button
                size="sm"
                variant="ghost"
                className="h-7 text-xs"
                onClick={() => {
                  setRows([]);
                  setReasoning(null);
                }}
              >
                Discard
              </Button>
              <Button
                size="sm"
                className="h-7 text-xs"
                disabled={rows.length === 0}
                onClick={approve}
              >
                Approve & spawn {rows.length}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function AgentCommandField() {
  const [cmd, setCmd] = useState(() => getAgentCommand());
  return (
    <div className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/40 px-3 py-2">
      <Label className="text-[11px] text-muted-foreground">Launch command</Label>
      <Input
        value={cmd}
        onChange={(e) => {
          setCmd(e.target.value);
          setAgentCommand(e.target.value);
        }}
        placeholder="claude"
        spellCheck={false}
        className="h-7 max-w-40 font-mono text-xs"
      />
      <span className="text-[10.5px] text-muted-foreground">
        used to start agents (model + prompt flags are appended)
      </span>
    </div>
  );
}

function AliasSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="h-6 w-20 shrink-0 text-[11px]">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {MODEL_ALIASES.map((m) => (
          <SelectItem key={m} value={m} className="text-xs capitalize">
            {m}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="text-center">
      <div className="text-sm font-semibold tabular-nums text-foreground">{value}</div>
      <div className="text-[10px] uppercase tracking-wide">{label}</div>
    </div>
  );
}

type SpawnDraft = {
  role: AgentRole;
  name: string;
  task: string;
  model: string;
  runInTerminal: boolean;
};

function SpawnForm({
  directorId,
  onSpawn,
}: {
  directorId: string | null;
  onSpawn: (req: SpawnDraft) => void;
}) {
  const [role, setRole] = useState<AgentRole>("coder");
  const [name, setName] = useState("");
  const [task, setTask] = useState("");
  const [model, setModel] = useState(defaultConfigForRole("coder").model);
  const [runInTerminal, setRunInTerminal] = useState(false);

  const onRoleChange = (r: AgentRole) => {
    setRole(r);
    setModel(defaultConfigForRole(r).model);
  };

  return (
    <div className="rounded-lg border border-border/60 bg-card/40 p-3">
      <div className="mb-2 text-xs font-semibold text-foreground">Spawn agent</div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <Label className="text-[11px]">Role</Label>
          <Select value={role} onValueChange={(v) => onRoleChange(v as AgentRole)}>
            <SelectTrigger className="h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AGENT_ROLES.filter((r) => r !== "director").map((r) => (
                <SelectItem key={r} value={r} className="text-xs capitalize">
                  {r}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-1">
          <Label className="text-[11px]">Name (optional)</Label>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={role.charAt(0).toUpperCase() + role.slice(1)}
            className="h-8 text-xs"
          />
        </div>
        <div className="flex flex-col gap-1 sm:col-span-2">
          <Label className="text-[11px]">Model</Label>
          <Input
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="h-8 font-mono text-xs"
          />
        </div>
        <div className="flex flex-col gap-1 sm:col-span-2">
          <Label className="text-[11px]">Task</Label>
          <Textarea
            value={task}
            onChange={(e) => setTask(e.target.value)}
            placeholder={roleBlurb(role)}
            rows={2}
            className="resize-none text-xs"
          />
        </div>
      </div>
      <div className="mt-2 flex items-center justify-between gap-2">
        <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            checked={runInTerminal}
            onChange={(e) => setRunInTerminal(e.target.checked)}
            className="size-3.5 accent-[var(--primary)]"
          />
          Run in a terminal tab
        </label>
        <Button
          size="sm"
          className="h-7 text-xs"
          disabled={directorId === null}
          onClick={() => {
            onSpawn({ role, name, task, model, runInTerminal });
            setName("");
            setTask("");
          }}
        >
          Spawn
        </Button>
      </div>
    </div>
  );
}

function AgentCard({
  agent,
  onSpawnTerminal,
  onActivateAgent,
}: {
  agent: Agent;
  onSpawnTerminal?: (req: SpawnTerminalRequest) => void;
  onActivateAgent?: (tabId: number, leafId: number) => void;
}) {
  const setStatus = useOrchestrationStore((s) => s.setStatus);
  const remove = useOrchestrationStore((s) => s.remove);
  const assign = useOrchestrationStore((s) => s.assign);
  const logFlow = useOrchestrationStore((s) => s.logFlow);
  const directorId = useOrchestrationStore((s) =>
    Object.values(s.agents).find((a) => a.role === "director")?.id ?? null,
  );
  const [taskDraft, setTaskDraft] = useState("");
  const role = ROLE_META[agent.role];
  const status = STATUS_META[agent.status];
  const isDirector = agent.role === "director";
  const isLinked = agent.tabId !== null && agent.leafId !== null;

  return (
    <div className="rounded-lg border border-border/60 bg-card/40 p-3">
      <div className="flex items-center gap-2">
        <span
          className="flex size-6 shrink-0 items-center justify-center rounded-md"
          style={{ background: `${role.accent}22`, color: role.accent }}
        >
          <HugeiconsIcon icon={role.icon} size={13} strokeWidth={2} />
        </span>
        <span className="truncate text-sm font-semibold text-foreground">
          {agent.name}
        </span>
        <Badge variant="secondary" className="h-5 px-1.5 text-[10px] capitalize">
          {agent.role}
        </Badge>
        <span
          className={cn("ml-auto size-2 shrink-0 rounded-full", status.pulse && "koden-pulse")}
          style={{ background: status.dot }}
          title={status.label}
        />
        <span className="text-[11px] text-muted-foreground">{status.label}</span>
      </div>

      <div className="mt-1.5 truncate text-xs text-muted-foreground">
        {agent.task ?? "No task assigned"}
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-muted-foreground">
        <span className="font-mono">{agent.config.model}</span>
        <span>in {formatTokens(agent.tokens.input)}</span>
        <span>out {formatTokens(agent.tokens.output)}</span>
        {agent.config.limits.costLimit !== null ? (
          <span>cap ${agent.config.limits.costLimit}</span>
        ) : null}
      </div>

      {!isDirector ? (
        <div className="mt-2 flex items-center gap-1.5">
          <Input
            value={taskDraft}
            onChange={(e) => setTaskDraft(e.target.value)}
            placeholder="Assign / route a task..."
            className="h-7 text-xs"
            onKeyDown={(e) => {
              if (e.key === "Enter" && taskDraft.trim() && directorId) {
                assign(directorId, agent.id, taskDraft.trim());
                setTaskDraft("");
              }
            }}
          />
          {isLinked ? (
            <Button
              size="sm"
              variant="secondary"
              className="h-7 shrink-0 text-xs"
              disabled={!onActivateAgent}
              onClick={() => {
                if (onActivateAgent && agent.tabId !== null && agent.leafId !== null)
                  onActivateAgent(agent.tabId, agent.leafId);
              }}
              title="Open agent terminal"
            >
              <HugeiconsIcon
                icon={ComputerTerminal02Icon}
                size={13}
                strokeWidth={2}
              />
              Open
            </Button>
          ) : (
            <Button
              size="sm"
              variant="secondary"
              className="h-7 shrink-0 text-xs"
              disabled={!onSpawnTerminal}
              onClick={() =>
                onSpawnTerminal?.({
                  agentId: agent.id,
                  role: agent.role,
                  task: agent.task ?? "",
                  model: agent.config.model,
                })
              }
              title="Launch as a Claude Code terminal"
            >
              <HugeiconsIcon
                icon={ComputerTerminal02Icon}
                size={13}
                strokeWidth={2}
              />
              Launch
            </Button>
          )}
          <Button
            size="sm"
            variant="secondary"
            className="h-7 shrink-0 text-xs"
            disabled={!directorId || agent.status !== "reviewing"}
            onClick={() => {
              if (directorId)
                logFlow({
                  kind: "approval",
                  fromId: directorId,
                  toId: agent.id,
                  summary: `Approved ${agent.name}'s work`,
                });
              setStatus(agent.id, "done");
            }}
            title="Approve and mark done"
          >
            <HugeiconsIcon icon={Tick02Icon} size={13} strokeWidth={2} />
            Approve
          </Button>
          <Select
            value={agent.status}
            onValueChange={(v) =>
              setStatus(agent.id, v as Agent["status"])
            }
          >
            <SelectTrigger className="h-7 w-28 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AGENT_STATUSES.map((s) => (
                <SelectItem key={s} value={s} className="text-xs capitalize">
                  {s}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            size="icon"
            variant="ghost"
            className="size-7 shrink-0 text-muted-foreground hover:text-destructive"
            onClick={() => remove(agent.id)}
            title="Remove agent"
          >
            <HugeiconsIcon icon={Cancel01Icon} size={13} strokeWidth={2} />
          </Button>
        </div>
      ) : null}

      <AgentConfigEditor agent={agent} />
    </div>
  );
}

function AgentConfigEditor({ agent }: { agent: Agent }) {
  const updateConfig = useOrchestrationStore((s) => s.updateConfig);
  const c = agent.config;
  return (
    <Collapsible className="mt-2">
      <CollapsibleTrigger className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground">
        <HugeiconsIcon icon={Settings02Icon} size={12} strokeWidth={2} />
        Configuration
      </CollapsibleTrigger>
      <CollapsibleContent className="mt-2 grid grid-cols-2 gap-2">
        <LabeledNumber
          label="Context limit"
          value={c.limits.contextLimit}
          onChange={(n) =>
            updateConfig(agent.id, {
              limits: { ...c.limits, contextLimit: n },
            })
          }
        />
        <LabeledNumber
          label="Cost limit ($)"
          value={c.limits.costLimit}
          onChange={(n) =>
            updateConfig(agent.id, { limits: { ...c.limits, costLimit: n } })
          }
        />
        <div className="col-span-2 flex flex-col gap-1">
          <Label className="text-[10px]">Permissions</Label>
          <div className="flex flex-wrap gap-1">
            {c.permissions.length ? (
              c.permissions.map((p) => (
                <Badge key={p} variant="outline" className="h-5 px-1.5 text-[9px]">
                  {p}
                </Badge>
              ))
            ) : (
              <span className="text-[10px] text-muted-foreground">none</span>
            )}
          </div>
        </div>
        <div className="col-span-2 flex flex-col gap-1">
          <Label className="text-[10px]">Tools</Label>
          <div className="flex flex-wrap gap-1">
            {c.tools.map((t) => (
              <Badge key={t} variant="outline" className="h-5 px-1.5 text-[9px] font-mono">
                {t}
              </Badge>
            ))}
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

function LabeledNumber({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number | null;
  onChange: (n: number | null) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Label className="text-[10px]">{label}</Label>
      <Input
        type="number"
        value={value ?? ""}
        placeholder="unlimited"
        className="h-7 text-xs"
        onChange={(e) => {
          const v = e.target.value.trim();
          onChange(v === "" ? null : Number(v));
        }}
      />
    </div>
  );
}
