import { native } from "@/modules/ai/lib/native";
import { useEffect, useRef } from "react";
import { type DirectorCommand, readNewCommands } from "../lib/bus";

const POLL_MS = 1500;

type Props = {
  /** Absolute path of the command bus file, or null when no Director is live. */
  busPath: string | null;
  onCommand: (cmd: DirectorCommand) => void;
};

/**
 * Tails the Director command bus file and dispatches new commands. Active only
 * while a Director is running (busPath non-null). Resets its offset when the
 * path changes so a freshly-cleared bus starts from the top.
 */
export function DirectorBusBridge({ busPath, onCommand }: Props) {
  const processed = useRef(0);
  const cmdRef = useRef(onCommand);
  cmdRef.current = onCommand;

  useEffect(() => {
    if (!busPath) return;
    processed.current = 0;
    let stopped = false;

    const tick = async () => {
      try {
        const res = await native.readFile(busPath);
        if (res.kind !== "text") return;
        const { commands, processedLines } = readNewCommands(
          res.content,
          processed.current,
        );
        processed.current = processedLines;
        if (!stopped) for (const c of commands) cmdRef.current(c);
      } catch {
        // bus file may not exist yet, or read denied — retry next tick
      }
    };

    const id = window.setInterval(() => void tick(), POLL_MS);
    void tick();
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [busPath]);

  return null;
}
