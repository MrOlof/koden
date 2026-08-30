import { Kbd, KbdGroup } from "@/components/ui/kbd";

/** Keycaps for a space-separated binding label ("Ctrl Shift T"). */
export function KeyTokens({ label }: { label: string }) {
  const tokens = label.split(" ").filter(Boolean);
  if (tokens.length === 0) {
    return (
      <span className="font-mono text-[10px] text-muted-foreground/40">
        unbound
      </span>
    );
  }
  return (
    <KbdGroup className="gap-0.5">
      {tokens.map((t) => (
        <Kbd
          key={t}
          className="h-[18px] min-w-[18px] rounded-[3px] border-border/60 bg-muted/40 px-1 text-[10px] font-medium text-muted-foreground"
        >
          {t}
        </Kbd>
      ))}
    </KbdGroup>
  );
}
