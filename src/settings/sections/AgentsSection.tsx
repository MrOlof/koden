import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  getProvider,
  MODEL_PRICING,
  PROVIDERS,
  type ProviderId,
  providerSupportsKey,
} from "@/modules/ai/config";
import { getAllKeys, setKey } from "@/modules/ai/lib/keyring";
import {
  brainBudgetStatus,
  brainLibrarianActivity,
  brainLibrarianStatus,
  brainSetBudget,
  brainSetCurationMode,
  brainSetLibrarian,
  type CurationMode,
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
import { useEffect, useRef, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";

// Warning color rides the theme's ANSI yellow (amber = needs-input, per the
// status-color convention) instead of a hardcoded Tailwind literal.
const WARN_CLS = "text-[color:var(--terminal-ansi-yellow)]";

// The Librarian tab: the chat persona is the Brain's Librarian, so its
// instructions and its engine (provider/model/key/cap) live together here.
export function AgentsSection() {
  const customInstructions = usePreferencesStore((s) => s.customInstructions);

  return (
    <div className="flex flex-col gap-7">
      <SectionHeader
        title="Librarian"
        description="The Librarian is Koden's chat: it answers questions about your projects grounded in the Brain's index and memory notes, and curates that memory over time — autonomously by default, with every change revertible, or through the review inbox if you prefer to approve first."
      />

      <LibrarianInstructionsBlock value={customInstructions} />

      <BrainLibrarianBlock />
    </div>
  );
}

// The Librarian's LLM selection + spending cap, changeable here as well as in
// onboarding. Reuses the shared librarian helpers so the picker matches the wizard.
function BrainLibrarianBlock() {
  const [keyed, setKeyed] = useState<ProviderId[]>([]);
  const [provider, setProvider] = useState<ProviderId>("anthropic");
  const [model, setModel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [cap, setCap] = useState("");
  const [spent, setSpent] = useState(0);
  const [enabled, setEnabled] = useState(false);
  // ADR-018: 'autonomous' (default) applies memory changes itself, revertible;
  // 'review' parks them in the inbox for approval.
  const [curation, setCuration] = useState<CurationMode>("autonomous");
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
        setCuration(lib.curation_mode === "review" ? "review" : "autonomous");
        setCap(budget[0] > 0 ? String(budget[0]) : "");
        setSpent(budget[1]);
        setEnabled(budget[0] > 0);
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
  const needsKey = enabled && !local && !keyed.includes(provider);

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
      await brainSetCurationMode(curation);
      setEnabled(budget > 0);
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
      <div className="flex items-center justify-between">
        <Label>Engine</Label>
        {enabled ? (
          needsKey ? (
            <span className={`font-mono text-[10px] ${WARN_CLS}`}>
              ◐ on · key missing
            </span>
          ) : (
            <span className="font-mono text-[10px] text-primary">
              ● on · {model || "no model"}
            </span>
          )
        ) : (
          <span className="font-mono text-[10px] text-muted-foreground">
            ○ off
          </span>
        )}
      </div>
      <span className="text-[10.5px] text-muted-foreground">
        {enabled
          ? curation === "autonomous"
            ? `Curates project memory after you work and applies the changes itself — every one is revertible from the Brain's Memory tab. Spent $${spent.toFixed(4)} so far. Clear the cap to $0 to turn it off.`
            : `Proposes memory updates after you work; changes wait in the review inbox for your approval. Spent $${spent.toFixed(4)} so far. Clear the cap to $0 to turn it off.`
          : "Off — indexing and search stay free and always-on. Turn the Librarian on by setting a spending cap; it curates project memory (autonomously by default — everything revertible, or review-first below)."}
      </span>

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
                  : "Paste API key (stored in the OS keyring)"
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

      <div className="flex flex-col gap-1">
        <Label>Curation</Label>
        <div className="flex flex-col gap-1.5">
          {(
            [
              {
                id: "autonomous" as CurationMode,
                title: "Autonomous",
                blurb: "applies changes itself; everything revertible",
              },
              {
                id: "review" as CurationMode,
                title: "Review first",
                blurb: "changes wait in the inbox for your approval",
              },
            ] as const
          ).map((opt) => (
            <label
              key={opt.id}
              className="flex cursor-pointer items-center gap-2 text-[11.5px]"
            >
              <input
                type="radio"
                name="librarian-curation"
                checked={curation === opt.id}
                onChange={() => {
                  setCuration(opt.id);
                  setSaved(false);
                }}
                className="accent-[color:var(--primary)]"
              />
              <span className="text-foreground/90">{opt.title}</span>
              <span className="text-muted-foreground">— {opt.blurb}</span>
            </label>
          ))}
        </div>
      </div>

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
          {busy
            ? "Saving…"
            : saved
              ? "Saved"
              : !enabled && Number.parseFloat(cap) > 0
                ? "Enable Librarian"
                : "Save"}
        </Button>
      </div>

      <span className="text-[10px] text-muted-foreground">
        {local
          ? "Local models are free; any non-zero cap just switches the Librarian on."
          : `≈ $${rates.inRate} / $${rates.outRate} per 1M tokens. A few dollars is plenty.`}
      </span>
      <span className="text-[10px] text-muted-foreground">
        Chat uses the same model, but chat calls are not counted against this
        cap — the ledger only meters memory-curation (reflect) runs.
      </span>
      {err ? <span className="text-[10px] text-destructive">{err}</span> : null}

      <LibrarianActivityPanel
        activity={activity}
        capOn={enabled || Number.parseFloat(cap) > 0}
        needsKey={needsKey}
        autonomous={curation === "autonomous"}
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
  autonomous,
}: {
  activity: LibrarianActivity | null;
  capOn: boolean;
  needsKey: boolean;
  autonomous: boolean;
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
        <span className={`text-[10.5px] ${WARN_CLS}`}>
          Enabled, but no API key for this provider — reflect can't call. Add a
          key for it above.
        </span>
      ) : autonomous ? (
        <span className="text-[10.5px] text-muted-foreground">
          Enabled · applies changes autonomously — see Memory changes in the
          Brain's Memory tab (every change is revertible).
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
                  c.status === "spent" ? "text-primary" : "text-muted-foreground"
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

function LibrarianInstructionsBlock({ value }: { value: string }) {
  const [draft, setDraft] = useState(value);
  // Save-on-blur like every other field. Keep the draft in sync with external
  // writes, but not while the field is focused (don't clobber in-progress edits).
  const focused = useRef(false);
  useEffect(() => {
    if (!focused.current) setDraft(value);
  }, [value]);

  return (
    <div className="flex flex-col gap-2">
      <Label>Librarian instructions</Label>
      <Textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onFocus={() => {
          focused.current = true;
        }}
        onBlur={() => {
          focused.current = false;
          if (draft !== value) void setCustomInstructions(draft);
        }}
        placeholder="e.g. Be terse. Prefer pnpm over npm. I'm on Windows 11 with PowerShell."
        className="min-h-[100px] resize-y bg-card/60 font-sans text-[12px] leading-relaxed border border-border"
      />
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
