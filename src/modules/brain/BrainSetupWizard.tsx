import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
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
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-[480px] max-w-full rounded-2xl border bg-background p-6 shadow-2xl">
        {doneCount === null ? (
          <>
            <h2 className="text-lg font-semibold">Set up your Koden Brain</h2>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              The Brain indexes your code and notes <span className="text-foreground">locally</span> — no
              cloud — so search, the Brain Map, and your agents all share one understanding of your work.
            </p>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              Point it at the folder that <span className="text-foreground">holds your projects</span>.
              Each project inside becomes its own branch on the map. This choice is your{" "}
              <span className="text-foreground">source of truth</span> — it's saved and survives restarts.
            </p>

            <div className="mt-4">
              <span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                Workspace root
              </span>
              <Input
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void submit();
                  }
                }}
                placeholder="C:\Users\you\Projects"
                className="mt-1 h-9 text-sm"
                autoFocus
              />
              <p className="mt-1.5 text-[11px] text-muted-foreground">
                Absolute path to the parent folder. (A native folder picker is coming — paste the path
                for now.) Only sub-folders that are real projects (have a <code>.git</code> or a
                manifest) are added.
              </p>
            </div>

            {error ? <div className="mt-3 text-xs text-red-500">{error}</div> : null}

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
          </>
        ) : (
          <>
            <h2 className="text-lg font-semibold">Brain is ready</h2>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              Added <span className="text-foreground font-semibold">{doneCount}</span>{" "}
              project{doneCount === 1 ? "" : "s"} from your workspace. Indexing runs in the background —
              open the Brain button (top bar) to search, browse the map, or review memory.
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
          </>
        )}
      </div>
    </div>
  );
}
