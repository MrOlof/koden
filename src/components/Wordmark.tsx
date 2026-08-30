import { cn } from "@/lib/utils";

type Props = {
  className?: string;
};

/** Mono lowercase "koden" trailed by a spruce block cursor with a slow blink.
 * Identity mark only — used in the About header and the Librarian chat empty
 * state. */
export function Wordmark({ className }: Props) {
  return (
    <span
      className={cn(
        "relative inline-flex select-none items-baseline font-mono lowercase tracking-tight text-foreground",
        className,
      )}
    >
      koden
      {/* Out of flow so the mark's box is the word alone: anything centered on
          it (the launcher icon) lines up with "koden", not "koden" + cursor. */}
      <span
        aria-hidden
        className="koden-wordmark-cursor absolute left-full top-0 -ml-px text-primary"
      >
        {"▊"}
      </span>
    </span>
  );
}
