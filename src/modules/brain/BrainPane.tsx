import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Cancel01Icon, Search01Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import {
  brainAddProject,
  brainDoctor,
  brainIndexStatus,
  brainListProjects,
  brainNotes,
  brainProposals,
  brainRescan,
  brainResolveProposal,
  brainSearch,
  type BrainStatusReport,
  type Hit,
  type MemoryProposal,
  type NoteSummary,
  type Project,
} from "./lib/bindings";

const MIN_QUERY_LEN = 2;
const DEBOUNCE_MS = 300;
const STATUS_POLL_MS = 2000;
// Worker events (doctor/resolve) are async; refetch shortly after firing one.
const ASYNC_SETTLE_MS = 500;

type Mode = "search" | "memory";

function basename(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(i + 1) : p;
}

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

function statusLabel(report: BrainStatusReport | null): string {
  if (!report) return "Connecting…";
  const projectCount = report.projects.length;
  const fileCount = report.projects.reduce((acc, p) => acc + p.files, 0);
  switch (report.status.state) {
    case "warming":
      return `Indexing… ${report.status.pct}% · ${projectCount} project(s)`;
    case "ready":
      return `Ready · ${projectCount} project(s) · ${fileCount} files`;
    case "degraded":
      return `Degraded: ${report.status.reason}`;
  }
}

/**
 * Koden Brain pane (P1): a Search mode over the index and a Memory mode that
 * surfaces curated notes + the doctor's review inbox (approve/reject proposals).
 */
export function BrainPane() {
  const [mode, setMode] = useState<Mode>("search");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Hit[]>([]);
  const [searching, setSearching] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<string | null>(null); // null = all
  const [report, setReport] = useState<BrainStatusReport | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [addPath, setAddPath] = useState("");
  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [proposals, setProposals] = useState<MemoryProposal[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastKeyboardNavAt = useRef(0);

  const active = query.trim().length > 0;

  useEffect(() => {
    let alive = true;
    const refresh = async () => {
      try {
        const [ps, rep] = await Promise.all([brainListProjects(), brainIndexStatus()]);
        if (alive) {
          setProjects(ps);
          setReport(rep);
        }
      } catch (e) {
        console.error("brain status failed:", e);
      }
    };
    void refresh();
    const id = setInterval(() => void refresh(), STATUS_POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  // Debounced search (alive-flag cancel — mirrors ExplorerSearch).
  useEffect(() => {
    const q = query.trim();
    if (q.length < MIN_QUERY_LEN) {
      setResults([]);
      setSelectedIndex(0);
      setSearching(false);
      return;
    }
    setSearching(true);
    let alive = true;
    const handle = setTimeout(async () => {
      try {
        const hits = await brainSearch(q, project, 30);
        if (alive) {
          setResults(hits);
          setSelectedIndex(0);
        }
      } catch (e) {
        if (alive) {
          console.error("brain_search failed:", e);
          setResults([]);
        }
      } finally {
        if (alive) setSearching(false);
      }
    }, DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(handle);
    };
  }, [query, project]);

  useEffect(() => {
    if (active && results.length > 0) {
      const el = scrollRef.current?.querySelector(`[data-index="${selectedIndex}"]`);
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex, results, active]);

  const loadMemory = useCallback(async () => {
    try {
      const [ns, ps] = await Promise.all([brainProposals(project), brainNotes(project)]);
      setProposals(ns);
      setNotes(ps);
    } catch (e) {
      console.error("brain memory load failed:", e);
    }
  }, [project]);

  useEffect(() => {
    if (mode === "memory") void loadMemory();
  }, [mode, loadMemory]);

  const copyPath = (path: string) => {
    void navigator.clipboard?.writeText(path);
  };

  const addProject = async () => {
    const path = addPath.trim();
    if (!path) return;
    try {
      await brainAddProject(path);
      setAddPath("");
      setShowAdd(false);
      setProjects(await brainListProjects());
    } catch (e) {
      console.error("brain_add_project failed:", e);
    }
  };

  const runDoctor = () => {
    void brainDoctor(project, today());
    setTimeout(() => void loadMemory(), ASYNC_SETTLE_MS);
  };

  const resolve = (p: MemoryProposal, reject: boolean) => {
    void brainResolveProposal(p.project, p.signature, reject);
    // optimistic removal; reconcile shortly after the worker applies it
    setProposals((prev) => prev.filter((x) => x.signature !== p.signature));
    setTimeout(() => void loadMemory(), ASYNC_SETTLE_MS);
  };

  return (
    <div className="flex h-full flex-col">
      {/* status bar */}
      <div className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5">
        <span className="text-xs font-medium">Brain</span>
        <div className="flex items-center gap-0.5 rounded bg-muted/50 p-0.5 text-[11px]">
          {(["search", "memory"] as Mode[]).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              className={cn(
                "rounded px-1.5 py-0.5 capitalize",
                mode === m ? "bg-background text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              {m}
            </button>
          ))}
        </div>
        <span className="truncate text-[11px] text-muted-foreground">{statusLabel(report)}</span>
        <button
          type="button"
          onClick={() => void brainRescan(project)}
          className="ml-auto rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Reconcile the index (add/change/delete)"
        >
          Rescan
        </button>
      </div>

      {/* project filter + add (shared) */}
      <div className="flex shrink-0 items-center gap-2 px-2 pt-1.5">
        {projects.length > 1 ? (
          <select
            value={project ?? ""}
            onChange={(e) => setProject(e.target.value || null)}
            className="h-7 flex-1 rounded border bg-transparent px-1.5 text-[11px] text-foreground/80"
            title="Filter by project"
          >
            <option value="">All projects</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        ) : (
          <span className="flex-1 truncate text-[11px] text-muted-foreground">
            {projects.length === 1 ? projects[0].name : "No project indexed"}
          </span>
        )}
        <button
          type="button"
          onClick={() => setShowAdd((v) => !v)}
          className="rounded border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Add a project folder"
        >
          + Add
        </button>
      </div>
      {showAdd ? (
        <div className="flex shrink-0 items-center gap-1.5 px-2 pt-1.5">
          <Input
            value={addPath}
            onChange={(e) => setAddPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void addProject();
              }
            }}
            placeholder="Absolute path to a project folder"
            className="h-7 flex-1 text-xs"
          />
          <button
            type="button"
            onClick={() => void addProject()}
            className="rounded border px-2 py-0.5 text-[11px] hover:bg-accent"
          >
            Add
          </button>
        </div>
      ) : null}

      {mode === "search" ? (
        <SearchView
          query={query}
          setQuery={setQuery}
          results={results}
          searching={searching}
          active={active}
          selectedIndex={selectedIndex}
          setSelectedIndex={setSelectedIndex}
          inputRef={inputRef}
          scrollRef={scrollRef}
          lastKeyboardNavAt={lastKeyboardNavAt}
          onCopy={copyPath}
        />
      ) : (
        <MemoryView
          notes={notes}
          proposals={proposals}
          onRunDoctor={runDoctor}
          onResolve={resolve}
        />
      )}
    </div>
  );
}

type SearchViewProps = {
  query: string;
  setQuery: (v: string) => void;
  results: Hit[];
  searching: boolean;
  active: boolean;
  selectedIndex: number;
  setSelectedIndex: (fn: (prev: number) => number) => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
  scrollRef: React.RefObject<HTMLDivElement | null>;
  lastKeyboardNavAt: React.MutableRefObject<number>;
  onCopy: (path: string) => void;
};

function SearchView({
  query,
  setQuery,
  results,
  searching,
  active,
  selectedIndex,
  setSelectedIndex,
  inputRef,
  scrollRef,
  lastKeyboardNavAt,
  onCopy,
}: SearchViewProps) {
  return (
    <>
      <div className="relative flex shrink-0 items-center gap-2 px-2 py-1.5">
        <div className="relative flex-1">
          <HugeiconsIcon
            icon={Search01Icon}
            size={13}
            strokeWidth={2}
            className="absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (results.length === 0) return;
              if (e.key === "ArrowDown") {
                e.preventDefault();
                lastKeyboardNavAt.current = Date.now();
                setSelectedIndex((prev) => (prev + 1) % results.length);
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                lastKeyboardNavAt.current = Date.now();
                setSelectedIndex((prev) => (prev - 1 + results.length) % results.length);
              } else if (e.key === "Enter") {
                e.preventDefault();
                onCopy(results[selectedIndex].path);
              }
            }}
            placeholder="Search code & notes…"
            className="h-7 pr-7 pl-6.5 text-xs"
          />
          {query ? (
            <button
              type="button"
              onClick={() => setQuery("")}
              className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
              aria-label="Clear search"
            >
              <HugeiconsIcon icon={Cancel01Icon} size={11} strokeWidth={2} />
            </button>
          ) : null}
        </div>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="py-1" ref={scrollRef}>
          {!active ? (
            <div className="px-3 py-2 text-[11px] text-muted-foreground">
              Type to search the indexed workspace.
            </div>
          ) : searching && results.length === 0 ? (
            <div className="px-3 py-2 text-[11px] text-muted-foreground">Searching…</div>
          ) : results.length === 0 ? (
            <div className="px-3 py-2 text-[11px] text-muted-foreground">No matches</div>
          ) : (
            results.map((hit, index) => (
              <button
                key={`${hit.project} ${hit.path}`}
                type="button"
                data-index={index}
                onClick={() => onCopy(hit.path)}
                onMouseEnter={() => {
                  if (Date.now() - lastKeyboardNavAt.current > 250) {
                    setSelectedIndex(() => index);
                  }
                }}
                className={cn(
                  "flex w-full items-center gap-1.5 px-2 py-1 text-left text-xs transition-colors",
                  index === selectedIndex
                    ? "bg-accent text-foreground"
                    : "text-foreground/80 hover:bg-accent/50",
                )}
                title={`${hit.path} — click to copy path`}
              >
                <span className="truncate">{basename(hit.path)}</span>
                <span className="ml-auto truncate text-[10px] text-muted-foreground">{hit.path}</span>
              </button>
            ))
          )}
        </div>
      </ScrollArea>
    </>
  );
}

type MemoryViewProps = {
  notes: NoteSummary[];
  proposals: MemoryProposal[];
  onRunDoctor: () => void;
  onResolve: (p: MemoryProposal, reject: boolean) => void;
};

function MemoryView({ notes, proposals, onRunDoctor, onResolve }: MemoryViewProps) {
  return (
    <ScrollArea className="min-h-0 flex-1">
      <div className="flex flex-col gap-3 p-2">
        {/* Review inbox */}
        <section>
          <div className="mb-1 flex items-center gap-2">
            <h3 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Review inbox ({proposals.length})
            </h3>
            <button
              type="button"
              onClick={onRunDoctor}
              className="ml-auto rounded border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
              title="Run the memory doctor"
            >
              Run doctor
            </button>
          </div>
          {proposals.length === 0 ? (
            <div className="px-1 py-1 text-[11px] text-muted-foreground">No pending proposals.</div>
          ) : (
            <div className="flex flex-col gap-1.5">
              {proposals.map((p) => (
                <div key={`${p.project} ${p.signature}`} className="rounded border p-2 text-xs">
                  <div className="flex items-center gap-1.5">
                    <span className="rounded bg-muted px-1 py-0.5 text-[10px] uppercase text-muted-foreground">
                      {p.action}
                    </span>
                    <span className="truncate font-medium">{p.title}</span>
                  </div>
                  <p className="mt-1 text-[11px] text-muted-foreground">{p.detail}</p>
                  <div className="mt-1.5 flex gap-1.5">
                    <button
                      type="button"
                      onClick={() => onResolve(p, false)}
                      className="rounded border px-1.5 py-0.5 text-[11px] hover:bg-accent"
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      onClick={() => onResolve(p, true)}
                      className="rounded border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent"
                    >
                      Reject
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Notes */}
        <section>
          <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            Notes ({notes.length})
          </h3>
          {notes.length === 0 ? (
            <div className="px-1 py-1 text-[11px] text-muted-foreground">
              No memory notes yet. Add markdown to a project's .koden-memory/ folder.
            </div>
          ) : (
            <div className="flex flex-col gap-1">
              {notes.map((n) => (
                <div key={`${n.path}`} className="rounded border px-2 py-1 text-xs">
                  <div className="flex items-center gap-1.5">
                    {n.note_type ? (
                      <span className="rounded bg-muted px-1 py-0.5 text-[10px] uppercase text-muted-foreground">
                        {n.note_type}
                      </span>
                    ) : null}
                    <span className="truncate font-medium">{n.title}</span>
                    {n.status ? (
                      <span className="ml-auto text-[10px] text-muted-foreground">{n.status}</span>
                    ) : null}
                  </div>
                  <span className="text-[10px] text-muted-foreground">{n.path}</span>
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </ScrollArea>
  );
}
