import { lazy, Suspense } from "react";
import type { ComponentProps } from "react";
import type { LauncherPane as LauncherPaneType } from "./LauncherPane";

// The launcher renders only while a launcher tab exists; keep its section
// components, form and icons out of the startup chunk.
const LauncherPaneInner = lazy(() =>
  import("./LauncherPane").then((m) => ({ default: m.LauncherPane })),
);

export function LauncherPane(props: ComponentProps<typeof LauncherPaneType>) {
  return (
    <Suspense fallback={null}>
      <LauncherPaneInner {...props} />
    </Suspense>
  );
}
