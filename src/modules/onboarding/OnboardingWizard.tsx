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
  MODEL_PRICING,
  MODELS,
  type ModelId,
  PROVIDERS,
  type ProviderId,
  providerSupportsKey,
} from "@/modules/ai/config";
import { getAllKeys, hasAnyKey, setKey } from "@/modules/ai/lib/keyring";
import {
  brainSetBudget,
  brainSetLibrarian,
  brainSetWorkspace,
  brainWorkspaceStatus,
} from "@/modules/brain/lib/bindings";
import {
  cheapestLibModel,
  isCuratedModelId,
  isLocalLibProvider,
  LOCAL_LIB_PROVIDERS,
  libLocalBaseUrl,
  libRates,
} from "@/modules/brain/lib/librarian";
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
  RoboticIcon,
  RocketIcon,
  type Search01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";

const DONE_KEY = "koden.onboarding.v1.done";

// Providers offered in the inline cloud quick-setup: keyed AND with a curated
// (non "-custom") model, so saving a key here yields a WORKING default model.
// Free-form providers (OpenRouter, openai-compatible) only ship a "-custom"
// placeholder whose model id the user must supply, so offering them here would let
// someone finish onboarding with a non-working AI. Those + local servers route
// to the full Settings > Models panel instead.
const CLOUD_PROVIDERS: readonly ProviderId[] = PROVIDERS.filter(
  (p) =>
    providerSupportsKey(p.id) &&
    MODELS.some((m) => m.provider === p.id && isCuratedModelId(m.id)),
).map((p) => p.id);

function defaultModelForProvider(id: ProviderId): ModelId | null {
  return (
    MODELS.find((m) => m.provider === id && isCuratedModelId(m.id))?.id ?? null
  );
}

type StepMeta = { title: string; subtitle: string };

const STEPS: StepMeta[] = [
  { title: "Welcome to Koden", subtitle: "An AI workspace for your code." },
  { title: "How Koden works", subtitle: "Three pieces, set up once." },
  { title: "Connect an AI model", subtitle: "Bring your own, cloud or local." },
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
 * AI model (real BYOK: cloud key or hand-off to local model settings), choosing
 * the workspace source-of-truth folder, and optionally enabling the Brain Librarian.
 *
 * Every step writes through the real backend: API keys go to the OS keyring via
 * secrets_set (never logged), the active model to the settings store, the workspace
 * to brain_set_workspace, and the Librarian provider/model + budget to
 * brain_set_librarian / brain_set_budget. Nothing here fakes a setting. Gated on a
 * localStorage completion key + an unconfigured workspace so existing users are
 * never re-prompted.
 */
export function OnboardingWizard() {
  const [show, setShow] = useState(false);
  const [step, setStep] = useState(0);

  // AI step
  const [aiConfigured, setAiConfigured] = useState(false);
  const [keyedProviders, setKeyedProviders] = useState<ProviderId[]>([]);
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
  const [libProvider, setLibProvider] = useState<ProviderId | "">("");
  const [libModel, setLibModel] = useState("");
  const [libBaseUrl, setLibBaseUrl] = useState("");
  const [libBudget, setLibBudget] = useState("");
  const [libBusy, setLibBusy] = useState(false);
  const [libEnabled, setLibEnabled] = useState(false);

  const [error, setError] = useState<string | null>(null);

  const loadKeys = useCallback(async () => {
    try {
      const keys = await getAllKeys();
      setAiConfigured(hasAnyKey(keys));
      setKeyedProviders(
        PROVIDERS.filter((p) => providerSupportsKey(p.id) && keys[p.id]).map(
          (p) => p.id,
        ),
      );
    } catch {}
  }, []);

  // First-run: show once, only when setup isn't done AND no workspace is configured.
  useEffect(() => {
    let alive = true;
    if (localStorage.getItem(DONE_KEY)) return;
    brainWorkspaceStatus()
      .then((s) => {
        if (!alive || s.configured) return;
        setShow(true);
        void loadKeys();
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [loadKeys]);

  // Manual re-open from the Brain menu ("Setup guide") — bypasses the first-run
  // gate so users can re-run setup or change provider/model/workspace anytime.
  useEffect(() => {
    const openIt = () => {
      setStep(0);
      setLibProvider("");
      setError(null);
      setShow(true);
      void loadKeys();
    };
    window.addEventListener("koden:open-onboarding", openIt);
    return () => window.removeEventListener("koden:open-onboarding", openIt);
  }, [loadKeys]);

  // The set of providers the Librarian can use: connected cloud keys (minus the
  // free-form openai-compatible, which needs its own base URL) + the local servers.
  const libProviders: ProviderId[] = [
    ...keyedProviders.filter((p) => p !== "openai-compatible"),
    ...LOCAL_LIB_PROVIDERS,
  ];

  // Switch the Librarian to a provider: cloud → its cheapest model; local → a
  // model-name field + the server's default URL, and a nominal cap so one click
  // enables it (local is free, but ceiling > 0 is the on-switch).
  const pickLibProvider = useCallback((p: ProviderId) => {
    setLibProvider(p);
    if (isLocalLibProvider(p)) {
      setLibBaseUrl(libLocalBaseUrl(p));
      setLibModel("");
      setLibBudget((b) => (b.trim() ? b : "1"));
    } else {
      setLibModel(cheapestLibModel(p));
      setLibBaseUrl("");
    }
  }, []);

  // Default the Librarian provider when the user lands on its step.
  useEffect(() => {
    if (step !== 4 || libProvider) return;
    const initial = keyedProviders.includes(provider)
      ? provider
      : (keyedProviders.find((p) => p !== "openai-compatible") ?? "ollama");
    pickLibProvider(initial);
  }, [step, libProvider, keyedProviders, provider, pickLibProvider]);

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
      setKeyedProviders((prev) =>
        prev.includes(provider) ? prev : [...prev, provider],
      );
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
    // No budget (or no provider) → leave the Librarian off (the safe default).
    if (!(budget > 0) || !libProvider) {
      next();
      return;
    }
    const model = libModel.trim();
    if (!model) {
      setError("Choose a model for the Librarian.");
      return;
    }
    setLibBusy(true);
    setError(null);
    try {
      const { inRate, outRate } = libRates(libProvider, model);
      const baseUrl = isLocalLibProvider(libProvider) ? libBaseUrl.trim() : "";
      await brainSetLibrarian(libProvider, model, baseUrl, inRate, outRate);
      await brainSetBudget(budget);
      setLibEnabled(true);
      next();
    } catch (e) {
      setError(String(e));
    } finally {
      setLibBusy(false);
    }
  };

  const meta = STEPS[step];
  const libBudgetOn = Number.parseFloat(libBudget) > 0;

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
              providers={libProviders}
              provider={libProvider}
              onProvider={pickLibProvider}
              model={libModel}
              onModel={setLibModel}
              baseUrl={libBaseUrl}
              onBaseUrl={setLibBaseUrl}
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
                  : libBudgetOn
                    ? "Enable & continue"
                    : "Leave it off"}
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
        Koden is a terminal and editor with an AI that actually does the work.
        It reads your code, runs commands, and remembers how your projects fit
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
      <FeatureRow icon={HierarchySquare01Icon} title="The Brain">
        A local index of your projects for instant search and a live map of how
        your code connects. Nothing is uploaded.
      </FeatureRow>
      <FeatureRow icon={CloudIcon} title="The AI">
        Bring your own model: a cloud API key (Anthropic, OpenAI, …) or a local
        model (Ollama, LM Studio). You're in control of cost and privacy.
      </FeatureRow>
      <FeatureRow icon={RoboticIcon} title="The Librarian">
        Koden's chat: it answers questions about your projects from the
        Brain's memory, and optionally tidies your notes over time. Curation
        is off by default; you can turn it on in a moment.
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
        <div className="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-primary">
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
          Prefer a local or open model (Ollama, LM Studio)? It's free and
          private. Set it up in the full model settings.
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
  providers,
  provider,
  onProvider,
  model,
  onModel,
  baseUrl,
  onBaseUrl,
  budget,
  onBudget,
}: {
  providers: ProviderId[];
  provider: ProviderId | "";
  onProvider: (p: ProviderId) => void;
  model: string;
  onModel: (v: string) => void;
  baseUrl: string;
  onBaseUrl: (v: string) => void;
  budget: string;
  onBudget: (v: string) => void;
}) {
  const local = !!provider && isLocalLibProvider(provider);
  const rates = provider
    ? libRates(provider, model)
    : { inRate: 0, outRate: 0 };
  const cloudModels =
    provider && !local
      ? MODELS.filter((m) => m.provider === provider && isCuratedModelId(m.id))
      : [];

  return (
    <div className="space-y-4">
      <p className="text-xs leading-relaxed text-muted-foreground">
        The Librarian does light background work, so it runs best on a small,
        cheap model. It proposes tidy-ups to your project memory; you always
        approve before anything is saved.
      </p>

      <div>
        <div className="mb-1.5 text-xs font-medium text-muted-foreground">
          Run it on
        </div>
        <select
          value={provider}
          onChange={(e) => onProvider(e.target.value as ProviderId)}
          className="h-9 w-full rounded-lg border bg-background px-2 text-sm outline-none [&>option]:bg-popover [&>option]:text-popover-foreground"
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
        <div className="space-y-2">
          <div>
            <div className="mb-1.5 text-xs font-medium text-muted-foreground">
              Model name
            </div>
            <Input
              value={model}
              onChange={(e) => onModel(e.target.value)}
              placeholder="e.g. llama3.1"
              className="h-9 text-sm"
            />
          </div>
          <div>
            <div className="mb-1.5 text-xs font-medium text-muted-foreground">
              Server URL
            </div>
            <Input
              value={baseUrl}
              onChange={(e) => onBaseUrl(e.target.value)}
              placeholder="http://localhost:11434/v1"
              className="h-9 text-sm"
            />
          </div>
          <p className="text-[11px] text-muted-foreground">
            Free and private. Needs the local server running with this model
            loaded.
          </p>
        </div>
      ) : provider ? (
        <div>
          <div className="mb-1.5 flex items-center justify-between text-xs font-medium text-muted-foreground">
            <span>Model</span>
            <span className="font-normal text-muted-foreground/80">
              {`$${rates.inRate} / $${rates.outRate} per 1M tokens`}
            </span>
          </div>
          <select
            value={model}
            onChange={(e) => onModel(e.target.value)}
            className="h-9 w-full rounded-lg border bg-background px-2 text-sm outline-none [&>option]:bg-popover [&>option]:text-popover-foreground"
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
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            A small, cheap model is recommended; it only writes proposals you
            approve.
          </p>
        </div>
      ) : null}

      <div>
        <div className="mb-1.5 text-xs font-medium text-muted-foreground">
          Spending cap (USD)
        </div>
        <Input
          inputMode="decimal"
          value={budget}
          onChange={(e) => onBudget(e.target.value)}
          placeholder="0.00 (blank keeps it off)"
          className="h-9 text-sm"
        />
        <p className="mt-1.5 text-[11px] text-muted-foreground">
          {local
            ? "Local models are free; any non-zero value just switches the Librarian on."
            : "A few dollars is plenty. A spending cap, not a recurring charge; spend only counts up, clear it to $0 to disable."}
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
        : "AI model: set up later in Settings › Models",
    },
    {
      ok: wsAdded !== null && wsAdded > 0,
      icon: FolderOpenIcon,
      label:
        wsAdded !== null && wsAdded > 0
          ? `Workspace set: ${wsAdded} project${wsAdded === 1 ? "" : "s"} indexing`
          : "Workspace: add a folder later from the Brain button",
    },
    {
      ok: libEnabled,
      icon: RoboticIcon,
      label: libEnabled
        ? "Librarian enabled"
        : "Librarian curation off (enable anytime in Settings › Librarian)",
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
              className={r.ok ? "text-primary" : "text-muted-foreground/60"}
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
