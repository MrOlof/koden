import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { brainSearch, type Hit } from "@/modules/brain/lib/bindings";
import {
  BookOpen01Icon,
  Cancel01Icon,
  RefreshIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useMemo, useState } from "react";
import {
  type PageRef,
  pageFromNote,
  type Shelf,
  useLibrary,
} from "./lib/useLibrary";
import { NotePage } from "./NotePage";
import { ShelfRail } from "./ShelfRail";

const MIN_QUERY_LEN = 2;
const DEBOUNCE_MS = 300;
const MEMORY_PREFIX = ".koden-memory/";

function basename(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(i + 1) : p;
}

/** Resolve a search hit against the shelves: known notes keep their parsed
 *  meta; an unlisted memory file still opens, titled by its stem. */
function pageFromHit(shelves: Shelf[], hit: Hit): PageRef | null {
  const shelf = shelves.find((s) => s.project.id === hit.project);
  if (!shelf) return null;
  const note = shelf.notes.find((n) => n.path === hit.path);
  if (note) return pageFromNote(shelf, note);
  return {
    project: shelf.project,
    path: hit.path,
    title: basename(hit.path).replace(/\.md$/i, ""),
    noteType: null,
    status: null,
    anchors: [],
  };
}

/**
 * The Library: a read-only wiki of the Librarian's mind. Shelves per project
 * on the left, one note rendered as a page on the right, search across every
 * project's memory on top. Notes stay editable as plain files via the explorer.
 */
export function LibraryPane() {
  const { shelves, error, reload } = useLibrary();
  const [page, setPage] = useState<PageRef | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [searching, setSearching] = useState(false);

  const active = query.trim().length >= MIN_QUERY_LEN;

  // Debounced search, filtered to memory paths (alive-flag cancel like BrainPane).
  useEffect(() => {
    const q = query.trim();
    if (q.length < MIN_QUERY_LEN) {
      setHits([]);
      setSearching(false);
      return;
    }
    setSearching(true);
    let alive = true;
    const handle = setTimeout(async () => {
      try {
        const all = await brainSearch(q, null, 60);
        if (alive) setHits(all.filter((h) => h.path.startsWith(MEMORY_PREFIX)));
      } catch (e) {
        if (alive) {
          console.error("brain_search failed:", e);
          setHits([]);
        }
      } finally {
        if (alive) setSearching(false);
      }
    }, DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(handle);
    };
  }, [query]);

  const totalNotes = useMemo(
    () => (shelves ?? []).reduce((acc, s) => acc + s.notes.length, 0),
    [shelves],
  );

  const openHit = (hit: Hit) => {
    if (!shelves) return;
    const ref = pageFromHit(shelves, hit);
    if (!ref) return;
    setPage(ref);
    setQuery("");
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5">
        <HugeiconsIcon
          icon={BookOpen01Icon}
          size={14}
          strokeWidth={2}
          className="shrink-0 text-muted-foreground"
        />
        <span className="font-mono text-xs font-medium tracking-widest uppercase">
          Library
        </span>
        <div className="relative ml-1 w-72 max-w-[45%]">
          <HugeiconsIcon
            icon={Search01Icon}
            size={12}
            strokeWidth={2}
            className="absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search the shelves…"
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
        <span className="ml-auto truncate font-mono text-[10px] text-muted-foreground tabular-nums">
          {shelves === null
            ? ""
            : `${totalNotes} ${totalNotes === 1 ? "note" : "notes"} · ${shelves.length} ${shelves.length === 1 ? "shelf" : "shelves"}`}
        </span>
        <button
          type="button"
          onClick={() => void reload()}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Reload the shelves"
          aria-label="Reload the shelves"
        >
          <HugeiconsIcon icon={RefreshIcon} size={12} strokeWidth={2} />
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="flex w-64 shrink-0 flex-col border-r">
          <ShelfRail
            shelves={shelves}
            error={error}
            selected={page}
            onOpen={setPage}
          />
        </div>
        <div className="flex min-h-0 flex-1 flex-col">
          {active ? (
            <SearchResults
              hits={hits}
              searching={searching}
              shelves={shelves ?? []}
              onOpen={openHit}
            />
          ) : page ? (
            <NotePage page={page} />
          ) : (
            <EmptyState hasNotes={totalNotes > 0} />
          )}
        </div>
      </div>
    </div>
  );
}

function SearchResults({
  hits,
  searching,
  shelves,
  onOpen,
}: {
  hits: Hit[];
  searching: boolean;
  shelves: Shelf[];
  onOpen: (hit: Hit) => void;
}) {
  const nameOf = (projectId: string) =>
    shelves.find((s) => s.project.id === projectId)?.project.name ?? projectId;
  return (
    <ScrollArea className="min-h-0 flex-1">
      <div className="py-1">
        {searching && hits.length === 0 ? (
          <div className="px-3 py-2 text-[11px] text-muted-foreground">
            Searching the stacks…
          </div>
        ) : hits.length === 0 ? (
          <div className="px-3 py-2 text-[11px] text-muted-foreground">
            Nothing on these shelves.
          </div>
        ) : (
          hits.map((hit) => {
            const note = shelves
              .find((s) => s.project.id === hit.project)
              ?.notes.find((n) => n.path === hit.path);
            return (
              <button
                key={`${hit.project}:${hit.path}`}
                type="button"
                onClick={() => onOpen(hit)}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-foreground/80 hover:bg-accent/50"
                title={hit.path}
              >
                <span className="truncate">
                  {note?.title ?? basename(hit.path).replace(/\.md$/i, "")}
                </span>
                <span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">
                  {nameOf(hit.project)}
                </span>
              </button>
            );
          })
        )}
      </div>
    </ScrollArea>
  );
}

function EmptyState({ hasNotes }: { hasNotes: boolean }) {
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="flex max-w-xs flex-col items-center text-center">
        <HugeiconsIcon
          icon={BookOpen01Icon}
          size={22}
          strokeWidth={1.5}
          className="text-muted-foreground/60"
        />
        <p className="mt-3 text-xs text-muted-foreground">
          {hasNotes
            ? "Pick a note from the shelves."
            : "No notes yet. The Librarian shelves what your sessions teach it."}
        </p>
      </div>
    </div>
  );
}
