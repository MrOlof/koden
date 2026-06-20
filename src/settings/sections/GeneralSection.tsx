import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import type {
  PaneColorMode,
  PaneColorPalette,
  ThemePref,
} from "@/modules/settings/store";
import {
  TERMINAL_FONT_SIZES,
  TERMINAL_SCROLLBACK_PRESETS,
  setAgentNotifications,
  setAutoRetryEnabled,
  setAutostart,
  setCommandMinimapEnabled,
  setEditorAutoSave,
  setEditorAutoSaveDelay,
  setDefaultFolder,
  setExplorerGitDecorations,
  setPaneColorMode,
  setPaneColorNotes,
  setPaneColorPalette,
  setPaneColorTask,
  setPaneColorTerminal,
  setLinkTypes,
  setRestoreWindowState,
  setShowHidden,
  setSmartLinksEnabled,
  setTerminalFontFamily,
  setTerminalLetterSpacing,
  setTerminalFontSize,
  setTerminalCursorBlink,
  setTerminalScrollback,
  setTerminalWebglEnabled,
  setUsageGuardEnabled,
  setUsageGuardHardStop,
  setUsageGuardPausePct,
  setUsageGuardWarnPct,
  setVimMode,
  setZoomLevel,
} from "@/modules/settings/store";
import {
  type LinkAction,
  type LinkCategory,
  LINK_CATEGORY_LABELS,
  LINK_CATEGORY_ORDER,
  type LinkTypeConfig,
} from "@/modules/terminal/lib/linkDetect";
import { paneColorAt } from "@/modules/terminal/lib/paneAutoColor";
import { useTheme } from "@/modules/theme";
import {
  ComputerIcon,
  Moon02Icon,
  Sun03Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useEffect, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";

const APPEARANCE: {
  id: ThemePref;
  label: string;
  icon: typeof ComputerIcon;
}[] = [
  { id: "system", label: "System", icon: ComputerIcon },
  { id: "light", label: "Light", icon: Sun03Icon },
  { id: "dark", label: "Dark", icon: Moon02Icon },
];

const PALETTES: { id: PaneColorPalette; label: string }[] = [
  { id: "muted", label: "Muted" },
  { id: "vibrant", label: "Vibrant" },
  { id: "pastel", label: "Pastel" },
];

const LETTER_SPACINGS = [-4, -3, -2, -1, 0, 1, 2, 3, 4] as const;
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 2.0;
const ZOOM_STEP = 0.05;
const AUTO_SAVE_STEP = 100;
const AUTO_SAVE_MIN = 100;
const AUTO_SAVE_MAX = 60000;
// Mirror the clamps in store.ts (clampUsageWarnPct / clampUsagePausePct).
const USAGE_WARN_MIN = 50;
const USAGE_WARN_MAX = 99;
const USAGE_PAUSE_MIN = 50;
const USAGE_PAUSE_MAX = 100;

export function GeneralSection() {
  const { mode, setMode } = useTheme();

  const autostart = usePreferencesStore((s) => s.autostart);
  const restoreWindowState = usePreferencesStore((s) => s.restoreWindowState);
  const vimMode = usePreferencesStore((s) => s.vimMode);
  const editorAutoSave = usePreferencesStore((s) => s.editorAutoSave);
  const editorAutoSaveDelay = usePreferencesStore((s) => s.editorAutoSaveDelay);
  const showHidden = usePreferencesStore((s) => s.showHidden);
  const explorerGitDecorations = usePreferencesStore(
    (s) => s.explorerGitDecorations,
  );
  const defaultFolder = usePreferencesStore((s) => s.defaultFolder);
  const [folderDraft, setFolderDraft] = useState(defaultFolder);
  useEffect(() => setFolderDraft(defaultFolder), [defaultFolder]);
  const paneColorMode = usePreferencesStore((s) => s.paneColorMode);
  const paneColorPalette = usePreferencesStore((s) => s.paneColorPalette);
  const paneColorTerminal = usePreferencesStore((s) => s.paneColorTerminal);
  const paneColorNotes = usePreferencesStore((s) => s.paneColorNotes);
  const paneColorTask = usePreferencesStore((s) => s.paneColorTask);
  const terminalWebglEnabled = usePreferencesStore(
    (s) => s.terminalWebglEnabled,
  );
  const smartLinksEnabled = usePreferencesStore((s) => s.smartLinksEnabled);
  const commandMinimapEnabled = usePreferencesStore(
    (s) => s.commandMinimapEnabled,
  );
  const linkTypes = usePreferencesStore((s) => s.linkTypes);
  const terminalCursorBlink = usePreferencesStore(
    (s) => s.terminalCursorBlink,
  );
  const terminalFontFamily = usePreferencesStore((s) => s.terminalFontFamily);
  const terminalLetterSpacing = usePreferencesStore(
    (s) => s.terminalLetterSpacing,
  );
  const terminalFontSize = usePreferencesStore((s) => s.terminalFontSize);
  const terminalScrollback = usePreferencesStore((s) => s.terminalScrollback);
  const zoomLevel = usePreferencesStore((s) => s.zoomLevel);
  const agentNotifications = usePreferencesStore((s) => s.agentNotifications);
  const autoRetryEnabled = usePreferencesStore((s) => s.autoRetryEnabled);
  const usageGuardEnabled = usePreferencesStore((s) => s.usageGuardEnabled);
  const usageGuardWarnPct = usePreferencesStore((s) => s.usageGuardWarnPct);
  const usageGuardPausePct = usePreferencesStore((s) => s.usageGuardPausePct);
  const usageGuardHardStop = usePreferencesStore((s) => s.usageGuardHardStop);

  useEffect(() => {
    let alive = true;
    void isEnabled()
      .then((on) => {
        if (!alive) return;
        if (on !== usePreferencesStore.getState().autostart) {
          void setAutostart(on);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const onToggleAutostart = async (next: boolean) => {
    try {
      if (next) await enable();
      else await disable();
      await setAutostart(next);
    } catch (e) {
      console.error("autostart toggle failed", e);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="General"
        description="Mode, editor, and startup."
      />

      <div className="flex flex-col gap-2">
        <Label>Appearance</Label>
        <div className="grid grid-cols-3 gap-2">
          {APPEARANCE.map((o) => (
            <button
              key={o.id}
              type="button"
              onClick={() => setMode(o.id)}
              className={cn(
                "group flex h-20 flex-col items-center justify-center gap-1.5 rounded-lg border bg-card transition-all",
                mode === o.id
                  ? "border-foreground/60 ring-1 ring-foreground/20"
                  : "border-border/60 hover:border-border",
              )}
            >
              <HugeiconsIcon icon={o.icon} size={18} strokeWidth={1.5} />
              <span className="text-[11.5px]">{o.label}</span>
            </button>
          ))}
        </div>
        <p className="text-[11px] text-muted-foreground">
          For theme, background and customization, see the{" "}
          <strong className="font-medium text-foreground">Themes</strong> tab.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Zoom</Label>
        <div className="flex flex-col gap-3 rounded-lg border border-border/60 p-3">
          <div className="flex items-center justify-between gap-3">
            <span className="text-[11.5px] text-muted-foreground">
              UI zoom level
            </span>
            <span className="tabular-nums text-[11px] text-muted-foreground">
              {Math.round(zoomLevel * 100)}%
            </span>
          </div>
          <Slider
            value={[zoomLevel]}
            min={ZOOM_MIN}
            max={ZOOM_MAX}
            step={ZOOM_STEP}
            onValueChange={(v) => void setZoomLevel(v[0] ?? 1)}
          />
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Editor</Label>
        <SettingRow
          title="Vim mode"
          description="Enable Vim keybindings in the code editor."
        >
          <Switch
            checked={vimMode}
            onCheckedChange={(v) => void setVimMode(v)}
          />
        </SettingRow>
        <SettingRow
          title="Auto save"
          description="Automatically save files after a delay when changes are detected."
        >
          <Switch
            checked={editorAutoSave}
            onCheckedChange={(v) => void setEditorAutoSave(v)}
          />
        </SettingRow>
        {editorAutoSave && (
          <AutoSaveDelayInput
            value={editorAutoSaveDelay}
            onChange={(v) => void setEditorAutoSaveDelay(v)}
          />
        )}
      </div>

      <div className="flex flex-col gap-2">
        <Label>Explorer</Label>
        <SettingRow
          title="Show hidden files"
          description="Include dot-prefixed files and folders (.env, .gitignore, .config) in the file explorer and search."
        >
          <Switch
            checked={showHidden}
            onCheckedChange={(v) => void setShowHidden(v)}
          />
        </SettingRow>
        <SettingRow
          title="Git decorations"
          description="Tint changed files and dim gitignored entries in the file explorer."
        >
          <Switch
            checked={explorerGitDecorations}
            onCheckedChange={(v) => void setExplorerGitDecorations(v)}
          />
        </SettingRow>
        <SettingRow
          title="Default folder"
          description="Folder the explorer and new terminals open to. Leave blank to use the launch directory, then your home folder."
        >
          <input
            type="text"
            value={folderDraft}
            placeholder="Launch dir / home"
            spellCheck={false}
            onChange={(e) => setFolderDraft(e.target.value)}
            onBlur={() => void setDefaultFolder(folderDraft)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void setDefaultFolder(folderDraft);
            }}
            className="h-8 w-56 rounded-md border border-border bg-background px-2.5 text-[12px] outline-none focus:border-foreground/40"
          />
        </SettingRow>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Terminal</Label>
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
                  <TooltipContent
                    side="top"
                    className="max-w-65 text-[11px]"
                  >
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
          title="Cursor blinking"
          description="Blink the terminal cursor. Off by default for lower idle CPU, matching VS Code and the macOS terminal."
        >
          <Switch
            checked={terminalCursorBlink}
            onCheckedChange={(v) => void setTerminalCursorBlink(v)}
          />
        </SettingRow>
        <SettingRow
          title="Font family"
          description='Nerd Font name for icons (e.g. "CaskaydiaCove Nerd Font Mono"). Leave blank to auto-detect.'
        >
          <input
            type="text"
            value={terminalFontFamily}
            placeholder="Auto-detect"
            onChange={(e) => void setTerminalFontFamily(e.target.value)}
            className="h-8 w-48 rounded-md border border-border bg-background px-2.5 text-[12px] outline-none focus:border-foreground/40"
          />
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
                <SelectItem key={size} value={String(size)} className="text-[12px]">
                  {size} px
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
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
      </div>

      <div className="flex flex-col gap-2">
        <Label>Pane colors</Label>
        <SettingRow
          title="Color mode"
          description="Manual uses the fixed per-type colors below. Automatic gives every new pane a distinct generated color from a palette."
        >
          <Select
            value={paneColorMode}
            onValueChange={(v) =>
              void setPaneColorMode(v as PaneColorMode)
            }
          >
            <SelectTrigger size="sm" className="h-8 w-32 text-[12px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="manual" className="text-[12px]">
                Manual
              </SelectItem>
              <SelectItem value="automatic" className="text-[12px]">
                Automatic
              </SelectItem>
            </SelectContent>
          </Select>
        </SettingRow>
        {paneColorMode === "automatic" ? (
          <SettingRow
            title="Palette"
            description="Hue family for generated pane colors."
          >
            <div className="flex items-center gap-2">
              <PalettePreview palette={paneColorPalette} />
              <Select
                value={paneColorPalette}
                onValueChange={(v) =>
                  void setPaneColorPalette(v as PaneColorPalette)
                }
              >
                <SelectTrigger size="sm" className="h-8 w-28 text-[12px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PALETTES.map((p) => (
                    <SelectItem
                      key={p.id}
                      value={p.id}
                      className="text-[12px]"
                    >
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </SettingRow>
        ) : (
          <>
            <ColorRow
              title="Terminal pane color"
              description="Default accent for new terminal panes (status dot and header)."
              value={paneColorTerminal}
              onChange={(v) => void setPaneColorTerminal(v)}
            />
            <ColorRow
              title="Notes pane color"
              description="Default accent for new notes panes."
              value={paneColorNotes}
              onChange={(v) => void setPaneColorNotes(v)}
            />
            <ColorRow
              title="Task pane color"
              description="Default accent for new task panes."
              value={paneColorTask}
              onChange={(v) => void setPaneColorTask(v)}
            />
          </>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <Label>Agents</Label>
        <SettingRow
          title="Coding agent notifications"
          description="Alert when Claude Code or Codex running in a terminal needs your input or finishes. Desktop notification when Koden is unfocused, in-app otherwise."
        >
          <Switch
            checked={agentNotifications}
            onCheckedChange={(v) => void setAgentNotifications(v)}
          />
        </SettingRow>
        <SettingRow
          title="Auto-retry on rate limit"
          description="When a Claude Code terminal hits its usage limit, wait until the reset time then resend a continue prompt (max 3 retries per terminal). This is the default for new agent terminals; toggle per tab from the Agents dock."
        >
          <Switch
            checked={autoRetryEnabled}
            onCheckedChange={(v) => void setAutoRetryEnabled(v)}
          />
        </SettingRow>
        <SettingRow
          title="Usage guard (proactive 5h limit)"
          description="Poll your 5-hour usage window in the background and warn as it fills, then pause starting new subagents before you hit the limit — instead of only reacting once a terminal is already rate-limited."
        >
          <Switch
            checked={usageGuardEnabled}
            onCheckedChange={(v) => void setUsageGuardEnabled(v)}
          />
        </SettingRow>
        {usageGuardEnabled && (
          <>
            <UsagePctInput
              title="Warn at"
              description="Percent of the 5-hour window used before showing a heads-up."
              value={usageGuardWarnPct}
              min={USAGE_WARN_MIN}
              max={USAGE_WARN_MAX}
              onChange={(v) => void setUsageGuardWarnPct(v)}
            />
            <UsagePctInput
              title="Pause at"
              description="Percent used before pausing the start of new subagents. Kept at or above the warn threshold."
              value={usageGuardPausePct}
              min={USAGE_PAUSE_MIN}
              max={USAGE_PAUSE_MAX}
              onChange={(v) => void setUsageGuardPausePct(v)}
            />
            <SettingRow
              title="Force-stop at limit (advanced)"
              description="When the pause threshold is reached, send Ctrl-C to armed agent terminals to interrupt them. Off by default; only the soft pause applies otherwise."
            >
              <Switch
                checked={usageGuardHardStop}
                onCheckedChange={(v) => void setUsageGuardHardStop(v)}
              />
            </SettingRow>
          </>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <Label>Startup</Label>
        <div className="flex flex-col gap-2">
          <SettingRow
            title="Launch at login"
            description="Open Koden automatically when you sign in."
          >
            <Switch
              checked={autostart}
              onCheckedChange={(v) => void onToggleAutostart(v)}
            />
          </SettingRow>
          <SettingRow
            title="Restore window position & size"
            description="Reopen the main window where you left it. Applies on next launch."
          >
            <Switch
              checked={restoreWindowState}
              onCheckedChange={(v) => void setRestoreWindowState(v)}
            />
          </SettingRow>
        </div>
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

function PalettePreview({ palette }: { palette: PaneColorPalette }) {
  // Fixed seed: a stable preview of the palette's hue family, not the live
  // session sequence. Five well-spread swatches read as a representative row.
  const swatches = [0, 1, 2, 3, 4].map((i) => paneColorAt(palette, i, 40));
  return (
    <span aria-hidden className="flex items-center gap-1">
      {swatches.map((c) => (
        <span
          key={c}
          className="size-3.5 rounded-full ring-1 ring-border"
          style={{ background: c }}
        />
      ))}
    </span>
  );
}

function ColorRow({
  title,
  description,
  value,
  onChange,
}: {
  title: string;
  description: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <SettingRow title={title} description={description}>
      <label className="flex items-center gap-2">
        <span
          aria-hidden
          className="size-4 rounded-full ring-1 ring-border"
          style={{ background: value }}
        />
        <input
          type="color"
          aria-label={title}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="h-8 w-12 cursor-pointer rounded-md border border-border bg-background p-0.5"
        />
      </label>
    </SettingRow>
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
        <div
          key={category}
          className="flex items-center justify-between gap-3"
        >
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
                <SelectItem
                  key={o.id}
                  value={o.id}
                  className="text-[12px]"
                >
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

function AutoSaveDelayInput({
  value,
  onChange,
}: {
  value: number;
  onChange: (v: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const commit = () => {
    const n = Number(draft);
    if (!Number.isFinite(n)) {
      setDraft(String(value));
      return;
    }
    const clamped = Math.min(
      AUTO_SAVE_MAX,
      Math.max(AUTO_SAVE_MIN, Math.round(n)),
    );
    setDraft(String(clamped));
    if (clamped !== value) onChange(clamped);
  };

  return (
    <SettingRow
      title="Auto save delay"
      description="Delay before unsaved changes are saved automatically."
    >
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={AUTO_SAVE_MIN}
          max={AUTO_SAVE_MAX}
          step={AUTO_SAVE_STEP}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.currentTarget.blur();
            }
          }}
          className="h-8 w-20 rounded-md border border-border bg-background px-2.5 text-right text-[12px] md:text-[12px] tabular-nums outline-none focus:border-foreground/40 focus-visible:ring-0 focus-visible:border-foreground/40 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
        />
        <span className="text-[11px] text-muted-foreground">ms</span>
      </div>
    </SettingRow>
  );
}

function UsagePctInput({
  title,
  description,
  value,
  min,
  max,
  onChange,
}: {
  title: string;
  description: string;
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const commit = () => {
    const n = Number(draft);
    if (!Number.isFinite(n)) {
      setDraft(String(value));
      return;
    }
    const clamped = Math.min(max, Math.max(min, Math.round(n)));
    setDraft(String(clamped));
    if (clamped !== value) onChange(clamped);
  };

  return (
    <SettingRow title={title} description={description}>
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={min}
          max={max}
          step={1}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.currentTarget.blur();
            }
          }}
          className="h-8 w-20 rounded-md border border-border bg-background px-2.5 text-right text-[12px] md:text-[12px] tabular-nums outline-none focus:border-foreground/40 focus-visible:ring-0 focus-visible:border-foreground/40 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
        />
        <span className="text-[11px] text-muted-foreground">%</span>
      </div>
    </SettingRow>
  );
}

