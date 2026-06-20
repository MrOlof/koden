import { terminalLeaves } from "@/modules/terminal";
import type { Tab } from "@/modules/tabs";

export type TerminalAgentSeed = {
  leafId: number;
  tabId: number;
  /** cwd-derived display name, or "shell" when the cwd is unknown. */
  name: string;
};

/**
 * Pure selector for terminal→agent pre-registration. Given the current tabs and
 * the leaf ids already claimed by an agent, returns one seed per running,
 * non-note terminal leaf that still needs a node in the Agents panel.
 *
 * - Only warm (non-cold) terminal tabs count: a cold restored tab is not yet
 *   running, so it should not show as a live agent.
 * - Note panes are excluded (they are textareas, not terminals).
 * - Already-owned leaves are skipped, so an existing agent (e.g. the Director,
 *   or one already created from an OSC signal) is never double-registered.
 */
export function terminalsToRegister(
  tabs: Tab[],
  owned: ReadonlySet<number>,
): TerminalAgentSeed[] {
  const seeds: TerminalAgentSeed[] = [];
  const claimed = new Set(owned);
  for (const t of tabs) {
    if (t.kind !== "terminal" || t.cold) continue;
    for (const leaf of terminalLeaves(t.paneTree)) {
      if (claimed.has(leaf.id)) continue;
      const cwd = leaf.cwd ?? t.cwd;
      const base = cwd?.split(/[\\/]/).filter(Boolean).pop();
      seeds.push({ leafId: leaf.id, tabId: t.id, name: base || "shell" });
      claimed.add(leaf.id);
    }
  }
  return seeds;
}
