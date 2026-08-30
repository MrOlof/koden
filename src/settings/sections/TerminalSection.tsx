import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { detectNerdFonts } from "@/lib/fonts";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  TERMINAL_FONT_SIZES,
  TERMINAL_LINE_HEIGHTS,
  TERMINAL_SCROLLBACK_PRESETS,
  TERMINAL_SCROLLBACK_RESTORE_PRESETS,
  setAutoResumeAgents,
  setCommandMinimapEnabled,
  setLinkTypes,
  setSmartLinksEnabled,
  setTerminalCursorBlink,
  setTerminalFontFamily,
  setTerminalFontSize,
  setTerminalLetterSpacing,
  setTerminalLineHeight,
  setTerminalScrollback,
  setTerminalScrollbackRestoreLines,
  setTerminalWebglEnabled,
} from "@/modules/settings/store";
import { Switch } from "@/components/ui/switch";
import {
  type LinkAction,
  type LinkCategory,
  LINK_CATEGORY_LABELS,
  LINK_CATEGORY_ORDER,
  type LinkTypeConfig,
} from "@/modules/terminal/lib/linkDetect";
import { useMemo } from "react";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";

const LETTER_SPACINGS = [-4, -3, -2, -1, 0, 1, 2, 3, 4] as const;

// Auto-detect is the empty-string default; Radix Select needs a non-empty
// sentinel value to represent it.
const FONT_AUTO = "__auto__";
const CURATED_FONTS = ["Commit Mono", "JetBrains Mono"] as const;

export function TerminalSection() {
  const terminalWebglEnabled = usePreferencesStore(
    (s) => s.terminalWebglEnabled,
  );
  const smartLinksEnabled = usePreferencesStore((s) => s.smartLinksEnabled);
  const commandMinimapEnabled = usePreferencesStore(
    (s) => s.commandMinimapEnabled,
  );
  const linkTypes = usePreferencesStore((s) => s.linkTypes);
  const terminalCursorBlink = usePreferencesStore((s) => s.terminalCursorBlink);
  const terminalFontFamily = usePreferencesStore((s) => s.terminalFontFamily);
  const terminalLetterSpacing = usePreferencesStore(
    (s) => s.terminalLetterSpacing,
  );
  const terminalLineHeight = usePreferencesStore((s) => s.terminalLineHeight);
  const terminalFontSize = usePreferencesStore((s) => s.terminalFontSize);
  const terminalScrollback = usePreferencesStore((s) => s.terminalScrollback);
  const restoreLines = usePreferencesStore(
    (s) => s.terminalScrollbackRestoreLines,
  );
  const autoResumeAgents = usePreferencesStore((s) => s.autoResumeAgents);

  // Detected Nerd Fonts join the two bundled fonts in the quick-pick. Probed
  // once on mount; the free-text field below still accepts anything else.
  const fontOptions = useMemo(() => {
    return Array.from(new Set([...CURATED_FONTS, ...detectNerdFonts()]));
  }, []);
  const fontIsCustom =
    terminalFontFamily !== "" && !fontOptions.includes(terminalFontFamily);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="Terminal"
        description="Rendering, fonts, smart links, and history."
      />

      <div className="flex flex-col gap-2">
        <Label>Rendering</Label>
        <SettingRow
          title={
            <span className="inline-flex items-center gap-1.5">
              Use WebGL renderer
              <TooltipProvider delayDuration={200}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span
                      className="cursor-help text-[11px] text-muted-foreground/70 leading-none"
                      aria-label="More info about WebGL renderer"
                    >
                      ⓘ
                    </span>
                  </TooltipTrigger>
                  <TooltipContent side="top" className="max-w-65 text-[11px]">
                    xterm's WebGL renderer caches glyphs in a GPU texture
                    atlas. On some macOS setups (especially with Nerd Fonts),
                    the atlas corrupts and terminal text becomes unreadable.
                    Turn this off as a fallback — performance dips slightly,
                    but text renders correctly via the DOM renderer.
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </span>
          }
          description="Hardware-accelerated rendering. Turn off if text shows corruption or blank tiles."
        >
          <Switch
            checked={terminalWebglEnabled}
            onCheckedChange={(v) => void setTerminalWebglEnabled(v)}
          />
        </SettingRow>
        <SettingRow
          title="Cursor blinking"
          description="Blink the terminal cursor. Off by default for lower idle CPU, matching VS Code and the macOS terminal."
        >
          <Switch
            checked={terminalCursorBlink}
            onCheckedChange={(v) => void setTerminalCursorBlink(v)}
          />
        </SettingRow>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Font</Label>
        <SettingRow
          title="Font family"
          description='Pick a detected or bundled mono font, or type any Nerd Font name (e.g. "CaskaydiaCove Nerd Font Mono"). Leave blank to auto-detect.'
        >
          <div className="flex flex-col items-end gap-1.5">
            <Select
              value={terminalFontFamily === "" ? FONT_AUTO : terminalFontFamily}
              onValueChange={(v) =>
                void setTerminalFontFamily(v === FONT_AUTO ? "" : v)
              }
            >
              <SelectTrigger size="sm" className="h-8 w-48 text-[12px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={FONT_AUTO} className="text-[12px]">
                  Auto-detect
                </SelectItem>
                {fontOptions.map((f) => (
                  <SelectItem key={f} value={f} className="text-[12px]">
                    {f}
                  </SelectItem>
                ))}
                {fontIsCustom ? (
                  <SelectItem
                    value={terminalFontFamily}
                    className="text-[12px]"
                  >
                    {terminalFontFamily} (custom)
                  </SelectItem>
                ) : null}
              </SelectContent>
            </Select>
            <input
              type="text"
              value={terminalFontFamily}
              placeholder="Auto-detect"
              spellCheck={false}
              onChange={(e) => void setTerminalFontFamily(e.target.value)}
              className="h-8 w-48 rounded-md border border-border bg-background px-2.5 text-[12px] outline-none focus:border-foreground/40"
            />
          </div>
        </SettingRow>
        <SettingRow title="Font size" description="Terminal text size.">
          <Select
            value={String(terminalFontSize)}
            onValueChange={(v) => void setTerminalFontSize(Number(v))}
          >
            <SelectTrigger size="sm" className="h-8 w-28 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TERMINAL_FONT_SIZES.map((size) => (
                <SelectItem
                  key={size}
                  value={String(size)}
                  className="text-[12px]"
                >
                  {size} px
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow
          title="Letter spacing"
          description="Extra horizontal space between characters (px). Use negative values to tighten Nerd Fonts."
        >
          <Select
            value={String(terminalLetterSpacing)}
            onValueChange={(v) => void setTerminalLetterSpacing(Number(v))}
          >
            <SelectTrigger size="sm" className="h-8 w-28 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LETTER_SPACINGS.map((v) => (
                <SelectItem key={v} value={String(v)} className="text-[12px]">
                  {v > 0 ? `+${v}` : v} px
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow
          title="Line height"
          description="Vertical space between rows, as a multiple of the font size. 1.0 is tightest."
        >
          <Select
            value={terminalLineHeight.toFixed(1)}
            onValueChange={(v) => void setTerminalLineHeight(Number(v))}
          >
            <SelectTrigger size="sm" className="h-8 w-28 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TERMINAL_LINE_HEIGHTS.map((v) => (
                <SelectItem
                  key={v}
                  value={v.toFixed(1)}
                  className="text-[12px]"
                >
                  {v.toFixed(1)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Links & history</Label>
        <SettingRow
          title="Smart links"
          description="Ctrl/Cmd+click detected tokens in the terminal. Choose what each type does below. URLs are always clickable."
        >
          <Switch
            checked={smartLinksEnabled}
            onCheckedChange={(v) => void setSmartLinksEnabled(v)}
          />
        </SettingRow>
        <LinkTypesGroup
          linkTypes={linkTypes}
          disabled={!smartLinksEnabled}
          onChange={(category, action) =>
            void setLinkTypes({ ...linkTypes, [category]: action })
          }
        />
        <SettingRow
          title="Terminal command history"
          description="Show a search button in the pane header that lists every command and Claude prompt in the terminal — click an entry to jump to it."
        >
          <Switch
            checked={commandMinimapEnabled}
            onCheckedChange={(v) => void setCommandMinimapEnabled(v)}
          />
        </SettingRow>
        <SettingRow
          title="Scrollback"
          description="Lines of history kept per terminal. Higher uses more RAM (~3 KB / line)."
        >
          <Select
            value={String(terminalScrollback)}
            onValueChange={(v) => void setTerminalScrollback(Number(v))}
          >
            <SelectTrigger size="sm" className="h-8 w-36 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TERMINAL_SCROLLBACK_PRESETS.map((lines) => (
                <SelectItem
                  key={lines}
                  value={String(lines)}
                  className="text-[12px]"
                >
                  {lines.toLocaleString()} lines
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow
          title="Restore scrollback on launch"
          description="Snapshot each terminal's recent output when the layout saves and replay it into the restored terminal next launch. Private terminals are never saved."
        >
          <Select
            value={String(restoreLines)}
            onValueChange={(v) => void setTerminalScrollbackRestoreLines(Number(v))}
          >
            <SelectTrigger size="sm" className="h-8 w-36 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TERMINAL_SCROLLBACK_RESTORE_PRESETS.map((lines) => (
                <SelectItem
                  key={lines}
                  value={String(lines)}
                  className="text-[12px]"
                >
                  {lines === 0 ? "Off" : `${lines.toLocaleString()} lines`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow
          title="Auto-resume agents"
          description="On launch, resume a Claude Code session in its restored terminal automatically (when a session id was captured) instead of showing a resume card."
        >
          <Switch
            checked={autoResumeAgents}
            onCheckedChange={(v) => void setAutoResumeAgents(v)}
          />
        </SettingRow>
      </div>
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

const LINK_ACTION_OPTIONS: { id: LinkAction; label: string }[] = [
  { id: "off", label: "Off" },
  { id: "copy", label: "Copy" },
  { id: "open", label: "Open" },
];

const LINK_CATEGORY_HINTS: Record<LinkCategory, string> = {
  path: "Reveal in your file manager.",
  filename: "Bare name.ext like config.json or setup.exe.",
  ip: "IPv4 addresses, with optional port.",
  email: "Addresses and UPNs.",
  guid: "Any-version UUIDs.",
  secret: "Tokens, keys, JWTs, hashes, labeled credentials.",
  sid: "Windows security identifiers (S-1-5-…).",
  winuser: "DOMAIN\\user logins.",
};

function LinkTypesGroup({
  linkTypes,
  disabled,
  onChange,
}: {
  linkTypes: LinkTypeConfig;
  disabled: boolean;
  onChange: (category: LinkCategory, action: LinkAction) => void;
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 rounded-lg border border-border/60 p-3 transition-opacity",
        disabled && "pointer-events-none opacity-50",
      )}
      aria-disabled={disabled}
    >
      <div className="mb-1 flex items-center justify-between">
        <span className="text-[11px] font-medium text-muted-foreground">
          Link types
        </span>
        <span className="text-[10.5px] text-muted-foreground/70">
          Off / Copy / Open
        </span>
      </div>
      {LINK_CATEGORY_ORDER.map((category) => (
        <div key={category} className="flex items-center justify-between gap-3">
          <div className="flex min-w-0 flex-col">
            <span className="text-[12px] text-foreground">
              {LINK_CATEGORY_LABELS[category]}
            </span>
            <span className="truncate text-[10.5px] text-muted-foreground">
              {LINK_CATEGORY_HINTS[category]}
            </span>
          </div>
          <Select
            value={linkTypes[category]}
            disabled={disabled}
            onValueChange={(v) => onChange(category, v as LinkAction)}
          >
            <SelectTrigger size="sm" className="h-8 w-24 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LINK_ACTION_OPTIONS.map((o) => (
                <SelectItem key={o.id} value={o.id} className="text-[12px]">
                  {o.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      ))}
    </div>
  );
}
