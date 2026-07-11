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
        "inline-flex select-none items-baseline font-mono lowercase tracking-tight text-foreground",
        className,
      )}
    >
      koden
      <span
        aria-hidden
        className="koden-wordmark-cursor -ml-px text-primary"
      >
        {"▊"}
      </span>
    </span>
  );
}
