import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { ArrowRight02Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  filterHosts,
  hostHint,
  type SshEnv,
  type SshHost,
  validateHost,
} from "./lib/launcherItems";

export type RemoteConnectOptions = {
  /** Run the remote shell inside a tmux session named after the Space. */
  sshTmux: boolean;
};

type Props = {
  /** Hosts from ~/.ssh/config; null while loading, [] when unavailable. */
  hosts: SshHost[] | null;
  onConnect: (
    env: SshEnv,
    options: RemoteConnectOptions,
  ) => Promise<void> | void;
  hostInputRef?: RefObject<HTMLInputElement | null>;
  /** Esc anywhere in the form. */
  onCancel?: () => void;
};

/**
 * Inline "connect to a remote host" form. Free text always works; known ssh
 * config hosts appear as chips under the field. Enter in either field submits.
 */
export function RemoteConnectForm({
  hosts,
  onConnect,
  hostInputRef,
  onCancel,
}: Props) {
  const [host, setHost] = useState("");
  const [path, setPath] = useState("");
  // Default on: the whole point of a remote Space is picking up where you
  // left off; opting OUT of host-side persistence is the unusual choice.
  const [tmux, setTmux] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pathRef = useRef<HTMLInputElement>(null);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const suggestions = hosts ? filterHosts(hosts, host) : [];

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const problem = validateHost(host);
    if (problem) {
      setError(problem);
      hostInputRef?.current?.focus();
      return;
    }
    setError(null);
    setBusy(true);
    try {
      await onConnect(
        { kind: "ssh", host: host.trim(), path: path.trim() },
        { sshTmux: tmux },
      );
    } catch (err) {
      if (alive.current) setError(String(err));
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLFormElement>) => {
    if (e.key !== "Escape" || !onCancel || busy) return;
    e.preventDefault();
    onCancel();
  };

  return (
    <form
      onSubmit={(e) => void submit(e)}
      onKeyDown={onKeyDown}
      aria-label="Connect to a remote host"
      className="flex flex-col gap-2 rounded-md border border-border/40 p-3"
    >
      <Input
        ref={hostInputRef}
        value={host}
        onChange={(e) => {
          setHost(e.target.value);
          if (error) setError(null);
        }}
        placeholder="user@host or an ssh config alias"
        aria-label="Remote host"
        aria-invalid={error ? true : undefined}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        disabled={busy}
        className="h-8 w-full font-mono text-xs"
      />
      <div className="flex items-center gap-2">
        <Input
          ref={pathRef}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="~ (remote home)"
          aria-label="Remote path"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          disabled={busy}
          className="h-8 min-w-0 flex-1 font-mono text-xs"
        />
        <Button
          type="submit"
          size="sm"
          disabled={busy}
          className="h-8 shrink-0"
        >
          {busy ? "Connecting…" : "Connect"}
          {busy ? null : (
            <HugeiconsIcon icon={ArrowRight02Icon} size={14} strokeWidth={2} />
          )}
        </Button>
      </div>
      <label
        htmlFor="remote-connect-tmux"
        className="flex cursor-pointer items-start gap-2 pt-0.5"
      >
        <Checkbox
          id="remote-connect-tmux"
          checked={tmux}
          onCheckedChange={(v) => setTmux(v === true)}
          disabled={busy}
          className="mt-px size-3.5"
        />
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="text-[11.5px] leading-tight text-foreground/90">
            Keep the session alive on the host (tmux)
          </span>
          <span className="text-[10.5px] leading-snug text-muted-foreground/60">
            Reattach the same session from any device. Prompt tracking and
            agent status are limited inside tmux.
          </span>
        </span>
      </label>
      {error ? (
        <p role="alert" className="text-[11px] text-destructive">
          {error}
        </p>
      ) : null}
      {suggestions.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="mr-0.5 font-mono text-[10px] text-muted-foreground/50">
            ssh config
          </span>
          {suggestions.map((h) => {
            const hint = hostHint(h);
            return (
              <button
                key={h.alias}
                type="button"
                disabled={busy}
                onClick={() => {
                  setHost(h.alias);
                  setError(null);
                  pathRef.current?.focus();
                }}
                title={hint ?? h.alias}
                className="flex h-6 items-center gap-1.5 rounded-md border border-border/60 px-2 font-mono text-[11px] text-muted-foreground transition-colors hover:border-border hover:bg-accent/60 hover:text-foreground focus-visible:bg-accent/60 focus-visible:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary/40"
              >
                <span>{h.alias}</span>
                {hint ? (
                  <span className="max-w-40 truncate text-[10px] text-muted-foreground/50">
                    {hint}
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>
      ) : hosts === null ? (
        <p className="text-[10.5px] text-muted-foreground/40">
          Reading ssh config…
        </p>
      ) : null}
    </form>
  );
}
