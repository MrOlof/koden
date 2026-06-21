import {
  FolderOpenIcon,
  HierarchySquare01Icon,
  RoboticIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { brainSetWorkspace, brainWorkspaceStatus } from "./lib/bindings";

const DISMISS_KEY = "koden.brain.setupDismissed";

/**
 * First-run setup for the Koden Brain. Appears once (until a workspace root is chosen
 * or the user dismisses it) and explains the idea, then lets them point the brain at
 * the folder that holds their projects — each child project becomes its own branch,
 * and the choice is the persisted source of truth.
 */
export function BrainSetupWizard() {
  const [show, setShow] = useState(false);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [doneCount, setDoneCount] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    if (localStorage.getItem(DISMISS_KEY)) return;
    brainWorkspaceStatus()
      .then((s) => {
        if (alive && !s.configured) setShow(true);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  if (!show) return null;

  const dismiss = () => {
    localStorage.setItem(DISMISS_KEY, "1");
    setShow(false);
  };

  const browse = async () => {
    setError(null);
    try {
      const sel = await open({
        directory: true,
        multiple: false,
        title: "Choose the folder that holds your projects",
      });
      if (typeof sel === "string") setPath(sel);
    } catch (e) {
      setError(String(e));
    }
  };

  const submit = async () => {
    const p = path.trim();
    if (!p) return;
    setBusy(true);
    setError(null);
    try {
      const added = await brainSetWorkspace(p);
      setDoneCount(added.length);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
      <div className="w-[460px] max-w-full overflow-hidden rounded-2xl border bg-background shadow-2xl">
        {doneCount === null ? (
          <div className="p-7">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl border bg-muted/40">
                <HugeiconsIcon
                  icon={HierarchySquare01Icon}
                  size={20}
                  strokeWidth={1.6}
                />
              </div>
              <div>
                <h2 className="text-base font-semibold leading-tight">
                  Set up your Koden Brain
                </h2>
                <p className="text-xs text-muted-foreground">
                  Point Koden at where your projects live — once.
                </p>
              </div>
            </div>

            {/* Concept diagram: one folder fans out into project branches */}
            <div className="mt-5 rounded-xl border bg-muted/20 px-4 py-4">
              <svg
                viewBox="0 0 256 92"
                className="mx-auto h-[88px] w-full"
                role="img"
                aria-label="One folder fans out into separate project branches"
              >
                <g
                  fill="none"
                  strokeWidth={1.5}
                  className="stroke-muted-foreground/40"
                >
                  <path d="M72 52 C 140 52, 150 22, 226 22" />
                  <path d="M72 52 C 140 52, 150 52, 226 52" />
                  <path d="M72 52 C 140 52, 150 74, 226 74" />
                </g>
                <path
                  d="M20 40 a5 5 0 0 1 5 -5 h13 l5 6 h22 a5 5 0 0 1 5 5 v18 a5 5 0 0 1 -5 5 h-40 a5 5 0 0 1 -5 -5 z"
                  strokeWidth={1.5}
                  className="fill-muted stroke-muted-foreground/60"
                />
                {[
                  { cy: 22, c: "#6ee7b7" },
                  { cy: 52, c: "#93c5fd" },
                  { cy: 74, c: "#fcd34d" },
                ].map((n) => (
                  <circle
                    key={n.cy}
                    cx={234}
                    cy={n.cy}
                    r={8}
                    strokeWidth={1.5}
                    className="stroke-background"
                    style={{ fill: n.c }}
                  />
                ))}
              </svg>
              <p className="mt-1 text-center text-[11px] leading-relaxed text-muted-foreground">
                One folder in — every project inside becomes its own branch on
                the map.
              </p>
            </div>

            <div className="mt-4">
              <div className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                Workspace folder
              </div>
              <div className="mt-1.5 flex items-stretch overflow-hidden rounded-lg border bg-background/60 transition-colors focus-within:border-foreground/40">
                <input
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void submit();
                    }
                  }}
                  placeholder="C:\Users\you\Projects"
                  className="min-w-0 flex-1 bg-transparent px-3 py-2.5 text-sm outline-none placeholder:text-muted-foreground/50"
                  // biome-ignore lint/a11y/noAutofocus: setup modal's single primary input
                  autoFocus
                />
                <button
                  type="button"
                  onClick={() => void browse()}
                  className="flex items-center gap-1.5 border-l px-3.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
                >
                  <HugeiconsIcon
                    icon={FolderOpenIcon}
                    size={15}
                    strokeWidth={1.75}
                  />
                  Browse
                </button>
              </div>
              <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">
                Browse to pick it, or type the full path. Only sub-folders that
                are real projects (have a <code>.git</code> or a manifest) get
                added.
              </p>
            </div>

            {/* What the brain powers, once it's set up */}
            <div className="mt-4 grid grid-cols-3 gap-2">
              {[
                { icon: Search01Icon, label: "Search" },
                { icon: HierarchySquare01Icon, label: "Brain Map" },
                { icon: RoboticIcon, label: "Agents" },
              ].map((f) => (
                <div
                  key={f.label}
                  className="flex flex-col items-center gap-1 rounded-lg border bg-background/40 px-2 py-2.5"
                >
                  <HugeiconsIcon
                    icon={f.icon}
                    size={16}
                    strokeWidth={1.75}
                    className="text-muted-foreground"
                  />
                  <span className="text-[10px] text-muted-foreground">
                    {f.label}
                  </span>
                </div>
              ))}
            </div>
            <p className="mt-2 text-center text-[11px] text-muted-foreground">
              All three share one memory — indexed locally, nothing leaves your
              machine.
            </p>

            {error ? (
              <div className="mt-3 text-xs text-red-500">{error}</div>
            ) : null}

            <div className="mt-5 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={dismiss}
                className="rounded-lg px-3 py-2 text-xs font-medium text-muted-foreground hover:text-foreground"
              >
                Skip for now
              </button>
              <button
                type="button"
                onClick={() => void submit()}
                disabled={busy || !path.trim()}
                className="rounded-lg bg-foreground px-4 py-2 text-xs font-semibold text-background hover:opacity-90 disabled:opacity-50"
              >
                {busy ? "Setting up…" : "Set up brain"}
              </button>
            </div>
          </div>
        ) : (
          <div className="p-7">
            <h2 className="text-base font-semibold">Brain is ready</h2>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              Added{" "}
              <span className="font-semibold text-foreground">{doneCount}</span>{" "}
              project{doneCount === 1 ? "" : "s"} from your workspace. Indexing
              runs in the background — open the Brain button (top bar) to
              search, browse the map, or review memory.
            </p>
            <div className="mt-5 flex justify-end">
              <button
                type="button"
                onClick={() => setShow(false)}
                className="rounded-lg bg-foreground px-4 py-2 text-xs font-semibold text-background hover:opacity-90"
              >
                Done
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
