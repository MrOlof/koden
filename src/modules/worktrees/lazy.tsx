import { lazy, Suspense } from "react";
import type { ComponentProps } from "react";
import type { NewWorktreeDialog as NewWorktreeDialogType } from "./NewWorktreeDialog";
import type { RemoveWorktreeDialog as RemoveWorktreeDialogType } from "./RemoveWorktreeDialog";

// Both dialogs sit mounted in App, so a value import would drag them into the
// startup chunk; they only matter once the user asks for a worktree.
const NewInner = lazy(() =>
  import("./NewWorktreeDialog").then((m) => ({ default: m.NewWorktreeDialog })),
);
const RemoveInner = lazy(() =>
  import("./RemoveWorktreeDialog").then((m) => ({
    default: m.RemoveWorktreeDialog,
  })),
);

export function NewWorktreeDialog(
  props: ComponentProps<typeof NewWorktreeDialogType>,
) {
  return (
    <Suspense fallback={null}>
      <NewInner {...props} />
    </Suspense>
  );
}

export function RemoveWorktreeDialog(
  props: ComponentProps<typeof RemoveWorktreeDialogType>,
) {
  return (
    <Suspense fallback={null}>
      <RemoveInner {...props} />
    </Suspense>
  );
}
