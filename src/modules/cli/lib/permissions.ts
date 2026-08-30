// Surface x access gate for the koden CLI (Settings > CLI). Read is the
// harmless half (lists, buffers); Control is anything that changes what the
// user sees or what a shell receives.

export type CliPrefs = {
  cliEnabled: boolean;
  cliTerminalRead: boolean;
  cliTerminalInput: boolean;
  cliPanelControl: boolean;
  cliNotify: boolean;
};

export type CliGate = {
  surface: "Terminal" | "Panel" | "Notify";
  access: "read" | "control";
  /** Pref that governs it; null = always allowed once the CLI is on. */
  pref: keyof CliPrefs | null;
};

const GATES: Record<string, CliGate> = {
  "terminal.list": {
    surface: "Terminal",
    access: "read",
    pref: "cliTerminalRead",
  },
  "terminal.read": {
    surface: "Terminal",
    access: "read",
    pref: "cliTerminalRead",
  },
  "terminal.type": {
    surface: "Terminal",
    access: "control",
    pref: "cliTerminalInput",
  },
  "terminal.press": {
    surface: "Terminal",
    access: "control",
    pref: "cliTerminalInput",
  },
  "terminal.run": {
    surface: "Terminal",
    access: "control",
    pref: "cliTerminalInput",
  },
  "tab.open": { surface: "Panel", access: "control", pref: "cliPanelControl" },
  "pane.split": {
    surface: "Panel",
    access: "control",
    pref: "cliPanelControl",
  },
  "space.new": { surface: "Panel", access: "control", pref: "cliPanelControl" },
  "space.list": { surface: "Panel", access: "read", pref: null },
  notify: { surface: "Notify", access: "control", pref: "cliNotify" },
  ping: { surface: "Panel", access: "read", pref: null },
};

export const CLI_COMMANDS: readonly string[] = Object.keys(GATES);

export function gateFor(cmd: string): CliGate | null {
  return GATES[cmd] ?? null;
}

/** Null when allowed, else the error the CLI prints. Unknown commands are
 * refused here too so a new client can never reach an unimplemented path. */
export function checkPermission(cmd: string, prefs: CliPrefs): string | null {
  const gate = gateFor(cmd);
  if (!gate) return `unknown command '${cmd}'`;
  if (!prefs.cliEnabled) return "the koden CLI is disabled in Settings > CLI";
  if (gate.pref && !prefs[gate.pref])
    return `${gate.surface} ${gate.access} is disabled in Settings > CLI`;
  return null;
}
