import {
  brainListProjects,
  brainMemoryChanges,
  brainNotes,
  type MemoryChange,
  type NoteSummary,
  type Project,
} from "@/modules/brain/lib/bindings";
import { useCallback, useEffect, useState } from "react";

/** One project's shelf: its notes plus the Librarian's recent decisions. */
export type Shelf = {
  project: Project;
  notes: NoteSummary[];
  changes: MemoryChange[];
  lastActivityMs: number | null;
};

/** A selected note page. `path` stays project-root-relative (forward-slash),
 *  exactly as the index reports it; joining happens at read time. */
export type PageRef = {
  project: Project;
  path: string;
  title: string;
  noteType: string | null;
  status: string | null;
  anchors: string[];
};

/** One fetch covers every project's recent decisions; grouped client-side. */
const CHANGES_SCAN_LIMIT = 200;

export function joinRoot(root: string, rel: string): string {
  return `${root.replace(/[\\/]+$/, "")}/${rel.replace(/^[\\/]+/, "")}`;
}

export function pageFromNote(shelf: Shelf, note: NoteSummary): PageRef {
  return {
    project: shelf.project,
    path: note.path,
    title: note.title,
    noteType: note.note_type,
    status: note.status,
    anchors: note.anchors,
  };
}

/** Day-grained stamp for shelf activity ("today", "3d ago", "Jun 30"). */
export function fmtDay(ms: number | null): string {
  if (!ms) return "";
  const days = Math.floor((Date.now() - ms) / 86_400_000);
  if (days <= 0) return "today";
  if (days < 30) return `${days}d ago`;
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function fmtAgo(ms: number | null): string {
  if (!ms) return "";
  const d = Date.now() - ms;
  if (d < 60_000) return "just now";
  const m = Math.floor(d / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function useLibrary() {
  const [shelves, setShelves] = useState<Shelf[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [projects, changes] = await Promise.all([
        brainListProjects(),
        brainMemoryChanges(null, CHANGES_SCAN_LIMIT),
      ]);
      // One degraded project must not blank the whole library.
      const notes = await Promise.all(
        projects.map((p) => brainNotes(p.id).catch(() => [] as NoteSummary[])),
      );
      const byProject = new Map<string, MemoryChange[]>();
      for (const ch of changes) {
        const list = byProject.get(ch.project);
        if (list) list.push(ch);
        else byProject.set(ch.project, [ch]);
      }
      setShelves(
        projects.map((project, i) => {
          const chs = byProject.get(project.id) ?? [];
          const last = chs.reduce(
            (acc, c) => Math.max(acc, c.applied_ms ?? 0, c.reverted_ms ?? 0),
            0,
          );
          return {
            project,
            notes: notes[i],
            changes: chs,
            lastActivityMs: last > 0 ? last : null,
          };
        }),
      );
      setError(null);
    } catch (e) {
      setError(String(e));
      setShelves((prev) => prev ?? []);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { shelves, error, reload };
}
