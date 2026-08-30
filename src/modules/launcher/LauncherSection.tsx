import { cn } from "@/lib/utils";
import { HugeiconsIcon } from "@hugeicons/react";
import type { ReactNode } from "react";
import { KeyTokens } from "./KeyTokens";
import type {
  LauncherItemModel,
  LauncherSectionModel,
} from "./lib/launcherItems";

export function LauncherSectionTitle({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <h2
      className={cn(
        "px-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground/60",
        className,
      )}
    >
      {children}
    </h2>
  );
}

/**
 * One titled, keyboard-navigable list of the start page. Renders nothing when
 * the section has no items and no empty-state line, so callers can pass
 * sections unconditionally.
 */
export function LauncherSection({
  section,
  className,
}: {
  section: LauncherSectionModel;
  className?: string;
}) {
  if (section.items.length === 0 && !section.empty) return null;
  return (
    <section
      aria-label={section.title}
      className={cn("flex flex-col gap-1.5", className)}
    >
      <LauncherSectionTitle>{section.title}</LauncherSectionTitle>
      {section.items.length === 0 ? (
        <p className="px-2.5 py-1.5 text-[11.5px] text-muted-foreground/50">
          {section.empty}
        </p>
      ) : (
        <ul className="flex flex-col">
          {section.items.map((item) => (
            <li key={item.id}>
              <LauncherRow item={item} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function LauncherRow({ item }: { item: LauncherItemModel }) {
  return (
    <button
      type="button"
      data-launcher-stop=""
      data-launcher-item={item.id}
      onClick={item.onSelect}
      className={cn(
        "group flex h-8 w-full items-center gap-3 rounded-md px-2.5 text-left outline-none transition-colors",
        "hover:bg-accent/40 focus-visible:bg-accent/60 focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary/40",
      )}
    >
      <span className="flex size-4 shrink-0 items-center justify-center">
        {item.icon ? (
          <HugeiconsIcon
            icon={item.icon}
            size={15}
            strokeWidth={1.75}
            className="text-muted-foreground transition-colors group-hover:text-foreground group-focus-visible:text-foreground"
          />
        ) : item.accent ? (
          <span
            aria-hidden
            className="size-2 rounded-full"
            style={{ backgroundColor: item.accent }}
          />
        ) : null}
      </span>
      <span className="flex min-w-0 flex-1 items-center gap-2">
        <span className="truncate font-mono text-[12.5px] tracking-[0.01em] text-foreground">
          {item.label}
        </span>
        {item.badge ? (
          <span className="shrink-0 rounded-[3px] border border-border/60 px-1 font-mono text-[9.5px] leading-4 text-muted-foreground/80">
            {item.badge}
          </span>
        ) : null}
      </span>
      {item.description ? (
        <span className="min-w-0 max-w-[55%] shrink truncate font-mono text-[11px] text-muted-foreground/55">
          {item.description}
        </span>
      ) : null}
      {item.hint ? (
        <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground/50">
          {item.hint}
        </span>
      ) : null}
      {item.shortcut ? (
        <span className="shrink-0">
          <KeyTokens label={item.shortcut} />
        </span>
      ) : null}
    </button>
  );
}
