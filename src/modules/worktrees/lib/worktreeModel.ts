import { slugify } from "./slug";

export type BranchList = {
  current: string | null;
  local: string[];
  remote: string[];
};

export type WorktreeAddPlan = {
  newBranch: string | null;
  base: string;
};

export const BRANCH_PREFIX = "feat/";
export const WORKTREES_DIR = ".koden/worktrees";

export function deriveBranch(name: string): string {
  const slug = slugify(name);
  return slug ? `${BRANCH_PREFIX}${slug}` : "";
}

export function worktreePathFor(repoRoot: string, slug: string): string {
  const root = repoRoot.replace(/\\/g, "/").replace(/\/+$/, "");
  return `${root}/${WORKTREES_DIR}/${slug}`;
}

/** Current branch first, then the other locals, then remotes. */
export function orderBases(branches: BranchList): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (name: string) => {
    if (!name || seen.has(name)) return;
    seen.add(name);
    out.push(name);
  };
  if (branches.current) push(branches.current);
  for (const b of branches.local) push(b);
  for (const b of branches.remote) push(b);
  return out;
}

/**
 * A branch field naming an existing local branch means "check it out";
 * anything else is a new branch created off the chosen base.
 */
export function planWorktreeAdd(
  branch: string,
  base: string,
  local: string[],
): WorktreeAddPlan {
  const trimmed = branch.trim();
  if (local.includes(trimmed)) return { newBranch: null, base: trimmed };
  return { newBranch: trimmed, base };
}

/**
 * Client-side mirror of the cheap parts of git check-ref-format; the Rust
 * side runs the real check, this only keeps obvious junk out of the request.
 */
export function isPlausibleBranchName(name: string): boolean {
  if (!name || name.length > 255) return false;
  if (name.startsWith("-") || name.startsWith("/") || name.endsWith("/"))
    return false;
  if (name.endsWith(".") || name.endsWith(".lock")) return false;
  if (name === "HEAD") return false;
  if (/[\s~^:?*[\\\x00-\x1f\x7f]/.test(name)) return false;
  if (name.includes("..") || name.includes("@{") || name.includes("//"))
    return false;
  return !name.split("/").some((seg) => seg.startsWith("."));
}

/** First accent index nobody uses; when all are taken, the least used one. */
export function nextFreeColorIndex(
  used: readonly (number | undefined)[],
  count: number,
): number {
  if (count <= 0) return 0;
  const tally = new Array<number>(count).fill(0);
  for (const c of used) {
    if (c != null && c >= 0 && c < count) tally[c] += 1;
  }
  let best = 0;
  for (let i = 1; i < count; i += 1) {
    if (tally[i] < tally[best]) best = i;
  }
  return best;
}

/** Comma / newline separated workspace-relative folders. Rejects escapes. */
export function parseSymlinkPaths(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of text.split(/[,\n]/)) {
    const cleaned = raw
      .trim()
      .replace(/\\/g, "/")
      .replace(/^(\.\/)+/, "")
      .replace(/^\/+|\/+$/g, "");
    if (!cleaned || seen.has(cleaned)) continue;
    if (cleaned.split("/").some((seg) => seg === "..")) continue;
    seen.add(cleaned);
    out.push(cleaned);
  }
  return out;
}

export function formatSymlinkPaths(paths: readonly string[]): string {
  return paths.join(", ");
}
