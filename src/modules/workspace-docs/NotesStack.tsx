import { cn } from "@/lib/utils";
import type { Tab } from "@/modules/tabs";
import { useDocsStore } from "./store/docsStore";

type Props = {
  tabs: Tab[];
  activeId: number;
};

/**
 * Notes / scratchpad surface. Content is the source of truth in the docs store
 * (persisted), so rendering only the active note loses no state on tab switch.
 */
export function NotesStack({ tabs, activeId }: Props) {
  const tab = tabs.find((t) => t.id === activeId && t.kind === "notes");
  if (tab?.kind !== "notes") return null;
  return <NotePane key={tab.docId} docId={tab.docId} />;
}

/**
 * A single note editor bound to a docs-store entry. Used both as a full tab
 * (NotesStack) and as a split pane inside a terminal tab (`embedded`), where
 * the surrounding pane already supplies the frame.
 */
export function NotePane({
  docId,
  embedded,
}: {
  docId: string;
  embedded?: boolean;
}) {
  const content = useDocsStore((s) => s.notes[docId]?.content ?? "");
  const setNote = useDocsStore((s) => s.setNote);
  const words = content.trim() ? content.trim().split(/\s+/).length : 0;

  return (
    <div
      className={cn(
        "flex h-full min-h-0 flex-col",
        embedded
          ? "bg-card/20"
          : "rounded-lg border border-border/60 bg-card/40",
      )}
    >
      <textarea
        value={content}
        onChange={(e) => setNote(docId, e.target.value)}
        spellCheck={false}
        placeholder="Write notes in Markdown..."
        aria-label="Notes"
        className="min-h-0 flex-1 resize-none bg-transparent p-4 font-mono text-sm leading-relaxed text-foreground outline-none placeholder:text-muted-foreground/60"
      />
      <div className="flex shrink-0 items-center justify-between border-t border-border/50 px-3 py-1.5 text-[11px] text-muted-foreground">
        <span>Markdown scratchpad</span>
        <span className="tabular-nums">
          {words} {words === 1 ? "word" : "words"}
        </span>
      </div>
    </div>
  );
}
