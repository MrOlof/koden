import { lazy, Suspense } from "react";

const LibraryPaneInner = lazy(() =>
  import("./LibraryPane").then((m) => ({ default: m.LibraryPane })),
);

// Keeps streamdown (the page renderer) out of the startup bundle, mirroring
// MarkdownStackLazy (locked by the eager-budget test).
export function LibraryPane() {
  return (
    <Suspense fallback={null}>
      <LibraryPaneInner />
    </Suspense>
  );
}
