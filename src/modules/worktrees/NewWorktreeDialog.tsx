import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  type GitBranches,
  type GitRepoInfo,
  native,
} from "@/modules/ai/lib/native";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { SPACE_COLORS } from "@/modules/spaces/lib/spaceColor";
import { useSpaces } from "@/modules/spaces/lib/useSpaces";
import { currentWorkspaceEnv } from "@/modules/workspace";
import { GitBranchIcon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { slugify } from "./lib/slug";
import {
  deriveBranch,
  isPlausibleBranchName,
  nextFreeColorIndex,
  orderBases,
  planWorktreeAdd,
  worktreePathFor,
} from "./lib/worktreeModel";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Any directory inside the repository the worktree should branch from. */
  cwd: string | null;
  onCreated: (spaceId: string) => void;
};

type Phase =
  | { kind: "loading" }
  | { kind: "no-repo" }
  | { kind: "error"; message: string }
  | { kind: "ready"; repo: GitRepoInfo; branches: GitBranches };

export function NewWorktreeDialog({
  open,
  onOpenChange,
  cwd,
  onCreated,
}: Props) {
  const symlinkPaths = usePreferencesStore((s) => s.worktreeSymlinkPaths);
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [name, setName] = useState("");
  const [branchDraft, setBranchDraft] = useState<string | null>(null);
  const [base, setBase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setPhase({ kind: "loading" });
    setName("");
    setBranchDraft(null);
    setBase("");
    setBusy(false);
    setError(null);
    let cancelled = false;
    void (async () => {
      if (!cwd) {
        if (!cancelled) setPhase({ kind: "no-repo" });
        return;
      }
      try {
        const repo = await native.gitResolveRepo(cwd);
        if (!repo) {
          if (!cancelled) setPhase({ kind: "no-repo" });
          return;
        }
        const branches = await native.gitBranches(repo.repoRoot);
        if (cancelled) return;
        setBase(orderBases(branches)[0] ?? "");
        setPhase({ kind: "ready", repo, branches });
        setTimeout(() => nameRef.current?.focus(), 0);
      } catch (e) {
        if (!cancelled) setPhase({ kind: "error", message: String(e) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, cwd]);

  const ready = phase.kind === "ready" ? phase : null;
  const slug = slugify(name);
  const branch = branchDraft ?? deriveBranch(name);
  const bases = ready ? orderBases(ready.branches) : [];
  const location = ready
    ? worktreePathFor(ready.repo.repoRoot, slug || "<name>")
    : "";
  const branchOk = isPlausibleBranchName(branch);
  const canCreate = !!ready && !busy && !!slug && branchOk && !!base;

  const submit = async () => {
    if (!ready || !canCreate) return;
    setBusy(true);
    setError(null);
    const repoRoot = ready.repo.repoRoot;
    const env = currentWorkspaceEnv();
    try {
      const plan = planWorktreeAdd(branch, base, ready.branches.local);
      const wt = await native.gitWorktreeAdd(
        repoRoot,
        worktreePathFor(repoRoot, slug),
        plan.newBranch,
        plan.base,
      );
      if (env.kind === "local" && symlinkPaths.length > 0) {
        await linkFolders(repoRoot, wt.path, symlinkPaths);
      }
      const { spaces, create, setActive } = useSpaces.getState();
      const meta = create({
        name: name.trim(),
        root: wt.path,
        env,
        color: nextFreeColorIndex(
          spaces.map((s) => s.color),
          SPACE_COLORS.length,
        ),
        worktree: { repoRoot, branch: wt.branch ?? branch },
      });
      setActive(meta.id);
      onCreated(meta.id);
      toast.success(`Worktree ready on ${wt.branch ?? branch}`);
      onOpenChange(false);
    } catch (e) {
      const message = String(e);
      setError(message);
      toast.error("Could not create worktree", { description: message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={busy ? undefined : onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex gap-1.75">
            <HugeiconsIcon icon={GitBranchIcon} size={16} strokeWidth={1.75} />
            New worktree Space
          </DialogTitle>
          <DialogDescription>
            Check out a branch into its own folder and open it as a Space. The
            main checkout stays untouched.
          </DialogDescription>
        </DialogHeader>

        {phase.kind === "loading" && (
          <div className="text-xs text-muted-foreground">
            Reading repository...
          </div>
        )}
        {phase.kind === "no-repo" && (
          <div className="text-xs text-muted-foreground">
            The current folder is not inside a git repository.
          </div>
        )}
        {phase.kind === "error" && (
          <div className="text-xs text-destructive">{phase.message}</div>
        )}

        {ready && (
          <div className="flex flex-col gap-3">
            <Field label="Name" htmlFor="worktree-name">
              <Input
                id="worktree-name"
                ref={nameRef}
                value={name}
                disabled={busy}
                placeholder="Fix login redirect"
                onChange={(e) => {
                  setName(e.target.value);
                  setError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void submit();
                  }
                }}
              />
            </Field>
            <Field label="Branch" htmlFor="worktree-branch">
              <Input
                id="worktree-branch"
                value={branch}
                disabled={busy}
                spellCheck={false}
                placeholder="feat/fix-login-redirect"
                aria-invalid={branch.length > 0 && !branchOk}
                onChange={(e) => {
                  setBranchDraft(e.target.value);
                  setError(null);
                }}
                className="font-mono text-xs"
              />
              <span className="text-[11px] text-muted-foreground">
                {ready.branches.local.includes(branch.trim())
                  ? "Existing branch: it will be checked out as is."
                  : "A new branch created off the base below."}
              </span>
            </Field>
            <Field label="Base" htmlFor="worktree-base">
              <Select value={base} onValueChange={setBase} disabled={busy}>
                <SelectTrigger
                  id="worktree-base"
                  className="w-full font-mono text-xs"
                >
                  <SelectValue placeholder="Pick a branch" />
                </SelectTrigger>
                <SelectContent>
                  {bases.map((b) => (
                    <SelectItem key={b} value={b} className="font-mono text-xs">
                      {b}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field label="Location">
              <span className="truncate font-mono text-[11px] text-muted-foreground">
                {location}
              </span>
            </Field>
            {error && <div className="text-xs text-destructive">{error}</div>}
          </div>
        )}

        <DialogFooter>
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button disabled={!canCreate} onClick={() => void submit()}>
            {busy ? "Creating..." : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={htmlFor} className="text-xs">
        {label}
      </Label>
      {children}
    </div>
  );
}

// Linking is a convenience on top of a worktree that already exists, so a
// failure here warns and moves on rather than undoing the checkout.
async function linkFolders(
  repoRoot: string,
  target: string,
  paths: string[],
): Promise<void> {
  try {
    const results = await native.gitLinkPaths(repoRoot, target, paths);
    const failed = results.filter((r) => r.outcome === "failed");
    if (failed.length > 0) {
      toast.warning(`Could not link ${failed.map((f) => f.path).join(", ")}`, {
        description: failed[0].detail ?? undefined,
      });
    }
  } catch (e) {
    toast.warning("Worktree created, but linking folders failed", {
      description: String(e),
    });
  }
}
