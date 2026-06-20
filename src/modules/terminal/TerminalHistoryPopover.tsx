import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  ArrowDown01Icon,
  ArrowUp01Icon,
  ListViewIcon,
  SearchAreaIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CommandMark, CommandStatus } from "./lib/commandMarks";
import {
  getCommandMarksForLeaf,
  getSearchAddonForLeaf,
  scrollToCommandForLeaf,
  subscribeCommandsForLeaf,
} from "./lib/useTerminalSession";

// The pane-header replacement for the old in-terminal minimap strip: a
// command-palette-style popover with two modes, switched by a slim toggle row
// at the top:
//
//   • "Inputs" (default) — lists every INPUT the user made in this terminal:
//     shell commands (OSC 133) AND Claude user turns (OSC 777). Clicking a row
//     scrolls the live terminal to that buffer line. This is the original,
//     unchanged behaviour.
//   • "Find in terminal" — full-text search over the ENTIRE scrollback
//     (command + AI OUTPUT included) via this leaf's xterm SearchAddon,
//     highlighting matches in the grid and stepping through them next/prev with
//     an "n / m" counter.
//
// Inputs mode pulls the same CommandMarks data the strip consumed, via the
// leaf-keyed module accessors (the header has a leafId, not the session
// object). Sort order is MOST-RECENT-FIRST: the collector appends in buffer
// order, so the newest mark is last; we reverse it so the latest input sits at
// the top — the thing you most likely want to jump back to.

type Mode = "inputs" | "find";

const MODE_STORAGE_KEY = "koden.terminalSearch.mode";

function loadInitialMode(): Mode {
  // ponytail: best-effort localStorage; any failure just defaults to "inputs".
  try {
    return localStorage.getItem(MODE_STORAGE_KEY) === "find"
      ? "find"
      : "inputs";
  } catch {
    return "inputs";
  }
}

// Visible match highlight: a warm yellow wash for all matches and a stronger,
// more saturated amber for the active match — mirrors the block-search accent
// (rgba(255,193,84,…)) but as the #RRGGBB the SearchAddon decorations require.
// The overview-ruler keys are mandatory in the addon's option type.
const FIND_DECORATIONS = {
  matchBackground: "#5a4a1f",
  matchOverviewRuler: "#ffc154",
  activeMatchBackground: "#d18616",
  activeMatchBorder: "#ffc154",
  activeMatchColorOverviewRuler: "#ffc154",
} as const;

type Props = {
  /** The leaf whose command history this popover shows. */
  leafId: number;
  /** Close the controlled popover after a row is chosen. */
  onClose: () => void;
};

function truncate(text: string, max = 140): string {
  const t = text.trim();
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

// A small leading dot communicating command outcome; turns get an accent glyph
// instead (rendered by the caller), so this only covers shell statuses.
function statusDotClass(status: CommandStatus): string {
  switch (status) {
    case "running":
      return "bg-primary animate-pulse";
    case "fail":
      return "bg-destructive";
    case "ok":
      return "bg-muted-foreground/50";
    default:
      return "bg-muted-foreground/50";
  }
}

export function TerminalHistoryPopover({ leafId, onClose }: Props) {
  const [mode, setMode] = useState<Mode>(loadInitialMode);

  const setModePersisted = useCallback((next: Mode) => {
    setMode(next);
    try {
      localStorage.setItem(MODE_STORAGE_KEY, next);
    } catch {
      // ponytail: non-fatal — mode just won't persist across sessions.
    }
  }, []);

  return (
    <div className="flex flex-col">
      <ModeToggle mode={mode} onChange={setModePersisted} />
      {mode === "inputs" ? (
        <InputsMode leafId={leafId} onClose={onClose} />
      ) : (
        <FindMode leafId={leafId} />
      )}
    </div>
  );
}

// Slim segmented control above the input. Two equal segments; the active one
// gets the muted "pressed" surface. Line icons only, no marks.
function ModeToggle({
  mode,
  onChange,
}: {
  mode: Mode;
  onChange: (m: Mode) => void;
}) {
  const seg = (m: Mode, label: string, icon: typeof ListViewIcon) => (
    <button
      type="button"
      aria-pressed={mode === m}
      onClick={() => onChange(m)}
      className={cn(
        "flex flex-1 items-center justify-center gap-1.5 rounded-xl px-2 py-1 text-[12px] font-medium transition-colors",
        mode === m
          ? "bg-muted text-foreground"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      <HugeiconsIcon icon={icon} size={13} strokeWidth={1.75} />
      {label}
    </button>
  );
  return (
    <div className="mb-1 flex items-center gap-1 rounded-2xl bg-input/40 p-0.5">
      {seg("inputs", "Inputs", ListViewIcon)}
      {seg("find", "Find in terminal", SearchAreaIcon)}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Inputs mode — UNCHANGED behaviour: curated command/turn list, most-recent
// first, substring filter, click → scrollToCommandForLeaf.
// ---------------------------------------------------------------------------
function InputsMode({ leafId, onClose }: Props) {
  const [marks, setMarks] = useState<CommandMark[]>([]);
  const [query, setQuery] = useState("");

  // Pull the current marks and re-pull on every CommandMarks change. The
  // subscription is rAF-coalesced upstream, so this re-reads at most once a
  // frame while the popover is open.
  useEffect(() => {
    const update = () => setMarks(getCommandMarksForLeaf(leafId)?.marks ?? []);
    update();
    return subscribeCommandsForLeaf(leafId, update);
  }, [leafId]);

  // Most-recent-first + case-insensitive substring filter on the mark text.
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const ordered = marks.slice().reverse();
    if (!q) return ordered;
    return ordered.filter((m) => m.text.toLowerCase().includes(q));
  }, [marks, query]);

  const select = (mark: CommandMark) => {
    scrollToCommandForLeaf(leafId, mark.line);
    onClose();
  };

  return (
    // shouldFilter=false: we own the ordering (most-recent-first) and the
    // substring filter, so cmdk only handles keyboard nav + highlight. With a
    // pre-filtered list the first row is auto-highlighted, so Enter selects the
    // first match. Esc is handled by the Popover wrapper.
    <Command shouldFilter={false} className="bg-transparent p-0">
      <CommandInput
        autoFocus
        value={query}
        onValueChange={setQuery}
        placeholder="Search terminal history…"
      />
      <CommandList className="mt-1 max-h-80">
        <CommandEmpty className="text-muted-foreground">
          {marks.length === 0 ? "No commands yet" : "No matches"}
        </CommandEmpty>
        {rows.map((mark) => {
          const isTurn = mark.status === "turn";
          return (
            <CommandItem
              key={mark.id}
              // cmdk matches on `value`; with shouldFilter=false it's only used
              // as the item identity, so the id keeps rows stable + selectable.
              value={String(mark.id)}
              onSelect={() => select(mark)}
              className="items-start gap-2.5"
            >
              {isTurn ? (
                // Claude user turn ("your question"): an accent "›" glyph so it
                // reads as a message, not a shell command.
                <span
                  aria-hidden
                  className="mt-px shrink-0 font-medium text-primary"
                >
                  ›
                </span>
              ) : (
                <span
                  aria-hidden
                  className={cn(
                    "mt-1.5 size-1.5 shrink-0 rounded-full",
                    statusDotClass(mark.status),
                  )}
                />
              )}
              <span
                className={cn(
                  "min-w-0 flex-1 truncate",
                  isTurn
                    ? "text-foreground"
                    : "font-mono text-[12px] text-muted-foreground",
                )}
                title={mark.text}
              >
                {truncate(mark.text)}
              </span>
            </CommandItem>
          );
        })}
      </CommandList>
    </Command>
  );
}

// ---------------------------------------------------------------------------
// Find mode — full-scrollback search via this leaf's SearchAddon.
// ---------------------------------------------------------------------------
function FindMode({ leafId }: { leafId: number }) {
  const [query, setQuery] = useState("");
  // resultCount === -1 sentinel: addon not ready / no search run yet.
  const [results, setResults] = useState<{ index: number; count: number }>({
    index: -1,
    count: -1,
  });
  const inputRef = useRef<HTMLInputElement>(null);
  const addon = getSearchAddonForLeaf(leafId);
  const ready = addon !== null;

  // Subscribe to result changes for the "n / m" counter, and tear everything
  // down on unmount / mode-switch: dispose the listener AND clear the grid
  // decorations so highlights never linger after Find mode goes away.
  useEffect(() => {
    if (!addon) return;
    const sub = addon.onDidChangeResults(({ resultIndex, resultCount }) => {
      setResults({ index: resultIndex, count: resultCount });
    });
    inputRef.current?.focus();
    return () => {
      sub.dispose();
      addon.clearDecorations();
      setResults({ index: -1, count: -1 });
    };
  }, [addon]);

  // Live incremental highlight on every keystroke: highlights all matches and
  // jumps to the first. Empty query clears decorations and resets the counter.
  const runIncremental = useCallback(
    (next: string) => {
      if (!addon) return;
      if (next) {
        addon.findNext(next, {
          incremental: true,
          decorations: FIND_DECORATIONS,
        });
      } else {
        addon.clearDecorations();
        setResults({ index: -1, count: -1 });
      }
    },
    [addon],
  );

  const step = useCallback(
    (forward: boolean) => {
      if (!addon || !query) return;
      const opts = { decorations: FIND_DECORATIONS };
      if (forward) addon.findNext(query, opts);
      else addon.findPrevious(query, opts);
      // Keep focus in the input while stepping with the arrow buttons.
      inputRef.current?.focus();
    },
    [addon, query],
  );

  // "n / m": resultIndex is 0-based and -1 when the highlight threshold is
  // exceeded; show "1000+" semantics gracefully by only adding 1 when valid.
  const counter = useMemo(() => {
    if (results.count <= 0) return "";
    const n = results.index >= 0 ? results.index + 1 : "—";
    return `${n} / ${results.count}`;
  }, [results]);

  const noMatches = ready && query.trim() !== "" && results.count === 0;

  return (
    <div className="flex flex-col">
      <div className="relative px-1 pt-1">
        <Input
          ref={inputRef}
          autoFocus
          value={query}
          disabled={!ready}
          placeholder={ready ? "Find in terminal output…" : "Search not ready…"}
          className="h-9 bg-input/50 pr-24 text-[13px]! focus-visible:ring-0"
          onChange={(e) => {
            const next = e.target.value;
            setQuery(next);
            runIncremental(next);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              step(!e.shiftKey);
            } else if (e.key === "ArrowDown") {
              e.preventDefault();
              step(true);
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              step(false);
            }
          }}
        />
        <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-1">
          {counter ? (
            <span className="mr-0.5 select-none text-[11px] tabular-nums text-muted-foreground">
              {counter}
            </span>
          ) : null}
          <button
            type="button"
            aria-label="Previous match"
            title="Previous match (Shift+Enter / ↑)"
            disabled={!ready || !query}
            onClick={() => step(false)}
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <HugeiconsIcon icon={ArrowUp01Icon} size={13} strokeWidth={2} />
          </button>
          <button
            type="button"
            aria-label="Next match"
            title="Next match (Enter / ↓)"
            disabled={!ready || !query}
            onClick={() => step(true)}
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <HugeiconsIcon icon={ArrowDown01Icon} size={13} strokeWidth={2} />
          </button>
        </div>
      </div>
      <p className="px-3 pt-2 pb-1 text-[12px] text-muted-foreground">
        {!ready
          ? "This terminal isn't ready to search yet."
          : query.trim() === ""
            ? "Search the full scrollback, including command and AI output."
            : noMatches
              ? "No matches in scrollback."
              : "Enter / ↓ next · Shift+Enter / ↑ previous"}
      </p>
    </div>
  );
}
