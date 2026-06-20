import type { Tab } from "@/modules/tabs";
import { labelFor } from "@/modules/tabs";
import type { Agent, AgentStatus } from "./types";

/** A bucket of root agents that all belong to the same tab. */
export type TabGroup = {
  /** Owning tab id, or null for roots with no/unknown tab ("Other"). */
  tabId: number | null;
  /** Display title: the tab's label, or "Other" for the fallback bucket. */
  title: string;
  agents: Agent[];
};

/**
 * Buckets root agents by their owning tab, ordered to match the `tabs` array.
 *
 * Roots arrive pre-sorted (by sortAgentsForDock) and that order is preserved
 * within each group — we never re-sort. Groups follow tab order; any root whose
 * tabId is null or doesn't match a known tab lands in a trailing "Other" group.
 *
 * Pure: no store or React imports, so it is unit-testable in isolation.
 */
export function groupRootsByTab(roots: Agent[], tabs: Tab[]): TabGroup[] {
  const tabById = new Map<number, Tab>();
  for (const t of tabs) tabById.set(t.id, t);

  // Bucket roots by tabId, preserving incoming (pre-sorted) order per group.
  const byTab = new Map<number, Agent[]>();
  const other: Agent[] = [];
  for (const root of roots) {
    const tab = root.tabId !== null ? tabById.get(root.tabId) : undefined;
    if (root.tabId === null || !tab) {
      other.push(root);
      continue;
    }
    const bucket = byTab.get(root.tabId);
    if (bucket) bucket.push(root);
    else byTab.set(root.tabId, [root]);
  }

  // Emit groups in tabs-array order, then the trailing "Other" bucket.
  const groups: TabGroup[] = [];
  for (const t of tabs) {
    const agents = byTab.get(t.id);
    if (agents && agents.length > 0) {
      groups.push({ tabId: t.id, title: labelFor(t), agents });
    }
  }
  if (other.length > 0) {
    groups.push({ tabId: null, title: "Other", agents: other });
  }
  return groups;
}

/** Tallies how many agents sit in each status, for the group roll-up. */
export function statusCounts(
  agents: Agent[],
): Partial<Record<AgentStatus, number>> {
  const counts: Partial<Record<AgentStatus, number>> = {};
  for (const a of agents) {
    counts[a.status] = (counts[a.status] ?? 0) + 1;
  }
  return counts;
}
