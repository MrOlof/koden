import { cn } from "@/lib/utils";
import { ArrowRight01Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import type { ReactNode } from "react";
import type {
  LauncherItemModel,
  LauncherSectionModel,
} from "./lib/launcherItems";

export function LauncherSectionTitle({ children }: { children: ReactNode }) {
  return (
    <h2 className="px-3 font-mono text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground/60">
      {children}
    </h2>
  );
}

/**
 * One titled, keyboard-navigable list of the launcher. Renders nothing when
 * the section has no items and no empty-state line, so callers can pass
 * sections unconditionally.
 */
export function LauncherSection({ section }: { section: LauncherSectionModel }) {
  if (section.items.length === 0 && !section.empty) return null;
  return (
    <section aria-label={section.title} className="flex flex-col gap-1">
      <LauncherSectionTitle>{section.title}</LauncherSectionTitle>
      {section.items.length === 0 ? (
        <p className="px-3 py-1.5 text-[11.5px] text-muted-foreground/50">
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
      onClick={item.onSelect}
      className={cn(
        "group flex w-full items-center gap-3 rounded-md px-3 py-2 text-left outline-none transition-colors",
        "hover:bg-accent/50 focus-visible:bg-accent/70 focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary/40",
      )}
    >
      <span className="flex size-4 shrink-0 items-center justify-center">
        {item.accent ? (
          <span
            aria-hidden
            className="size-2 rounded-full"
            style={{ backgroundColor: item.accent }}
          />
        ) : item.icon ? (
          <HugeiconsIcon
            icon={item.icon}
            size={15}
            strokeWidth={1.75}
            className="text-muted-foreground transition-colors group-hover:text-foreground group-focus-visible:text-foreground"
          />
        ) : null}
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-2">
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
          <span className="truncate text-[11px] leading-snug text-muted-foreground/60">
            {item.description}
          </span>
        ) : null}
      </span>
      {item.hint ? (
        <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground/50">
          {item.hint}
        </span>
      ) : null}
      <HugeiconsIcon
        icon={ArrowRight01Icon}
        size={13}
        strokeWidth={2}
        className="shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-60 group-focus-visible:opacity-80"
      />
    </button>
  );
}
