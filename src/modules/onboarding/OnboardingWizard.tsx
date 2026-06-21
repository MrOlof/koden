import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  getProvider,
  MODELS,
  type ModelId,
  PROVIDERS,
  type ProviderId,
  providerSupportsKey,
} from "@/modules/ai/config";
import { getAllKeys, hasAnyKey, setKey } from "@/modules/ai/lib/keyring";
import {
  brainSetBudget,
  brainSetWorkspace,
  brainWorkspaceStatus,
} from "@/modules/brain/lib/bindings";
import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { emitKeysChanged, setDefaultModel } from "@/modules/settings/store";
import {
  ArrowLeft01Icon,
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
  CloudIcon,
  ComputerIcon,
  FolderOpenIcon,
  HierarchySquare01Icon,
  Key01Icon,
  RoboticIcon,
  RocketIcon,
  type Search01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

const DONE_KEY = "koden.onboarding.v1.done";

// Providers offered in the inline cloud quick-setup: keyed AND with at least one
// curated model, so saving a key here yields a working default model. Everything
// else (OpenRouter free-text models, local servers, custom endpoints) is richer
// than a wizard should be — those route to the full Settings > Models panel.
const CLOUD_PROVIDERS: readonly ProviderId[] = PROVIDERS.filter(
  (p) => providerSupportsKey(p.id) && MODELS.some((m) => m.provider === p.id),
).map((p) => p.id);

function defaultModelForProvider(id: ProviderId): ModelId | null {
  return MODELS.find((m) => m.provider === id)?.id ?? null;
}

type StepMeta = { title: string; subtitle: string };

const STEPS: StepMeta[] = [
  { title: "Welcome to Koden", subtitle: "An AI workspace for your code." },
  { title: "How Koden works", subtitle: "Three pieces, set up once." },
  {
    title: "Connect an AI model",
    subtitle: "Bring your own — cloud or local.",
  },
  {
    title: "Choose your projects folder",
    subtitle: "The source of truth for the brain.",
  },
  {
    title: "Smarter memory (optional)",
    subtitle: "Let the Librarian curate your notes.",
  },
  { title: "You're all set", subtitle: "Here's what's ready." },
];

/**
 * First-run "Welcome to Koden" onboarding. A multi-step modal built on the shared
 * Dialog that walks a new user through: what Koden is, how it works, connecting an
 * AI model (real BYOK — cloud key or hand-off to local model settings), choosing
 * the workspace source-of-truth folder, and optionally enabling the Brain Librarian.
 *
 * Every step writes through the real backend: API keys go to the OS keyring via
 * secrets_set (never logged), the active model to the settings store, the workspace
 * to brain_set_workspace, and the Librarian budget to brain_set_budget. Nothing here
 * fakes a setting. Gated on a localStorage completion key + an unconfigured workspace
 * so existing users are never re-prompted.
 */
export function OnboardingWizard() {
  const [show, setShow] = useState(false);
  const [step, setStep] = useState(0);

  // AI step
  const [aiConfigured, setAiConfigured] = useState(false);
  const [provider, setProvider] = useState<ProviderId>(
    (CLOUD_PROVIDERS.includes("anthropic")
      ? "anthropic"
      : CLOUD_PROVIDERS[0]) ?? "anthropic",
  );
  const [apiKey, setApiKey] = useState("");
  const [aiBusy, setAiBusy] = useState(false);

  // Workspace step
  const [wsPath, setWsPath] = useState("");
  const [wsBusy, setWsBusy] = useState(false);
  const [wsAdded, setWsAdded] = useState<number | null>(null);

  // Librarian step
  const [anthropicPresent, setAnthropicPresent] = useState(false);
  const [libKey, setLibKey] = useState("");
  const [libBudget, setLibBudget] = useState("");
  const [libBusy, setLibBusy] = useState(false);
  const [libEnabled, setLibEnabled] = useState(false);

  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    if (localStorage.getItem(DONE_KEY)) return;
    brainWorkspaceStatus()
      .then(async (s) => {
        if (!alive || s.configured) return;
        setShow(true);
        try {
          const keys = await getAllKeys();
          if (alive) {
            setAiConfigured(hasAnyKey(keys));
            setAnthropicPresent(!!keys.anthropic);
          }
        } catch {}
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  if (!show) return null;

  const finish = () => {
    localStorage.setItem(DONE_KEY, "1");
    setShow(false);
  };

  const next = () => setStep((s) => Math.min(STEPS.length - 1, s + 1));
  const back = () => setStep((s) => Math.max(0, s - 1));

  // Save the API key (if typed) + pick a default model for the provider, then advance.
  const saveAiAndNext = async () => {
    const key = apiKey.trim();
    if (!key) {
      next();
      return;
    }
    setAiBusy(true);
    setError(null);
    try {
      await setKey(provider, key);
      const model = defaultModelForProvider(provider);
      if (model) await setDefaultModel(model);
      await emitKeysChanged();
      setAiConfigured(true);
      if (provider === "anthropic") setAnthropicPresent(true);
      setApiKey("");
      next();
    } catch (e) {
      setError(String(e));
    } finally {
      setAiBusy(false);
    }
  };

  const browseWorkspace = async () => {
    setError(null);
    try {
      const sel = await open({
        directory: true,
        multiple: false,
        title: "Choose the folder that holds your projects",
      });
      if (typeof sel === "string") setWsPath(sel);
    } catch (e) {
      setError(String(e));
    }
  };

  const applyWorkspaceAndNext = async () => {
    const p = wsPath.trim();
    if (!p) {
      next();
      return;
    }
    setWsBusy(true);
    setError(null);
    try {
      const added = await brainSetWorkspace(p);
      setWsAdded(added.length);
      next();
    } catch (e) {
      setError(String(e));
    } finally {
      setWsBusy(false);
    }
  };

  const enableLibrarianAndNext = async () => {
    const budget = Number.parseFloat(libBudget);
    const key = libKey.trim();
    // Nothing to do → just advance, Librarian stays off (the safe default).
    if (!key && !(budget > 0)) {
      next();
      return;
    }
    setLibBusy(true);
    setError(null);
    try {
      if (key) {
        await setKey("anthropic", key);
        await emitKeysChanged();
        setAnthropicPresent(true);
        setLibKey("");
      }
      if (budget > 0) {
        await brainSetBudget(budget);
        setLibEnabled(true);
      }
      next();
    } catch (e) {
      setError(String(e));
    } finally {
      setLibBusy(false);
    }
  };

  const meta = STEPS[step];

  return (
    <Dialog open={show} onOpenChange={() => {}}>
      <DialogContent
        showCloseButton={false}
        onInteractOutside={(e) => e.preventDefault()}
        onEscapeKeyDown={(e) => e.preventDefault()}
        className="sm:max-w-lg"
      >
        {/* step indicator */}
        <div className="flex items-center justify-center gap-1.5">
          {STEPS.map((s, i) => (
            <span
              key={s.title}
              className={`h-1.5 rounded-full transition-all ${
                i === step
                  ? "w-6 bg-foreground"
                  : i < step
                    ? "w-1.5 bg-foreground/60"
                    : "w-1.5 bg-border"
              }`}
            />
          ))}
        </div>

        <DialogHeader className="items-center text-center">
          <div className="mb-1 flex h-11 w-11 items-center justify-center rounded-2xl border bg-muted/40">
            <HugeiconsIcon icon={STEP_ICON[step]} size={22} strokeWidth={1.6} />
          </div>
          <DialogTitle className="text-lg">{meta.title}</DialogTitle>
          <DialogDescription>{meta.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="min-h-[180px]">
          {step === 0 && <WelcomeBody />}
          {step === 1 && <HowBody />}
          {step === 2 && (
            <AiBody
              configured={aiConfigured}
              provider={provider}
              onProvider={setProvider}
              apiKey={apiKey}
              onApiKey={setApiKey}
            />
          )}
          {step === 3 && (
            <WorkspaceBody
              path={wsPath}
              onPath={setWsPath}
              onBrowse={() => void browseWorkspace()}
            />
          )}
          {step === 4 && (
            <LibrarianBody
              anthropicPresent={anthropicPresent}
              libKey={libKey}
              onLibKey={setLibKey}
              budget={libBudget}
              onBudget={setLibBudget}
            />
          )}
          {step === 5 && (
            <DoneBody
              aiConfigured={aiConfigured}
              wsAdded={wsAdded}
              libEnabled={libEnabled}
            />
          )}
        </div>

        {error ? <div className="text-xs text-destructive">{error}</div> : null}

        {/* footer */}
        <div className="flex items-center justify-between">
          <Button
            variant="ghost"
            size="sm"
            onClick={back}
            className={step === 0 ? "invisible" : ""}
          >
            <HugeiconsIcon icon={ArrowLeft01Icon} size={15} strokeWidth={2} />
            Back
          </Button>

          <div className="flex items-center gap-2">
            {step < STEPS.length - 1 ? (
              <button
                type="button"
                onClick={finish}
                className="rounded-lg px-3 py-2 text-xs font-medium text-muted-foreground hover:text-foreground"
              >
                Skip setup
              </button>
            ) : null}

            {step === 0 && (
              <Button size="sm" onClick={next}>
                Get started
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
            {step === 1 && (
              <Button size="sm" onClick={next}>
                Next
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
            {step === 2 && (
              <Button
                size="sm"
                disabled={aiBusy}
                onClick={() => void saveAiAndNext()}
              >
                {aiBusy
                  ? "Saving…"
                  : apiKey.trim()
                    ? "Save & continue"
                    : "Continue"}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
            {step === 3 && (
              <Button
                size="sm"
                disabled={wsBusy}
                onClick={() => void applyWorkspaceAndNext()}
              >
                {wsBusy
                  ? "Adding…"
                  : wsPath.trim()
                    ? "Set folder & continue"
                    : "Continue"}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
            {step === 4 && (
              <Button
                size="sm"
                disabled={libBusy}
                onClick={() => void enableLibrarianAndNext()}
              >
                {libBusy
                  ? "Saving…"
                  : libBudget.trim() || libKey.trim()
                    ? "Enable & continue"
                    : "Skip — leave off"}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
            {step === 5 && (
              <Button size="sm" onClick={finish}>
                Done
                <HugeiconsIcon
                  icon={CheckmarkCircle02Icon}
                  size={15}
                  strokeWidth={2}
                />
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const STEP_ICON = [
  RocketIcon,
  HierarchySquare01Icon,
  CloudIcon,
  FolderOpenIcon,
  RoboticIcon,
  CheckmarkCircle02Icon,
];

function FeatureRow({
  icon,
  title,
  children,
}: {
  icon: typeof Search01Icon;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3">
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/30">
        <HugeiconsIcon icon={icon} size={16} strokeWidth={1.75} />
      </div>
      <div>
        <div className="text-sm font-medium">{title}</div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          {children}
        </p>
      </div>
    </div>
  );
}

function WelcomeBody() {
  return (
    <div className="space-y-3 text-center">
      <p className="text-sm leading-relaxed text-muted-foreground">
        Koden is a terminal and editor with an AI that actually does the work —
        it reads your code, runs commands, and remembers how your projects fit
        together.
      </p>
      <p className="text-sm leading-relaxed text-muted-foreground">
        This quick setup gets you running in under a minute. You can skip any
        step and change everything later in Settings.
      </p>
    </div>
  );
}

function HowBody() {
  return (
    <div className="space-y-4">
      <FeatureRow icon={HierarchySquare01Icon} title="The Brain — local index">
        Koden indexes your projects on your machine for instant search and a
        live map of how your code connects. Nothing is uploaded.
      </FeatureRow>
      <FeatureRow icon={CloudIcon} title="The AI — your model">
        Bring your own model: a cloud API key (Anthropic, OpenAI, …) or a
        local/open model (Ollama, LM Studio). You're in control of cost and
        privacy.
      </FeatureRow>
      <FeatureRow icon={RoboticIcon} title="The Librarian — optional memory">
        An optional helper that tidies your project notes over time. Off by
        default; you can turn it on in a moment.
      </FeatureRow>
    </div>
  );
}

function AiBody({
  configured,
  provider,
  onProvider,
  apiKey,
  onApiKey,
}: {
  configured: boolean;
  provider: ProviderId;
  onProvider: (p: ProviderId) => void;
  apiKey: string;
  onApiKey: (v: string) => void;
}) {
  const info = getProvider(provider);
  return (
    <div className="space-y-4">
      {configured ? (
        <div className="flex items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-600 dark:text-emerald-400">
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            size={15}
            strokeWidth={2}
          />
          An AI provider is already set up. You can add another below or
          continue.
        </div>
      ) : null}

      <div>
        <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <HugeiconsIcon icon={CloudIcon} size={14} strokeWidth={1.75} /> Use a
          cloud model
        </div>
        <div className="flex items-stretch gap-2">
          <select
            value={provider}
            onChange={(e) => onProvider(e.target.value as ProviderId)}
            className="h-9 rounded-lg border bg-background px-2 text-sm outline-none [&>option]:bg-popover [&>option]:text-popover-foreground"
          >
            {CLOUD_PROVIDERS.map((id) => (
              <option key={id} value={id}>
                {getProvider(id).label}
              </option>
            ))}
          </select>
          <Input
            type="password"
            value={apiKey}
            onChange={(e) => onApiKey(e.target.value)}
            placeholder={
              info.keyPrefix ? `${info.keyPrefix}…` : "Paste API key"
            }
            className="h-9 flex-1 text-sm"
            autoFocus
          />
        </div>
        <p className="mt-1.5 text-[11px] text-muted-foreground">
          Stored in your OS keychain, never uploaded by Koden. Get a key at{" "}
          <span className="text-foreground/80">
            {info.consoleUrl.replace(/^https?:\/\//, "")}
          </span>
          .
        </p>
      </div>

      <div className="flex items-center gap-2 rounded-lg border bg-muted/20 px-3 py-2">
        <HugeiconsIcon
          icon={ComputerIcon}
          size={16}
          strokeWidth={1.75}
          className="shrink-0 text-muted-foreground"
        />
        <div className="flex-1 text-xs text-muted-foreground">
          Prefer a local / open model (Ollama, LM Studio)? Free and private —
          set it up in the full model settings.
        </div>
        <Button
          variant="outline"
          size="xs"
          onClick={() => void openSettingsWindow("models")}
        >
          Open
        </Button>
      </div>
    </div>
  );
}

function WorkspaceBody({
  path,
  onPath,
  onBrowse,
}: {
  path: string;
  onPath: (v: string) => void;
  onBrowse: () => void;
}) {
  return (
    <div className="space-y-4">
      <div className="rounded-xl border bg-muted/20 px-4 py-3">
        <svg
          viewBox="0 0 256 76"
          className="mx-auto h-[72px] w-full"
          role="img"
          aria-label="One folder fans out into separate project branches"
        >
          <g
            fill="none"
            strokeWidth={1.5}
            className="stroke-muted-foreground/40"
          >
            <path d="M72 40 C 140 40, 150 14, 226 14" />
            <path d="M72 40 C 140 40, 150 40, 226 40" />
            <path d="M72 40 C 140 40, 150 64, 226 64" />
          </g>
          <path
            d="M20 30 a5 5 0 0 1 5 -5 h13 l5 6 h22 a5 5 0 0 1 5 5 v16 a5 5 0 0 1 -5 5 h-40 a5 5 0 0 1 -5 -5 z"
            strokeWidth={1.5}
            className="fill-muted stroke-muted-foreground/60"
          />
          {[
            { cy: 14, c: "#6ee7b7" },
            { cy: 40, c: "#93c5fd" },
            { cy: 64, c: "#fcd34d" },
          ].map((n) => (
            <circle
              key={n.cy}
              cx={234}
              cy={n.cy}
              r={7}
              strokeWidth={1.5}
              className="stroke-background"
              style={{ fill: n.c }}
            />
          ))}
        </svg>
        <p className="mt-1 text-center text-[11px] text-muted-foreground">
          Each project inside becomes its own branch on the map.
        </p>
      </div>

      <div className="flex items-stretch overflow-hidden rounded-lg border bg-background/60 focus-within:border-foreground/40">
        <input
          value={path}
          onChange={(e) => onPath(e.target.value)}
          placeholder="C:\Users\you\Projects"
          className="min-w-0 flex-1 bg-transparent px-3 py-2.5 text-sm outline-none placeholder:text-muted-foreground/50"
        />
        <button
          type="button"
          onClick={onBrowse}
          className="flex items-center gap-1.5 border-l px-3.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
        >
          <HugeiconsIcon icon={FolderOpenIcon} size={15} strokeWidth={1.75} />
          Browse
        </button>
      </div>
      <p className="text-[11px] text-muted-foreground">
        Only sub-folders that are real projects (a <code>.git</code> or a
        manifest) get added.
      </p>
    </div>
  );
}

function LibrarianBody({
  anthropicPresent,
  libKey,
  onLibKey,
  budget,
  onBudget,
}: {
  anthropicPresent: boolean;
  libKey: string;
  onLibKey: (v: string) => void;
  budget: string;
  onBudget: (v: string) => void;
}) {
  return (
    <div className="space-y-4">
      <p className="text-xs leading-relaxed text-muted-foreground">
        The Librarian uses an <span className="text-foreground">Anthropic</span>{" "}
        model to propose tidy-ups to your project memory — you always approve
        before anything is saved. It's Anthropic-only for now and costs a little
        per run, so it's gated by a small monthly budget (set $0 anytime to turn
        it off).
      </p>

      {anthropicPresent ? (
        <div className="flex items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-600 dark:text-emerald-400">
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            size={15}
            strokeWidth={2}
          />
          Anthropic key detected — just set a budget below to enable it.
        </div>
      ) : (
        <div>
          <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <HugeiconsIcon icon={Key01Icon} size={14} strokeWidth={1.75} />{" "}
            Anthropic API key
          </div>
          <Input
            type="password"
            value={libKey}
            onChange={(e) => onLibKey(e.target.value)}
            placeholder="sk-ant-…"
            className="h-9 text-sm"
          />
        </div>
      )}

      <div>
        <div className="mb-1.5 text-xs font-medium text-muted-foreground">
          Monthly budget (USD)
        </div>
        <Input
          inputMode="decimal"
          value={budget}
          onChange={(e) => onBudget(e.target.value)}
          placeholder="0.00 — leave blank to keep it off"
          className="h-9 text-sm"
        />
        <p className="mt-1.5 text-[11px] text-muted-foreground">
          A few dollars is plenty. This is a spending cap, not a recurring
          charge — spend only counts up; clear it to $0 to disable.
        </p>
      </div>
    </div>
  );
}

function DoneBody({
  aiConfigured,
  wsAdded,
  libEnabled,
}: {
  aiConfigured: boolean;
  wsAdded: number | null;
  libEnabled: boolean;
}) {
  const rows: { ok: boolean; icon: typeof Search01Icon; label: string }[] = [
    {
      ok: aiConfigured,
      icon: CloudIcon,
      label: aiConfigured
        ? "AI model connected"
        : "AI model — set up later in Settings › Models",
    },
    {
      ok: wsAdded !== null && wsAdded > 0,
      icon: FolderOpenIcon,
      label:
        wsAdded !== null && wsAdded > 0
          ? `Workspace set — ${wsAdded} project${wsAdded === 1 ? "" : "s"} indexing`
          : "Workspace — add a folder later from the Brain button",
    },
    {
      ok: libEnabled,
      icon: RoboticIcon,
      label: libEnabled
        ? "Librarian enabled"
        : "Librarian off (enable anytime in the Brain panel)",
    },
  ];
  return (
    <div className="space-y-3">
      <p className="text-sm leading-relaxed text-muted-foreground">
        That's it. Open the <span className="text-foreground">Brain</span>{" "}
        button in the top bar to search, browse the map, or review memory.
      </p>
      <div className="space-y-2">
        {rows.map((r) => (
          <div key={r.label} className="flex items-center gap-2.5 text-sm">
            <HugeiconsIcon
              icon={r.ok ? CheckmarkCircle02Icon : r.icon}
              size={17}
              strokeWidth={1.75}
              className={r.ok ? "text-emerald-500" : "text-muted-foreground/60"}
            />
            <span className={r.ok ? "" : "text-muted-foreground"}>
              {r.label}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
