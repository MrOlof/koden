import { Popover, PopoverAnchor } from "@/components/ui/popover";
import { Spinner } from "@/components/ui/spinner";
import { usePresence } from "@/lib/usePresence";
import { cn } from "@/lib/utils";
import { useShortcutLabel } from "@/modules/shortcuts/lib/useShortcutLabel";
import { Mic01Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useWorkspaceFiles } from "../hooks/useWorkspaceFiles";
import { useComposer } from "../lib/composer";
import { SLASH_COMMANDS } from "../lib/slashCommands";
import { VOICE_NEEDS_KEY_MESSAGE } from "../lib/whisperTranscribe";
import { useChatStore } from "../store/chatStore";
import { useSnippetsStore } from "../store/snippetsStore";
import { FilePickerContent } from "./FilePicker";
import { type PickerItem, SnippetPickerContent } from "./SnippetPicker";

type SnippetTrigger = {
  start: number;
  end: number;
  query: string;
  char: "#" | "/";
};

type FileTrigger = {
  start: number;
  end: number;
  query: string;
};

function detectSnippetTrigger(
  value: string,
  caret: number,
): SnippetTrigger | null {
  for (let i = caret - 1; i >= 0; i--) {
    const ch = value[i];
    if (ch === "#" || ch === "/") {
      const prev = i === 0 ? " " : value[i - 1];
      if (!/\s/.test(prev)) return null;
      const slice = value.slice(i + 1, caret);
      if (!/^[a-z0-9-]*$/i.test(slice)) return null;
      return { start: i, end: caret, query: slice.toLowerCase(), char: ch };
    }
    if (/\s/.test(ch)) return null;
    if (!/[a-z0-9-]/i.test(ch)) return null;
  }
  return null;
}

function detectFileTrigger(value: string, caret: number): FileTrigger | null {
  for (let i = caret - 1; i >= 0; i--) {
    const ch = value[i];
    if (ch === "@") {
      const prev = i === 0 ? " " : value[i - 1];
      if (!/\s/.test(prev)) return null;
      const slice = value.slice(i + 1, caret);
      return { start: i, end: caret, query: slice };
    }
    if (/\s/.test(ch)) return null;
  }
  return null;
}

export function AiComposerInput({ withMic = false }: { withMic?: boolean }) {
  const c = useComposer();
  const snippets = useSnippetsStore((s) => s.snippets);
  const workspaceRoot = useChatStore((s) => s.live.getWorkspaceRoot());

  const [trigger, setTrigger] = useState<SnippetTrigger | null>(null);
  const [fileTrigger, setFileTrigger] = useState<FileTrigger | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const workspaceFiles = useWorkspaceFiles(workspaceRoot, fileTrigger !== null);

  const [fileQuery, setFileQuery] = useState("");
  useEffect(() => {
    if (!fileTrigger) {
      setFileQuery("");
      return;
    }
    const q = fileTrigger.query;
    const t = window.setTimeout(() => setFileQuery(q), 50);
    return () => window.clearTimeout(t);
  }, [fileTrigger]);

  useEffect(() => {
    autoresize(c.textareaRef.current);
  }, [c.value, c.textareaRef]);

  const updateTrigger = () => {
    const el = c.textareaRef.current;
    if (!el) {
      setTrigger(null);
      setFileTrigger(null);
      return;
    }
    const caret = el.selectionStart ?? 0;
    setTrigger(detectSnippetTrigger(c.value, caret));
    setFileTrigger(detectFileTrigger(c.value, caret));
  };

  useEffect(updateTrigger, [c.value, c.textareaRef]);

  const filteredItems = useMemo<PickerItem[]>(() => {
    if (!trigger) return [];
    const q = trigger.query;
    const cmdItems: PickerItem[] = Object.values(SLASH_COMMANDS)
      .filter(
        (c) => !q || c.name.includes(q) || c.label.toLowerCase().includes(q),
      )
      .map((command) => ({ kind: "command", command }));
    if (trigger.char === "/") return cmdItems;
    const snipItems: PickerItem[] = snippets
      .filter(
        (s) =>
          !q ||
          s.handle.includes(q) ||
          s.name.toLowerCase().includes(q) ||
          s.description.toLowerCase().includes(q),
      )
      .map((snippet) => ({ kind: "snippet", snippet }));
    return [...cmdItems, ...snipItems];
  }, [trigger, snippets]);

  const FILE_PICKER_CAP = 30;
  const filteredFiles = useMemo<string[]>(() => {
    if (!fileTrigger) return [];
    const q = fileQuery.toLowerCase();
    if (!q) return workspaceFiles.files.slice(0, FILE_PICKER_CAP);
    const out: string[] = [];
    for (const f of workspaceFiles.files) {
      if (f.toLowerCase().includes(q)) {
        out.push(f);
        if (out.length >= FILE_PICKER_CAP) break;
      }
    }
    return out;
  }, [fileTrigger, fileQuery, workspaceFiles.files]);

  const fileTriggerOpen = fileTrigger !== null;
  const snippetTriggerOpen = trigger !== null;
  useEffect(() => {
    setActiveIndex(0);
  }, [snippetTriggerOpen, fileTriggerOpen, fileQuery]);

  const pickerOpen = trigger !== null || fileTrigger !== null;

  const onPickItem = (item: PickerItem) => {
    if (!trigger) return;
    const before = c.value.slice(0, trigger.start);
    const afterRaw = c.value.slice(trigger.end);
    let insert = "";
    if (item.kind === "snippet") {
      const needsSpace = afterRaw.length === 0 || !/^\s/.test(afterRaw);
      insert = `#${item.snippet.handle}${needsSpace ? " " : ""}`;
      c.addSnippet(item.snippet);
    } else {
      c.addCommand(item.command);
    }
    const after =
      item.kind === "command" ? afterRaw.replace(/^\s+/, "") : afterRaw;
    c.setValue(`${before}${insert}${after}`);
    setTrigger(null);
    setActiveIndex(0);
    requestAnimationFrame(() => {
      const el = c.textareaRef.current;
      if (!el) return;
      const caret = before.length + insert.length;
      el.focus();
      el.setSelectionRange(caret, caret);
    });
  };

  const onPickFile = async (filePath: string) => {
    if (!fileTrigger || !workspaceRoot) return;
    const before = c.value.slice(0, fileTrigger.start);
    const after = c.value.slice(fileTrigger.end);
    c.setValue(`${before}${after}`);
    setFileTrigger(null);
    setActiveIndex(0);
    const fullPath = workspaceRoot.endsWith("/")
      ? `${workspaceRoot}${filePath}`
      : `${workspaceRoot}/${filePath}`;
    await c.attachFileByPath(fullPath);
    requestAnimationFrame(() => {
      const el = c.textareaRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(before.length, before.length);
    });
  };

  const pickActive = () => {
    if (fileTrigger) {
      const file = filteredFiles[activeIndex];
      if (file) void onPickFile(file);
      return;
    }
    const it = filteredItems[activeIndex];
    if (it) onPickItem(it);
  };

  const voiceShortcut = useShortcutLabel("ai.voiceInput");
  // "hands-free" tracks the approvals PREF (the honest armed signal,
  // ADR-017), not autoSubmit — session captures auto-submit too. A MANUAL
  // take (hotkey tap, Wispr-style) gets its own copy: nothing auto-stops it,
  // so say what ends it — rebind-aware via useShortcutLabel.
  const voiceLabel = c.voice.recording
    ? c.voice.meta?.mode === "manual"
      ? `Recording — tap ${voiceShortcut || "the voice key"} to send`
      : c.handsFreeArmed
        ? "Listening — hands-free…"
        : "Listening…"
    : c.voice.transcribing
      ? "Transcribing…"
      : (c.voice.error?.message ??
        (c.voiceSessionActive ? "Voice session on" : null));
  const voiceRow = usePresence(Boolean(voiceLabel), 180);
  const lastVoiceLabel = useRef("");
  if (voiceLabel) lastVoiceLabel.current = voiceLabel;
  const voiceErrorTone =
    c.voice.error && c.voice.error.kind !== "no-speech"
      ? "text-destructive"
      : "text-muted-foreground";

  const micTitle = !c.voice.hasKey
    ? VOICE_NEEDS_KEY_MESSAGE
    : c.voice.recording
      ? "Stop & transcribe (Esc discards)"
      : c.voice.transcribing
        ? "Transcribing…"
        : `Voice input${c.handsFreeArmed ? " — hands-free armed" : ""}${
            voiceShortcut
              ? ` (${voiceShortcut} tap = one take, hold = push-to-talk)`
              : ""
          }`;

  return (
    <>
      <Popover open={pickerOpen}>
        <PopoverAnchor asChild>
          <div className="flex items-start gap-2">
            <textarea
              ref={c.textareaRef}
              value={c.value}
              onChange={(e) => c.setValue(e.target.value)}
              onKeyUp={updateTrigger}
              onClick={updateTrigger}
              onSelect={updateTrigger}
              onPaste={(e) => {
                // Ctrl+V is agnostic to clipboard type: image data is attached
                // (Windows screenshot, copied image, etc.), text falls through to
                // the default textarea paste. Paste events deliver images even in
                // WebView2 where programmatic clipboard reads are blocked.
                const items = e.clipboardData?.items;
                if (!items) return;
                const images: File[] = [];
                for (const it of Array.from(items)) {
                  if (it.kind === "file" && it.type.startsWith("image/")) {
                    const f = it.getAsFile();
                    if (f) images.push(f);
                  }
                }
                if (images.length === 0) return;
                e.preventDefault();
                const dt = new DataTransfer();
                for (const f of images) dt.items.add(f);
                void c.addFiles(dt.files);
              }}
              onKeyDown={(e) => {
                if (pickerOpen) {
                  const items = fileTrigger ? filteredFiles : filteredItems;
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setActiveIndex((i) =>
                      Math.min(i + 1, Math.max(0, items.length - 1)),
                    );
                    return;
                  }
                  if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setActiveIndex((i) => Math.max(0, i - 1));
                    return;
                  }
                  if (e.key === "Tab" || e.key === "Enter") {
                    if (items.length > 0) {
                      e.preventDefault();
                      pickActive();
                      return;
                    }
                  }
                  if (e.key === "Escape") {
                    e.preventDefault();
                    if (fileTrigger) {
                      const before = c.value.slice(0, fileTrigger.start);
                      const after = c.value.slice(fileTrigger.end);
                      c.setValue(`${before}${after}`);
                      setFileTrigger(null);
                    } else {
                      setTrigger(null);
                    }
                    return;
                  }
                }
                if (
                  e.key === "Escape" &&
                  c.voiceSessionActive &&
                  !c.voice.recording &&
                  !c.voice.transcribing
                ) {
                  // Esc tier 2 from the textarea: end the SESSION. (Tier 1 —
                  // discarding a live take — is the provider's capture-phase
                  // handler; the window's Esc-close ignores textareas.)
                  e.preventDefault();
                  c.voiceSessionEnd();
                  return;
                }
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  c.submit();
                }
              }}
              placeholder="Ask the Librarian about your projects   -   # for commands, @ for files"
              rows={1}
              className={cn(
                "max-h-40 flex-1 resize-none bg-transparent text-[13px] leading-relaxed outline-none",
                "placeholder:text-muted-foreground/60",
              )}
            />
            {withMic && c.voice.supported && (
              <button
                type="button"
                onClick={() => c.voiceToggle("mic")}
                disabled={c.isBusy || c.voice.transcribing || !c.voice.hasKey}
                title={micTitle}
                aria-label="Voice input"
                className={cn(
                  "relative mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md transition-colors",
                  "text-muted-foreground hover:bg-accent hover:text-foreground",
                  // No pointer-events-none: the disabled button keeps its
                  // native title so the no-key state stays explained.
                  "disabled:opacity-50",
                  c.voice.recording &&
                    "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary",
                )}
              >
                {c.voice.recording ? (
                  <span className="size-2 animate-pulse rounded-full bg-primary" />
                ) : c.voice.transcribing ? (
                  <Spinner className="size-3" />
                ) : (
                  <HugeiconsIcon icon={Mic01Icon} size={13} strokeWidth={1.75} />
                )}
                {(c.handsFreeArmed || c.voiceSessionActive) &&
                  !c.voice.recording && (
                    <span
                      aria-hidden
                      className="absolute right-0.5 top-0.5 size-1 rounded-full bg-primary"
                    />
                  )}
              </button>
            )}
          </div>
        </PopoverAnchor>
        {fileTrigger ? (
          <FilePickerContent
            files={filteredFiles}
            activeIndex={activeIndex}
            indexing={workspaceFiles.indexing}
            truncated={workspaceFiles.truncated}
            hasWorkspace={workspaceRoot !== null}
            onPick={(f) => void onPickFile(f)}
            onHover={setActiveIndex}
          />
        ) : (
          <SnippetPickerContent
            items={filteredItems}
            activeIndex={activeIndex}
            onPick={onPickItem}
            onHover={setActiveIndex}
          />
        )}
      </Popover>

      {voiceRow.mounted && (
        <div data-state={voiceRow.state} className="koden-reveal">
          <div
            className={cn(
              "flex items-center gap-1.5 px-1 text-[11px]",
              c.voice.error ? voiceErrorTone : "text-muted-foreground",
            )}
          >
            {c.voice.recording ? (
              <span className="size-1.5 animate-pulse rounded-full bg-primary" />
            ) : c.voice.transcribing ? (
              <Spinner className="size-3" />
            ) : c.voiceSessionActive && !c.voice.error ? (
              // Session on, between captures — subtle, no pulse.
              <span className="size-1.5 rounded-full bg-primary/60" />
            ) : null}
            <span className="truncate">
              {voiceLabel || lastVoiceLabel.current}
            </span>
          </div>
        </div>
      )}
    </>
  );
}

function autoresize(el: HTMLTextAreaElement | null) {
  if (!el) return;
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
}
