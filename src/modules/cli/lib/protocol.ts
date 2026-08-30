// Webview side of the koden CLI contract (src-tauri/src/modules/cli/protocol.rs).
// Rust strips the token before emitting; the webview answers via `cli_reply`.

export const CLI_REQUEST_EVENT = "koden:cli-request";

export type CliRequest = {
  id: string;
  cmd: string;
  args: Record<string, unknown>;
  /** KODEN_SESSION of the calling shell (the pty id), when set. */
  session: string | null;
};

export type CliResult =
  | { ok: true; result: unknown }
  | { ok: false; error: string };

export function cliError(error: string): CliResult {
  return { ok: false, error };
}

export function cliOk(result: unknown): CliResult {
  return { ok: true, result };
}

/** Rust already validated shape and size; this guards the event boundary
 * against anything else that might emit on the same channel. */
export function isCliRequest(v: unknown): v is CliRequest {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.id === "string" &&
    o.id.length > 0 &&
    o.id.length <= 64 &&
    typeof o.cmd === "string" &&
    o.cmd.length > 0 &&
    !!o.args &&
    typeof o.args === "object" &&
    !Array.isArray(o.args) &&
    (o.session === null ||
      o.session === undefined ||
      typeof o.session === "string")
  );
}
