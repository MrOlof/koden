import { cn } from "@/lib/utils";
import type { Tab } from "@/modules/tabs";
import {
  ArrowLeft01Icon,
  ArrowRight01Icon,
  Delete02Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import { type BoardColumn, useDocsStore } from "./store/docsStore";

type Props = {
  tabs: Tab[];
  activeId: number;
};

export function BoardStack({ tabs, activeId }: Props) {
  const tab = tabs.find((t) => t.id === activeId && t.kind === "board");
  if (!tab || tab.kind !== "board") return null;
  return <BoardPane key={tab.boardId} boardId={tab.boardId} />;
}

function BoardPane({ boardId }: { boardId: string }) {
  const board = useDocsStore((s) => s.boards[boardId]);
  const ensureBoard = useDocsStore((s) => s.ensureBoard);

  useEffect(() => {
    ensureBoard(boardId);
  }, [boardId, ensureBoard]);

  if (!board) return null;

  return (
    <div className="flex h-full min-h-0 gap-3 overflow-x-auto p-1">
      {board.columns.map((col, i) => (
        <Column
          key={col.id}
          boardId={boardId}
          col={col}
          canMoveLeft={i > 0}
          canMoveRight={i < board.columns.length - 1}
          leftId={board.columns[i - 1]?.id ?? null}
          rightId={board.columns[i + 1]?.id ?? null}
        />
      ))}
    </div>
  );
}

function Column({
  boardId,
  col,
  canMoveLeft,
  canMoveRight,
  leftId,
  rightId,
}: {
  boardId: string;
  col: BoardColumn;
  canMoveLeft: boolean;
  canMoveRight: boolean;
  leftId: string | null;
  rightId: string | null;
}) {
  const cards = useDocsStore((s) => s.boards[boardId]?.cards ?? {});
  const addCard = useDocsStore((s) => s.addCard);
  const editCard = useDocsStore((s) => s.editCard);
  const removeCard = useDocsStore((s) => s.removeCard);
  const moveCard = useDocsStore((s) => s.moveCard);
  const renameColumn = useDocsStore((s) => s.renameColumn);
  const [draft, setDraft] = useState("");
  const [editingTitle, setEditingTitle] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (editingTitle) {
      titleRef.current?.focus();
      titleRef.current?.select();
    }
  }, [editingTitle]);

  return (
    <div className="flex w-72 shrink-0 flex-col rounded-lg border border-border/60 bg-card/40">
      <div className="flex items-center justify-between gap-2 border-b border-border/50 px-3 py-2">
        {editingTitle ? (
          <input
            ref={titleRef}
            defaultValue={col.title}
            aria-label="Column name"
            className="min-w-0 flex-1 rounded-sm bg-background px-1 text-xs font-semibold text-foreground outline-none ring-1 ring-ring"
            onBlur={(e) => {
              renameColumn(boardId, col.id, e.target.value);
              setEditingTitle(false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                renameColumn(boardId, col.id, e.currentTarget.value);
                setEditingTitle(false);
              } else if (e.key === "Escape") setEditingTitle(false);
            }}
          />
        ) : (
          <button
            type="button"
            onClick={() => setEditingTitle(true)}
            className="truncate text-left text-xs font-semibold text-foreground"
          >
            {col.title}
          </button>
        )}
        <span className="shrink-0 rounded-full bg-foreground/[0.07] px-1.5 text-[10px] font-medium tabular-nums text-muted-foreground">
          {col.cardIds.length}
        </span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2">
        {col.cardIds.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs leading-relaxed text-muted-foreground">
            No cards. Add one below.
          </p>
        ) : null}
        {col.cardIds.map((cardId) => {
          const card = cards[cardId];
          if (!card) return null;
          return (
            <CardItem
              key={cardId}
              text={card.text}
              canMoveLeft={canMoveLeft}
              canMoveRight={canMoveRight}
              onEdit={(text) => editCard(boardId, cardId, text)}
              onRemove={() => removeCard(boardId, col.id, cardId)}
              onMoveLeft={() => leftId && moveCard(boardId, cardId, leftId)}
              onMoveRight={() => rightId && moveCard(boardId, cardId, rightId)}
            />
          );
        })}
      </div>

      <div className="border-t border-border/50 p-2">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add a card..."
          aria-label={`Add card to ${col.title}`}
          className="w-full rounded-md border border-border/60 bg-background/60 px-2 py-1.5 text-xs text-foreground outline-none placeholder:text-muted-foreground/60 focus:ring-1 focus:ring-ring"
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) {
              addCard(boardId, col.id, draft);
              setDraft("");
            }
          }}
        />
      </div>
    </div>
  );
}

function CardItem({
  text,
  canMoveLeft,
  canMoveRight,
  onEdit,
  onRemove,
  onMoveLeft,
  onMoveRight,
}: {
  text: string;
  canMoveLeft: boolean;
  canMoveRight: boolean;
  onEdit: (text: string) => void;
  onRemove: () => void;
  onMoveLeft: () => void;
  onMoveRight: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const editRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (editing) {
      editRef.current?.focus();
      editRef.current?.select();
    }
  }, [editing]);

  if (editing) {
    return (
      <textarea
        ref={editRef}
        defaultValue={text}
        aria-label="Edit card"
        rows={2}
        className="w-full resize-none rounded-md border border-border/60 bg-background px-2 py-1.5 text-xs text-foreground outline-none ring-1 ring-ring"
        onBlur={(e) => {
          onEdit(e.target.value);
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onEdit(e.currentTarget.value);
            setEditing(false);
          } else if (e.key === "Escape") setEditing(false);
        }}
      />
    );
  }

  return (
    <div className="group rounded-md border border-border/60 bg-background/70 p-2 text-xs text-foreground shadow-sm">
      <button
        type="button"
        onClick={() => setEditing(true)}
        className="block w-full whitespace-pre-wrap break-words text-left"
      >
        {text}
      </button>
      <div className="mt-1.5 flex items-center justify-between opacity-0 transition-opacity group-hover:opacity-100">
        <div className="flex gap-0.5">
          <IconBtn
            label="Move left"
            disabled={!canMoveLeft}
            onClick={onMoveLeft}
            icon={ArrowLeft01Icon}
          />
          <IconBtn
            label="Move right"
            disabled={!canMoveRight}
            onClick={onMoveRight}
            icon={ArrowRight01Icon}
          />
        </div>
        <IconBtn label="Delete card" onClick={onRemove} icon={Delete02Icon} />
      </div>
    </div>
  );
}

function IconBtn({
  label,
  icon,
  onClick,
  disabled,
}: {
  label: string;
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"];
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
        disabled && "pointer-events-none opacity-30",
      )}
    >
      <HugeiconsIcon icon={icon} size={12} strokeWidth={2} />
    </button>
  );
}
