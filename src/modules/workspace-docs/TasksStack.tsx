import { cn } from "@/lib/utils";
import type { Tab } from "@/modules/tabs";
import {
  ArrowDown01Icon,
  ArrowUp01Icon,
  CheckmarkSquare02Icon,
  Delete02Icon,
  SquareIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import { type TaskItem, useDocsStore } from "./store/docsStore";

type Props = {
  tabs: Tab[];
  activeId: number;
};

/**
 * Checklist surface. A durable, tracked to-do list with clickable checkboxes,
 * persisted to the docs store (so it survives restarts like the Notes tab). The
 * list id is the source of truth; rendering only the active list loses no state
 * on tab switch.
 */
export function TasksStack({ tabs, activeId }: Props) {
  const tab = tabs.find((t) => t.id === activeId && t.kind === "tasks");
  if (tab?.kind !== "tasks") return null;
  return <TaskPane key={tab.listId} listId={tab.listId} />;
}

export function TaskPane({
  listId,
  embedded,
}: {
  listId: string;
  embedded?: boolean;
}) {
  const list = useDocsStore((s) => s.tasks[listId]);
  const ensureTaskList = useDocsStore((s) => s.ensureTaskList);
  const addTask = useDocsStore((s) => s.addTask);
  const clearCompleted = useDocsStore((s) => s.clearCompleted);
  const [draft, setDraft] = useState("");

  useEffect(() => {
    ensureTaskList(listId);
  }, [listId, ensureTaskList]);

  const items = list?.items ?? [];
  const doneCount = items.filter((t) => t.done).length;

  return (
    <div
      className={cn(
        "flex h-full min-h-0 flex-col",
        embedded
          ? "bg-card/20"
          : "rounded-lg border border-border/60 bg-card/40",
      )}
    >
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Tasks
        </span>
        <span className="text-[11px] tabular-nums text-muted-foreground">
          {doneCount}/{items.length}
        </span>
        {doneCount > 0 ? (
          <button
            type="button"
            onClick={() => clearCompleted(listId)}
            className="ml-auto rounded px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            Clear done
          </button>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {items.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs leading-relaxed text-muted-foreground">
            No tasks yet. Add one below — check it off when it's done.
          </p>
        ) : (
          items.map((item, i) => (
            <TaskRow
              key={item.id}
              listId={listId}
              item={item}
              canMoveUp={i > 0}
              canMoveDown={i < items.length - 1}
            />
          ))
        )}
      </div>

      <div className="shrink-0 border-t border-border/50 p-2">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add a task..."
          aria-label="Add a task"
          className="w-full rounded-md border border-border/60 bg-background/60 px-2.5 py-1.5 text-sm text-foreground outline-none placeholder:text-muted-foreground/60 focus:ring-1 focus:ring-ring"
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) {
              addTask(listId, draft);
              setDraft("");
            }
          }}
        />
      </div>
    </div>
  );
}

function TaskRow({
  listId,
  item,
  canMoveUp,
  canMoveDown,
}: {
  listId: string;
  item: TaskItem;
  canMoveUp: boolean;
  canMoveDown: boolean;
}) {
  const toggleTask = useDocsStore((s) => s.toggleTask);
  const editTask = useDocsStore((s) => s.editTask);
  const removeTask = useDocsStore((s) => s.removeTask);
  const moveTask = useDocsStore((s) => s.moveTask);
  const [editing, setEditing] = useState(false);
  const editRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) {
      editRef.current?.focus();
      editRef.current?.select();
    }
  }, [editing]);

  return (
    <div className="group flex items-center gap-2 rounded-md px-1.5 py-1 transition-colors hover:bg-accent/40">
      <button
        type="button"
        aria-pressed={item.done}
        aria-label={item.done ? "Mark as not done" : "Mark as done"}
        onClick={() => toggleTask(listId, item.id)}
        className={cn(
          "flex size-4 shrink-0 items-center justify-center rounded transition-colors",
          item.done ? "text-primary" : "text-muted-foreground hover:text-foreground",
        )}
      >
        <HugeiconsIcon
          icon={item.done ? CheckmarkSquare02Icon : SquareIcon}
          size={16}
          strokeWidth={1.75}
        />
      </button>

      {editing ? (
        <input
          ref={editRef}
          defaultValue={item.text}
          aria-label="Edit task"
          className="min-w-0 flex-1 rounded-sm bg-background px-1 py-0.5 text-sm text-foreground outline-none ring-1 ring-ring"
          onBlur={(e) => {
            editTask(listId, item.id, e.target.value);
            setEditing(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              editTask(listId, item.id, e.currentTarget.value);
              setEditing(false);
            } else if (e.key === "Escape") {
              setEditing(false);
            }
          }}
        />
      ) : (
        <button
          type="button"
          onClick={() => setEditing(true)}
          className={cn(
            "min-w-0 flex-1 truncate text-left text-sm",
            item.done
              ? "text-muted-foreground line-through"
              : "text-foreground",
          )}
        >
          {item.text}
        </button>
      )}

      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        <IconBtn
          label="Move up"
          disabled={!canMoveUp}
          onClick={() => moveTask(listId, item.id, -1)}
          icon={ArrowUp01Icon}
        />
        <IconBtn
          label="Move down"
          disabled={!canMoveDown}
          onClick={() => moveTask(listId, item.id, 1)}
          icon={ArrowDown01Icon}
        />
        <IconBtn
          label="Delete task"
          onClick={() => removeTask(listId, item.id)}
          icon={Delete02Icon}
        />
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
