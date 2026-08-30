import { Switch } from "@/components/ui/switch";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  setCliEnabled,
  setCliNotify,
  setCliPanelControl,
  setCliTerminalInput,
  setCliTerminalRead,
} from "@/modules/settings/store";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";

// Settings > CLI: the permission matrix behind the `koden` command
// (modules/cli). Rows are surfaces, columns Read / Control; each cell maps
// to one preference, or is fixed "always" for list-only reads.

type Cell =
  | { kind: "pref"; value: boolean; set: (v: boolean) => Promise<void> }
  | { kind: "always" }
  | { kind: "none" };

type Row = {
  surface: string;
  read: Cell;
  control: Cell;
  readHint: string;
  controlHint: string;
};

export function CliSection() {
  const enabled = usePreferencesStore((s) => s.cliEnabled);
  const terminalRead = usePreferencesStore((s) => s.cliTerminalRead);
  const terminalInput = usePreferencesStore((s) => s.cliTerminalInput);
  const panelControl = usePreferencesStore((s) => s.cliPanelControl);
  const notify = usePreferencesStore((s) => s.cliNotify);

  const rows: Row[] = [
    {
      surface: "Terminal",
      read: { kind: "pref", value: terminalRead, set: setCliTerminalRead },
      control: { kind: "pref", value: terminalInput, set: setCliTerminalInput },
      readHint: "terminal list, terminal read",
      controlHint: "terminal type, press, run",
    },
    {
      surface: "Panels",
      read: { kind: "always" },
      control: { kind: "pref", value: panelControl, set: setCliPanelControl },
      readHint: "space list, ping",
      controlHint: "tab open, pane split, space new",
    },
    {
      surface: "Notify",
      read: { kind: "none" },
      control: { kind: "pref", value: notify, set: setCliNotify },
      readHint: "",
      controlHint: "notify",
    },
  ];

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="CLI"
        description="Every terminal Koden opens has a koden command. A coding agent running there can read panes, type into them, open tabs and spaces, and notify you. Each surface is gated below; a denied call answers with an error, never silently."
      />

      <SettingRow
        title="Enable the koden CLI"
        description="Off: every koden call answers 'disabled'. The command itself stays defined so scripts fail loudly instead of with 'not found'."
      >
        <Switch
          checked={enabled}
          onCheckedChange={(v) => void setCliEnabled(v)}
        />
      </SettingRow>

      <div className="flex flex-col gap-2">
        <Label>Permissions</Label>
        <div
          className={`overflow-hidden rounded-lg border border-border/60 bg-card/60 ${enabled ? "" : "opacity-50"}`}
        >
          <table className="w-full border-collapse text-[12px]">
            <thead>
              <tr className="border-b border-border/60 text-[10.5px] text-muted-foreground">
                <th className="px-3 py-2 text-left font-medium">Surface</th>
                <th className="px-3 py-2 text-left font-medium">Read</th>
                <th className="px-3 py-2 text-left font-medium">Control</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr
                  key={r.surface}
                  className="border-b border-border/40 last:border-b-0"
                >
                  <td className="px-3 py-2.5 align-top font-mono text-[12.5px] font-medium">
                    {r.surface}
                  </td>
                  <td className="px-3 py-2.5 align-top">
                    <CellView
                      cell={r.read}
                      hint={r.readHint}
                      disabled={!enabled}
                    />
                  </td>
                  <td className="px-3 py-2.5 align-top">
                    <CellView
                      cell={r.control}
                      hint={r.controlHint}
                      disabled={!enabled}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <span className="text-[10.5px] text-muted-foreground">
          Read never changes anything on screen. Control sends keystrokes to a
          pane or rearranges the window; input lands on whatever owns the pane,
          a shell or a foreground app. Privacy-mode tabs refuse both.
        </span>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Usage</Label>
        <div className="rounded-lg border border-border/60 bg-card/60 px-3 py-2.5 font-mono text-[11.5px] leading-relaxed">
          <div>koden --help</div>
          <div>koden terminal list</div>
          <div>koden terminal read --lines 40</div>
          <div>koden terminal run "pnpm test" --panel api</div>
          <div>koden notify "tests green, ready for review"</div>
        </div>
        <span className="text-[10.5px] text-muted-foreground">
          Without --panel a command targets the terminal it runs in, so an agent
          reads its own screen by default. Only shells opened by this Koden
          window can reach it; the link is per instance and dies with it.
        </span>
      </div>
    </div>
  );
}

function CellView({
  cell,
  hint,
  disabled,
}: {
  cell: Cell;
  hint: string;
  disabled: boolean;
}) {
  if (cell.kind === "none") {
    return <span className="text-[10.5px] text-muted-foreground/60">n/a</span>;
  }
  return (
    <div className="flex flex-col gap-1">
      {cell.kind === "always" ? (
        <span className="font-mono text-[10.5px] text-muted-foreground">
          always
        </span>
      ) : (
        <Switch
          size="sm"
          checked={cell.value}
          disabled={disabled}
          onCheckedChange={(v) => void cell.set(v)}
        />
      )}
      <span className="text-[10px] text-muted-foreground">{hint}</span>
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11px] font-medium tracking-tight text-muted-foreground">
      {children}
    </span>
  );
}
