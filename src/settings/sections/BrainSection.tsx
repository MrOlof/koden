import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  type BrainStatusReport,
  brainIndexStatus,
  brainRescan,
  brainSetWorkspace,
  brainWorkspaceStatus,
  type WorkspaceStatus,
} from "@/modules/brain/lib/bindings";
import { useCallback, useEffect, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";

// Warning color rides the theme's ANSI yellow (amber = needs-input, per the
// status-color convention) instead of a hardcoded Tailwind literal.
const WARN_CLS = "text-[color:var(--terminal-ansi-yellow)]";

export function BrainSection() {
  return (
    <div className="flex flex-col gap-7">
      <SectionHeader
        title="Brain"
        description="Koden Brain indexes your projects locally — search and context are free and always on. The Librarian's engine and chat settings live in the Librarian tab."
      />
      <IndexBlock />
    </div>
  );
}

// The always-on half of the Brain: what's indexed, from where, and its health.
// This is the post-setup home; first-run setup lives in the onboarding wizard,
// but an unconfigured workspace can also be fixed right here. (The Librarian's
// engine block moved to the Librarian tab — AgentsSection.tsx.)
function IndexBlock() {
  const [report, setReport] = useState<BrainStatusReport | null>(null);
  const [ws, setWs] = useState<WorkspaceStatus | null>(null);
  const [wsPath, setWsPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [r, w] = await Promise.all([
        brainIndexStatus(),
        brainWorkspaceStatus(),
      ]);
      setReport(r);
      setWs(w);
    } catch (e) {
      setErr(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Keep the progress live while the index warms.
  const warming = report?.status.state === "warming";
  useEffect(() => {
    if (!warming) return;
    const id = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(id);
  }, [warming, refresh]);

  const setWorkspace = async () => {
    const p = wsPath.trim();
    if (!p) return;
    setBusy(true);
    setErr(null);
    try {
      await brainSetWorkspace(p);
      setWsPath("");
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const rescan = async () => {
    setBusy(true);
    setErr(null);
    try {
      await brainRescan();
      // Rescan is non-blocking on the worker; give it a beat, then show progress.
      window.setTimeout(() => void refresh(), 1500);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const totalFiles =
    report?.projects.reduce((n, p) => n + p.files, 0) ?? 0;

  const chip =
    report == null ? null : report.status.state === "ready" ? (
      <span className="font-mono text-[10px] text-primary">● ready</span>
    ) : report.status.state === "warming" ? (
      <span className={`font-mono text-[10px] ${WARN_CLS}`}>
        ◐ indexing {report.status.pct}%
      </span>
    ) : (
      <span
        className="font-mono text-[10px] text-destructive"
        title={report.status.reason}
      >
        ● degraded
      </span>
    );

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <Label>Index</Label>
        {chip}
      </div>

      {ws && !ws.configured ? (
        <>
          <span className="text-[10.5px] text-muted-foreground">
            No workspace yet. Point the Brain at the folder that holds your
            projects — each real project inside (git repo or manifest) is
            registered on its own.
          </span>
          <div className="flex items-end gap-2">
            <Input
              value={wsPath}
              onChange={(e) => setWsPath(e.target.value)}
              placeholder="C:\path\to\your\projects"
              className="h-8 font-mono text-[12px]"
            />
            <Button
              size="sm"
              disabled={busy || !wsPath.trim()}
              onClick={() => void setWorkspace()}
              className="h-8"
            >
              {busy ? "Setting…" : "Set workspace"}
            </Button>
          </div>
        </>
      ) : (
        <>
          <div className="flex items-center justify-between gap-2">
            <span
              className="truncate font-mono text-[10.5px] text-muted-foreground"
              title={ws?.root ?? ""}
            >
              {ws?.root ?? "…"}
            </span>
            <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground tabular-nums">
              {report?.projects.length ?? 0} projects · {totalFiles} files
            </span>
          </div>

          {report && report.projects.length > 0 ? (
            <div className="flex max-h-44 flex-col gap-0.5 overflow-y-auto rounded-md border bg-muted/20 p-2">
              {report.projects.map((p) => (
                <div
                  key={p.project.id}
                  className="flex items-center gap-2 font-mono text-[10.5px]"
                >
                  <span className="flex-1 truncate text-foreground/80">
                    {p.project.name}
                  </span>
                  <span className="text-muted-foreground tabular-nums">
                    {p.files} files
                  </span>
                </div>
              ))}
            </div>
          ) : null}

          {report?.status.state === "degraded" ? (
            <span className="text-[10px] text-destructive">
              {report.status.reason}
            </span>
          ) : null}

          <div>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || warming}
              onClick={() => void rescan()}
              className="h-7 text-[11px]"
            >
              {warming ? "Indexing…" : busy ? "Rescanning…" : "Rescan all"}
            </Button>
          </div>
        </>
      )}

      {err ? <span className="text-[10px] text-destructive">{err}</span> : null}
    </section>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11px] font-medium tracking-tight text-muted-foreground">
      {children}
    </span>
  );
}
