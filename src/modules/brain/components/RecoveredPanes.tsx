import { Cancel01Icon, PlayIcon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { Button } from "@/components/ui/button";
import { AgentIcon } from "@/modules/agents/lib/agentIcon";
import {
  cardMeta,
  resumeActionLabel,
  type ResumeCardModel,
} from "../lib/resumeCards";

type Props = {
  cards: ResumeCardModel[];
  onResume: (key: string) => void;
  onDismiss: (key: string) => void;
  onDismissAll: () => void;
};

/** Dismissible strip above the tab area: one card per pane recoverable from
 * the previous session. Renders nothing once every card is handled. */
export function RecoveredPanesBanner({
  cards,
  onResume,
  onDismiss,
  onDismissAll,
}: Props) {
  if (cards.length === 0) return null;
  return (
    <section
      aria-label="Resume where you left off"
      className="shrink-0 px-3 pt-2"
    >
      <div className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/60 px-2.5 py-1.5">
        <span className="shrink-0 font-mono text-[11px] font-medium tracking-tight text-muted-foreground">
          Resume where you left off
        </span>
        <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto">
          {cards.map((c) => (
            <ResumeCard
              key={c.key}
              card={c}
              onResume={onResume}
              onDismiss={onDismiss}
            />
          ))}
        </div>
        {cards.length > 1 ? (
          <Button
            variant="ghost"
            size="xs"
            className="shrink-0 text-muted-foreground"
            onClick={onDismissAll}
          >
            Dismiss all
          </Button>
        ) : null}
      </div>
    </section>
  );
}

function ResumeCard({
  card,
  onResume,
  onDismiss,
}: {
  card: ResumeCardModel;
  onResume: (key: string) => void;
  onDismiss: (key: string) => void;
}) {
  const meta = cardMeta(card);
  return (
    <div
      className="flex shrink-0 items-center gap-2 rounded-md border border-border/50 bg-background/60 py-1 pr-1 pl-2"
      title={card.cwd}
    >
      <AgentIcon agent={card.agent} size={14} className="shrink-0 opacity-80" />
      <div className="flex min-w-0 max-w-56 flex-col leading-tight">
        <span className="truncate font-mono text-[12px]">{card.cwdShort}</span>
        <span className="truncate text-[10.5px] text-muted-foreground">
          {meta}
        </span>
      </div>
      <Button
        size="xs"
        variant={card.resumable ? "default" : "outline"}
        onClick={() => onResume(card.key)}
        title={
          card.resumable
            ? "Open a terminal here and resume this session"
            : "Open a terminal in this folder"
        }
      >
        <HugeiconsIcon icon={PlayIcon} />
        {resumeActionLabel(card)}
      </Button>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label="Dismiss"
        title="Dismiss"
        onClick={() => onDismiss(card.key)}
      >
        <HugeiconsIcon icon={Cancel01Icon} />
      </Button>
    </div>
  );
}
