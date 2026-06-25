import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { AGENT_ICONS } from "@/modules/ai/components/AgentSwitcher";
import {
  getProvider,
  MODEL_PRICING,
  PROVIDERS,
  type ProviderId,
  providerSupportsKey,
} from "@/modules/ai/config";
import {
  type Agent,
  type AgentIconId,
  BUILTIN_AGENTS,
} from "@/modules/ai/lib/agents";
import { getAllKeys, setKey } from "@/modules/ai/lib/keyring";
import {
  isValidHandle,
  normalizeHandle,
  type Snippet,
} from "@/modules/ai/lib/snippets";
import { newAgentId, useAgentsStore } from "@/modules/ai/store/agentsStore";
import {
  newSnippetId,
  useSnippetsStore,
} from "@/modules/ai/store/snippetsStore";
import {
  brainBudgetStatus,
  brainLibrarianActivity,
  brainLibrarianStatus,
  brainSetBudget,
  brainSetLibrarian,
  type LibrarianActivity,
} from "@/modules/brain/lib/bindings";
import {
  cheapestLibModel,
  isLocalLibProvider,
  LOCAL_LIB_PROVIDERS,
  libCloudModels,
  libLocalBaseUrl,
  libRates,
} from "@/modules/brain/lib/librarian";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setCustomInstructions } from "@/modules/settings/store";
import {
  Add01Icon,
  CheckmarkCircle02Icon,
  Delete02Icon,
  Edit02Icon,
  SparklesIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";

const ICON_OPTIONS: AgentIconId[] = [
  "coder",
  "architect",
  "reviewer",
  "security",
  "designer",
  "spark",
];

export function AgentsSection() {
  const customInstructions = usePreferencesStore((s) => s.customInstructions);
  const customAgents = useAgentsStore((s) => s.customAgents);
  const activeAgentId = useAgentsStore((s) => s.activeId);
  const setActiveAgentId = useAgentsStore((s) => s.setActiveId);
  const upsertAgent = useAgentsStore((s) => s.upsert);
  const removeAgent = useAgentsStore((s) => s.remove);
  const hydrateAgents = useAgentsStore((s) => s.hydrate);

  const snippets = useSnippetsStore((s) => s.snippets);
  const upsertSnippet = useSnippetsStore((s) => s.upsert);
  const removeSnippet = useSnippetsStore((s) => s.remove);
  const hydrateSnippets = useSnippetsStore((s) => s.hydrate);

  useEffect(() => {
    void hydrateAgents();
    void hydrateSnippets();
  }, [hydrateAgents, hydrateSnippets]);

  const [editingAgent, setEditingAgent] = useState<Agent | null>(null);
  const [editingSnippet, setEditingSnippet] = useState<Snippet | null>(null);

  return (
    <div className="flex flex-col gap-7">
      <SectionHeader
        title="Koden AI"
        description="The model the Brain Librarian uses, plus the agent personas and snippets. Your main AI provider keys live in the Models tab."
      />

      <CustomInstructionsBlock value={customInstructions} />

      <BrainLibrarianBlock />

      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <Label>Agents</Label>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 px-2 text-[11px]"
            onClick={() =>
              setEditingAgent({
                id: newAgentId(),
                name: "New agent",
                description: "",
                instructions: "",
                icon: "spark",
                builtIn: false,
              })
            }
          >
            <HugeiconsIcon icon={Add01Icon} size={12} strokeWidth={1.75} />
            New agent
          </Button>
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {[...BUILTIN_AGENTS, ...customAgents].map((a) => (
            <AgentCard
              key={a.id}
              agent={a}
              active={a.id === activeAgentId}
              onActivate={() => setActiveAgentId(a.id)}
              onEdit={a.builtIn ? null : () => setEditingAgent(a)}
              onDelete={a.builtIn ? null : () => removeAgent(a.id)}
            />
          ))}
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <div className="flex flex-col">
            <Label>Snippets</Label>
            <span className="text-[10.5px] text-muted-foreground">
              Reusable instructions you can drop into any prompt with{" "}
              <code className="rounded bg-muted/50 px-1 font-mono">
                #handle
              </code>
              .
            </span>
          </div>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 px-2 text-[11px]"
            onClick={() =>
              setEditingSnippet({
                id: newSnippetId(),
                handle: "",
                name: "",
                description: "",
                content: "",
              })
            }
          >
            <HugeiconsIcon icon={Add01Icon} size={12} strokeWidth={1.75} />
            New snippet
          </Button>
        </div>

        {snippets.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border/60 bg-card/30 px-4 py-6 text-center text-[11px] text-muted-foreground">
            No snippets yet. Create one and insert it with{" "}
            <code className="font-mono">#handle</code> in the AI input.
          </div>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {snippets.map((s) => (
              <li
                key={s.id}
                className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/60 px-3 py-2"
              >
                <code className="rounded bg-muted/50 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                  #{s.handle}
                </code>
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[12px] font-medium">
                    {s.name}
                  </span>
                  {s.description ? (
                    <span className="truncate text-[10.5px] text-muted-foreground">
                      {s.description}
                    </span>
                  ) : null}
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  className="size-7"
                  onClick={() => setEditingSnippet(s)}
                  title="Edit"
                >
                  <HugeiconsIcon
                    icon={Edit02Icon}
                    size={12}
                    strokeWidth={1.75}
                  />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="size-7 text-muted-foreground hover:text-destructive"
                  onClick={() => removeSnippet(s.id)}
                  title="Delete"
                >
                  <HugeiconsIcon
                    icon={Delete02Icon}
                    size={12}
                    strokeWidth={1.75}
                  />
                </Button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <AgentEditorDialog
        agent={editingAgent}
        existing={customAgents}
        onClose={() => setEditingAgent(null)}
        onSave={(a) => {
          upsertAgent(a);
          setEditingAgent(null);
        }}
      />
      <SnippetEditorDialog
        snippet={editingSnippet}
        existing={snippets}
        onClose={() => setEditingSnippet(null)}
        onSave={(s) => {
          upsertSnippet(s);
          setEditingSnippet(null);
        }}
      />
    </div>
  );
}

function AgentCard({
  agent,
  active,
  onActivate,
  onEdit,
  onDelete,
}: {
  agent: Agent;
  active: boolean;
  onActivate: () => void;
  onEdit: (() => void) | null;
  onDelete: (() => void) | null;
}) {
  const Icon = AGENT_ICONS[agent.icon] ?? SparklesIcon;
  return (
    <div
      className={cn(
        "group relative flex flex-col gap-1.5 rounded-lg border bg-card/60 px-3 py-2.5 transition-colors",
        active
          ? "border-foreground/30 ring-1 ring-foreground/10"
          : "border-border/60 hover:border-border",
      )}
    >
      <div className="flex items-start gap-2">
        <div className="flex size-7 shrink-0 items-center justify-center rounded-md bg-muted/40">
          <HugeiconsIcon icon={Icon} size={14} strokeWidth={1.5} />
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="flex items-center gap-1.5 text-[12.5px] font-medium">
            {agent.name}
            {agent.builtIn ? (
              <span className="rounded bg-muted/50 px-1 py-0.5 text-[9px] tracking-wide text-muted-foreground uppercase">
                Built-in
              </span>
            ) : null}
          </span>
          <span className="line-clamp-2 text-[10.5px] leading-relaxed text-muted-foreground">
            {agent.description}
          </span>
        </div>
      </div>
      <div className="mt-0.5 flex items-center justify-between gap-1">
        <Button
          size="sm"
          variant={active ? "default" : "outline"}
          onClick={onActivate}
          className="h-6 gap-1 px-2 text-[10.5px]"
        >
          {active ? (
            <>
              <HugeiconsIcon
                icon={CheckmarkCircle02Icon}
                size={10}
                strokeWidth={2}
              />
              Active
            </>
          ) : (
            "Use agent"
          )}
        </Button>
        <div className="flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
          {onEdit ? (
            <Button
              size="icon"
              variant="ghost"
              className="size-6"
              onClick={onEdit}
              title="Edit"
            >
              <HugeiconsIcon icon={Edit02Icon} size={11} strokeWidth={1.75} />
            </Button>
          ) : null}
          {onDelete ? (
            <Button
              size="icon"
              variant="ghost"
              className="size-6 text-muted-foreground hover:text-destructive"
              onClick={onDelete}
              title="Delete"
            >
              <HugeiconsIcon icon={Delete02Icon} size={11} strokeWidth={1.75} />
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function AgentEditorDialog({
  agent,
  existing,
  onClose,
  onSave,
}: {
  agent: Agent | null;
  existing: Agent[];
  onClose: () => void;
  onSave: (a: Agent) => void;
}) {
  const [draft, setDraft] = useState<Agent | null>(agent);
  useEffect(() => setDraft(agent), [agent]);
  if (!draft) return null;

  const isNew = !existing.some((a) => a.id === draft.id);
  const canSave =
    draft.name.trim().length > 0 && draft.instructions.trim().length > 0;

  return (
    <Dialog open={!!agent} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-[14px]">
            {isNew ? "New agent" : "Edit agent"}
          </DialogTitle>
        </DialogHeader>
        <div className="-mx-2 max-h-[calc(100vh-14rem)] overflow-y-auto px-2 flex flex-col gap-3">
          <div className="flex gap-2">
            <div className="flex flex-col gap-1">
              <Label>Icon</Label>
              <div className="flex flex-wrap gap-1">
                {ICON_OPTIONS.map((id) => {
                  const Icon = AGENT_ICONS[id] ?? SparklesIcon;
                  const active = draft.icon === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      onClick={() => setDraft({ ...draft, icon: id })}
                      className={cn(
                        "flex size-7 items-center justify-center rounded-md border transition-colors",
                        active
                          ? "border-foreground/40 bg-accent"
                          : "border-border/60 hover:bg-accent/40",
                      )}
                    >
                      <HugeiconsIcon icon={Icon} size={13} strokeWidth={1.75} />
                    </button>
                  );
                })}
              </div>
            </div>
            <div className="flex flex-1 flex-col gap-1">
              <Label>Name</Label>
              <Input
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                className="h-8 text-[12px]"
                placeholder="e.g. Test Engineer"
              />
            </div>
          </div>
          <div className="flex flex-col gap-1">
            <Label>Description</Label>
            <Input
              value={draft.description}
              onChange={(e) =>
                setDraft({ ...draft, description: e.target.value })
              }
              placeholder="One line — shown in the agent picker"
              className="h-8 text-[12px]"
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label>Instructions</Label>
            <Textarea
              value={draft.instructions}
              onChange={(e) =>
                setDraft({ ...draft, instructions: e.target.value })
              }
              placeholder="Persona & rules. Appended to Koden's core system prompt."
              className="min-h-40 resize-y text-[12px] leading-relaxed"
            />
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            disabled={!canSave}
            onClick={() => onSave({ ...draft, builtIn: false })}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SnippetEditorDialog({
  snippet,
  existing,
  onClose,
  onSave,
}: {
  snippet: Snippet | null;
  existing: Snippet[];
  onClose: () => void;
  onSave: (s: Snippet) => void;
}) {
  const [draft, setDraft] = useState<Snippet | null>(snippet);
  useEffect(() => setDraft(snippet), [snippet]);
  if (!draft) return null;

  const handleErr = !draft.handle
    ? "Required."
    : !isValidHandle(draft.handle)
      ? "Lowercase letters, digits, and dashes only."
      : existing.some((s) => s.id !== draft.id && s.handle === draft.handle)
        ? "Already in use."
        : null;
  const canSave =
    !handleErr &&
    draft.name.trim().length > 0 &&
    draft.content.trim().length > 0;

  return (
    <Dialog open={!!snippet} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-[14px]">
            {existing.some((s) => s.id === draft.id)
              ? "Edit snippet"
              : "New snippet"}
          </DialogTitle>
        </DialogHeader>
        <div className="-mx-2 max-h-[calc(100vh-14rem)] overflow-y-auto px-2 flex flex-col gap-3">
          <div className="flex gap-2">
            <div className="flex w-32 flex-col gap-1">
              <Label>Handle</Label>
              <div className="relative">
                <span className="absolute top-1/2 left-2 -translate-y-1/2 font-mono text-[11.5px] text-muted-foreground">
                  #
                </span>
                <Input
                  value={draft.handle}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      handle: normalizeHandle(e.target.value),
                    })
                  }
                  placeholder="review"
                  className="h-8 pl-5 font-mono text-[11.5px]"
                />
              </div>
              {handleErr ? (
                <span className="text-[10px] text-destructive">
                  {handleErr}
                </span>
              ) : null}
            </div>
            <div className="flex flex-1 flex-col gap-1">
              <Label>Name</Label>
              <Input
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="e.g. Pre-merge review checklist"
                className="h-8 text-[12px]"
              />
            </div>
          </div>
          <div className="flex flex-col gap-1">
            <Label>Description</Label>
            <Input
              value={draft.description}
              onChange={(e) =>
                setDraft({ ...draft, description: e.target.value })
              }
              placeholder="One line — shown in the # picker"
              className="h-8 text-[12px]"
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label>Content</Label>
            <Textarea
              value={draft.content}
              onChange={(e) => setDraft({ ...draft, content: e.target.value })}
              placeholder="Inserted into the prompt as a <snippet> block when you use #handle."
              className="min-h-40 resize-y font-mono text-[11.5px] leading-relaxed"
            />
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" disabled={!canSave} onClick={() => onSave(draft)}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CustomInstructionsBlock({ value }: { value: string }) {
  const [draft, setDraft] = useState(value);
  const hadFirstSync = useRef(false);

  useEffect(() => {
    if (!hadFirstSync.current) {
      hadFirstSync.current = true;
      setDraft(value);
    }
  }, [value]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <Label>Custom instructions</Label>
        {/* {savedTick > 0 ? (
          <span className="text-[10px] text-muted-foreground">Saved</span>
        ) : null} */}
        {draft && (
          <Button size="xs" onClick={() => void setCustomInstructions(draft)}>
            Save
          </Button>
        )}
      </div>
      <Textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="e.g. Always reply in concise bullet points. Prefer pnpm over npm. My machine is an M-series Mac."
        className="min-h-[100px] resize-y bg-card/60 font-sans text-[12px] leading-relaxed border border-border"
      />
    </div>
  );
}

// The Brain Librarian's LLM selection + spending cap, changeable here as well as in
// onboarding. Reuses the shared librarian helpers so the picker matches the wizard.
function BrainLibrarianBlock() {
  const [keyed, setKeyed] = useState<ProviderId[]>([]);
  const [provider, setProvider] = useState<ProviderId>("anthropic");
  const [model, setModel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [cap, setCap] = useState("");
  const [spent, setSpent] = useState(0);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [activity, setActivity] = useState<LibrarianActivity | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [keySaving, setKeySaving] = useState(false);

  const saveKey = async () => {
    const k = apiKey.trim();
    if (!k) return;
    setKeySaving(true);
    setErr(null);
    try {
      await setKey(provider, k);
      setKeyed((prev) =>
        prev.includes(provider) ? prev : [...prev, provider],
      );
      setApiKey("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setKeySaving(false);
    }
  };

  // Poll the read-only activity snapshot so the panel reflects real LLM calls as
  // they land (the Librarian is autonomous — this is how you watch it work).
  useEffect(() => {
    let alive = true;
    const tick = () =>
      brainLibrarianActivity()
        .then((a) => alive && setActivity(a))
        .catch(() => {});
    tick();
    const id = window.setInterval(tick, 4000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const [lib, budget, keys] = await Promise.all([
          brainLibrarianStatus(),
          brainBudgetStatus(),
          getAllKeys(),
        ]);
        if (!alive) return;
        setProvider(lib.provider as ProviderId);
        setModel(lib.model);
        setBaseUrl(lib.base_url);
        setCap(budget[0] > 0 ? String(budget[0]) : "");
        setSpent(budget[1]);
        setKeyed(
          PROVIDERS.filter((p) => providerSupportsKey(p.id) && keys[p.id]).map(
            (p) => p.id,
          ),
        );
      } catch {}
    })();
    return () => {
      alive = false;
    };
  }, []);

  const providers: ProviderId[] = [
    ...keyed.filter((p) => p !== "openai-compatible"),
    ...LOCAL_LIB_PROVIDERS,
  ];
  if (provider && !providers.includes(provider)) providers.unshift(provider);

  const local = isLocalLibProvider(provider);
  const rates = libRates(provider, model);
  const cloudModels = local ? [] : libCloudModels(provider);

  const pickProvider = (p: ProviderId) => {
    setProvider(p);
    setSaved(false);
    if (isLocalLibProvider(p)) {
      setBaseUrl(libLocalBaseUrl(p));
      setModel("");
    } else {
      setModel(cheapestLibModel(p));
      setBaseUrl("");
    }
  };

  const save = async () => {
    if (!model.trim()) {
      setErr("Choose a model for the Librarian.");
      return;
    }
    const budget = Number.parseFloat(cap);
    setBusy(true);
    setErr(null);
    try {
      const r = libRates(provider, model.trim());
      await brainSetLibrarian(
        provider,
        model.trim(),
        local ? baseUrl.trim() : "",
        r.inRate,
        r.outRate,
      );
      await brainSetBudget(budget > 0 ? budget : 0);
      setSaved(true);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const selectCls =
    "h-8 w-full rounded-md border bg-background px-2 text-[12px] outline-none [&>option]:bg-popover [&>option]:text-popover-foreground";

  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-col">
        <Label>Brain Librarian</Label>
        <span className="text-[10.5px] text-muted-foreground">
          Optional helper that curates your project memory — it only proposes,
          you approve. Runs on a small, cheap model; set a spending cap to
          enable it (clear to $0 to turn it off). Spent so far: $
          {spent.toFixed(4)}.
        </span>
      </div>

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <Label>Provider</Label>
          <select
            value={provider}
            onChange={(e) => pickProvider(e.target.value as ProviderId)}
            className={selectCls}
          >
            {providers.map((id) => (
              <option key={id} value={id}>
                {getProvider(id).label}
                {isLocalLibProvider(id) ? " (local, free)" : ""}
              </option>
            ))}
          </select>
        </div>

        {local ? (
          <div className="flex flex-col gap-1">
            <Label>Model name</Label>
            <Input
              value={model}
              onChange={(e) => {
                setModel(e.target.value);
                setSaved(false);
              }}
              placeholder="e.g. llama3.1"
              className="h-8 text-[12px]"
            />
          </div>
        ) : (
          <div className="flex flex-col gap-1">
            <Label>Model</Label>
            <select
              value={model}
              onChange={(e) => {
                setModel(e.target.value);
                setSaved(false);
              }}
              className={selectCls}
            >
              {cloudModels.map((m) => {
                const p = MODEL_PRICING[m.id];
                return (
                  <option key={m.id} value={m.id}>
                    {m.label}
                    {p ? ` ($${p.input}/$${p.output})` : ""}
                  </option>
                );
              })}
            </select>
          </div>
        )}
      </div>

      {!local ? (
        <div className="flex items-end gap-2">
          <div className="flex flex-1 flex-col gap-1">
            <Label>
              {getProvider(provider).label} API key
              {keyed.includes(provider) ? " · saved" : ""}
            </Label>
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={
                keyed.includes(provider)
                  ? "•••• saved — paste to replace"
                  : "Paste API key (stored securely)"
              }
              className="h-8 text-[12px]"
            />
          </div>
          <Button
            size="sm"
            disabled={keySaving || !apiKey.trim()}
            onClick={() => void saveKey()}
            className="h-8"
          >
            {keySaving ? "Saving…" : "Save key"}
          </Button>
        </div>
      ) : null}

      {local ? (
        <div className="flex flex-col gap-1">
          <Label>Server URL</Label>
          <Input
            value={baseUrl}
            onChange={(e) => {
              setBaseUrl(e.target.value);
              setSaved(false);
            }}
            placeholder="http://localhost:11434/v1"
            className="h-8 text-[12px]"
          />
        </div>
      ) : null}

      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1">
          <Label>Spending cap (USD)</Label>
          <Input
            inputMode="decimal"
            value={cap}
            onChange={(e) => {
              setCap(e.target.value);
              setSaved(false);
            }}
            placeholder="0.00 (blank / 0 = off)"
            className="h-8 text-[12px]"
          />
        </div>
        <Button
          size="sm"
          disabled={busy}
          onClick={() => void save()}
          className="h-8"
        >
          {busy ? "Saving…" : saved ? "Saved" : "Save"}
        </Button>
      </div>

      <span className="text-[10px] text-muted-foreground">
        {local
          ? "Local models are free; any non-zero cap just switches the Librarian on."
          : `≈ $${rates.inRate} / $${rates.outRate} per 1M tokens. A few dollars is plenty.`}
      </span>
      {err ? <span className="text-[10px] text-destructive">{err}</span> : null}

      <LibrarianActivityPanel
        activity={activity}
        capOn={Number.parseFloat(cap) > 0}
        needsKey={
          Number.parseFloat(cap) > 0 && !local && !keyed.includes(provider)
        }
      />
    </section>
  );
}

function fmtAgoMs(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000) return "just now";
  const m = Math.floor(d / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

function LibrarianActivityPanel({
  activity,
  capOn,
  needsKey,
}: {
  activity: LibrarianActivity | null;
  capOn: boolean;
  needsKey: boolean;
}) {
  return (
    <div className="mt-1 flex flex-col gap-1.5 rounded-md border bg-muted/20 p-2.5">
      <div className="flex items-center justify-between">
        <span className="text-[10.5px] font-medium tracking-tight text-muted-foreground">
          Librarian activity
        </span>
        {activity ? (
          <span className="font-mono text-[10px] text-muted-foreground tabular-nums">
            ${activity.spent_usd.toFixed(4)}
            {activity.ceiling_usd > 0
              ? ` / $${activity.ceiling_usd.toFixed(2)}`
              : ""}
          </span>
        ) : null}
      </div>

      {!capOn ? (
        <span className="text-[10.5px] text-muted-foreground">
          Off — set a spending cap above to enable it.
        </span>
      ) : needsKey ? (
        <span className="text-[10.5px] text-amber-500">
          Enabled, but no API key for this provider — reflect can't call. Add a
          key for it above.
        </span>
      ) : (
        <span className="text-[10.5px] text-muted-foreground">
          Enabled · {activity?.pending_proposals ?? 0} pending proposal
          {activity?.pending_proposals === 1 ? "" : "s"} in the review inbox.
        </span>
      )}

      {activity && activity.calls.length > 0 ? (
        <div className="mt-0.5 flex flex-col gap-0.5">
          {activity.calls.slice(0, 6).map((c) => (
            <div
              key={`${c.at_ms}-${c.status}-${c.model}`}
              className="flex items-center gap-2 font-mono text-[10px]"
            >
              <span
                className={
                  c.status === "spent"
                    ? "text-emerald-500"
                    : "text-muted-foreground"
                }
              >
                ●
              </span>
              <span className="flex-1 truncate text-foreground/80">
                {c.model}
              </span>
              <span className="text-muted-foreground tabular-nums">
                ${c.cost_usd.toFixed(4)}
              </span>
              <span className="w-14 text-right text-muted-foreground">
                {fmtAgoMs(c.at_ms)}
              </span>
            </div>
          ))}
        </div>
      ) : (
        <span className="text-[10px] text-muted-foreground/70">
          No LLM calls yet. It runs after you've worked in a project that has
          memory notes (and a key is set).
        </span>
      )}
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11px] font-medium tracking-tight text-muted-foreground">
      {children}
    </span>
  );
}
