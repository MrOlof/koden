import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { native } from "@/modules/ai/lib/native";
import type { SpaceMeta } from "@/modules/spaces/lib/store";
import { useEffect, useState } from "react";
import { toast } from "sonner";

type Props = {
  /** The worktree Space to remove; null closes the dialog. */
  space: SpaceMeta | null;
  onOpenChange: (open: boolean) => void;
  onRemoved: (spaceId: string) => void;
};

export function RemoveWorktreeDialog({
  space,
  onOpenChange,
  onRemoved,
}: Props) {
  const [force, setForce] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!space) return;
    setForce(false);
    setBusy(false);
    setError(null);
  }, [space]);

  const worktree = space?.worktree ?? null;
  const root = space?.root ?? null;

  const confirm = async () => {
    if (!space || !worktree || !root) return;
    setBusy(true);
    setError(null);
    try {
      await native.gitWorktreeRemove(worktree.repoRoot, root, force);
      onRemoved(space.id);
      toast.success(`Worktree removed, branch ${worktree.branch} kept`);
    } catch (e) {
      const message = String(e);
      setError(message);
      toast.error("Could not remove worktree", { description: message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <AlertDialog
      open={space != null}
      onOpenChange={(o) => {
        if (!busy) onOpenChange(o);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove worktree?</AlertDialogTitle>
          <AlertDialogDescription>
            This deletes the checkout and closes the Space. The branch stays in
            the repository.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {space && worktree && (
          <div className="flex flex-col gap-3 text-xs">
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
              <dt className="text-muted-foreground">Space</dt>
              <dd className="truncate">{space.name}</dd>
              <dt className="text-muted-foreground">Branch</dt>
              <dd className="truncate font-mono">{worktree.branch}</dd>
              <dt className="text-muted-foreground">Folder</dt>
              <dd className="truncate font-mono">{root}</dd>
            </dl>
            <div className="flex items-center gap-2">
              <Checkbox
                id="worktree-force"
                checked={force}
                disabled={busy}
                onCheckedChange={(v) => setForce(v === true)}
              />
              <Label htmlFor="worktree-force" className="text-xs font-normal">
                Discard uncommitted changes in this worktree
              </Label>
            </div>
            {error && <div className="text-destructive">{error}</div>}
          </div>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={busy || !worktree || !root}
            onClick={() => void confirm()}
          >
            {busy ? "Removing..." : "Remove worktree"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
