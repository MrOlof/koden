import { tool } from "ai";
import { z } from "zod";
import { snapshotSpaces, type SpaceInfo, type ToolContext } from "./context";

// snapshotSpaces lives in context.ts (no ai/zod imports) so the live bridge
// can use it without pulling the AI SDK eager; re-exported for callers here.
export { snapshotSpaces };

// Space tools: spaces are the header tab groups (the Spaces switcher,
// Ctrl+Shift+S). When the user says "workspace" they usually mean one of
// these. Same no-approval rationale as the layout lane: every action is
// immediately visible in the header, reversible with one click, and
// non-destructive. Create/switch/list only; delete and rename stay with the
// user in v1 (the create/arrange doctrine, ADR-017 addendum).

function describeSpace(s: SpaceInfo): string {
  const tabs = `${s.tabCount} ${s.tabCount === 1 ? "tab" : "tabs"}`;
  return `'${s.name}' (${s.id}, ${tabs}${s.active ? ", active" : ""})`;
}

function candidateList(spaces: SpaceInfo[]): string {
  return spaces.map(describeSpace).join("; ");
}

export type ResolvedSpace =
  | { ok: true; space: SpaceInfo; via: string }
  | { ok: false; error: string };

/**
 * Fuzzy space resolution, strictly tiered: space id > exact name >
 * case-insensitive name > name substring. Duplicate names are legal (the UI
 * never dedupes), so ambiguity within a tier is an error listing the
 * candidates, never a best-effort pick.
 */
export function resolveSpaceTarget(
  rawTarget: string,
  spaces: SpaceInfo[],
): ResolvedSpace {
  const target = rawTarget.trim();
  if (spaces.length === 0) return { ok: false, error: "no spaces exist" };
  if (!target)
    return {
      ok: false,
      error: `empty target. Spaces: ${candidateList(spaces)}`,
    };

  // Space id (from workspace_list_spaces): unambiguous, wins outright.
  const byId = spaces.find((s) => s.id === target);
  if (byId) return { ok: true, space: byId, via: "space-id" };

  const lower = target.toLowerCase();
  const tiers: Array<{ via: string; match: (s: SpaceInfo) => boolean }> = [
    { via: "name", match: (s) => s.name === target },
    { via: "name-ci", match: (s) => s.name.toLowerCase() === lower },
    {
      via: "name-substring",
      match: (s) => s.name.toLowerCase().includes(lower),
    },
  ];
  for (const tier of tiers) {
    const matches = spaces.filter(tier.match);
    if (matches.length === 1)
      return { ok: true, space: matches[0], via: tier.via };
    if (matches.length > 1)
      return {
        ok: false,
        error: `'${target}' is ambiguous: ${matches.length} spaces match: ${candidateList(matches)}. Target the space id instead.`,
      };
  }
  return {
    ok: false,
    error: `no space matches '${target}'. Spaces: ${candidateList(spaces)}`,
  };
}

export function buildSpaceTools(ctx: ToolContext) {
  return {
    workspace_list_spaces: tool({
      description:
        "List the workspace's spaces (the header tab groups): id, name, tab count, and which is active. Call before switching or creating when the target is loosely named. Auto-executes.",
      inputSchema: z.object({}),
      execute: async () => {
        const spaces = ctx.listSpaces();
        return {
          count: spaces.length,
          active: spaces.find((s) => s.active)?.name ?? null,
          spaces,
        };
      },
    }),

    workspace_create_space: tool({
      description:
        "Create a new space (a fresh tab group in the header) and switch to it. It opens with one terminal tab; follow with workspace_open_tab / workspace_split_pane to build the layout. When the user says 'create a workspace', they almost always mean this. Duplicate names are allowed (the UI allows them). Auto-executes (visible in the header, reversible).",
      inputSchema: z.object({
        name: z.string().min(1).describe("Name for the new space."),
      }),
      execute: async ({ name }) => {
        const trimmed = name.trim();
        if (!trimmed) return { error: "space name is empty" };
        const dup = ctx.listSpaces().some((s) => s.name === trimmed);
        const res = ctx.createSpace(trimmed);
        if ("error" in res) return res;
        return {
          ...res,
          ...(dup
            ? {
                note: "another space shares this name; target by id when switching",
              }
            : {}),
        };
      },
    }),

    workspace_switch_space: tool({
      description:
        "Switch the active space. target: a space id from workspace_list_spaces, an exact name, or a name fragment (resolved exact > case-insensitive > substring); ambiguity returns the candidates. Brings that space's tabs to the front. Auto-executes.",
      inputSchema: z.object({
        target: z
          .string()
          .min(1)
          .describe("Space id, exact name, or a name fragment."),
      }),
      execute: async ({ target }) => {
        const r = resolveSpaceTarget(target, ctx.listSpaces());
        if (!r.ok) return { error: r.error };
        const { id, name, active } = r.space;
        if (active)
          return {
            spaceId: id,
            name,
            matched_by: r.via,
            switched: false,
            note: "already the active space",
          };
        if (!ctx.switchSpace(id))
          return { error: `space '${name}' (${id}) no longer exists` };
        return { spaceId: id, name, matched_by: r.via, switched: true };
      },
    }),
  } as const;
}
