import { useHandsFreeMode } from "@/modules/settings/preferences";
import { currentWorkspaceEnv } from "@/modules/workspace";
import { invoke } from "@tauri-apps/api/core";
import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import type { VoiceCaptureMeta, VoiceOrigin } from "../hooks/useVoiceCapture";
import { useVoiceHotkey } from "../hooks/useVoiceHotkey";
import { useWhisperRecording } from "../hooks/useWhisperRecording";
import { escActionFor, shouldRearmVoice } from "../hooks/voiceSession";
import { expandSnippetTokens, type Snippet } from "../lib/snippets";
import { getChat, useChatStore } from "../store/chatStore";
import { useSnippetsStore } from "../store/snippetsStore";
import { tryRunSlashCommand, type SlashCommandMeta } from "./slashCommands";

export type FileAttachment = {
  id: string;
  name: string;
  kind: "image" | "text" | "selection";
  mediaType: string;
  url?: string;
  text?: string;
  size: number;
  /** For kind === "selection": which surface it came from. */
  source?: "terminal" | "editor";
};

type MessagePart =
  | { type: "text"; text: string }
  | { type: "file"; mediaType: string; url: string; filename?: string };

export const MAX_TEXT_INLINE = 200_000;
export const ACCEPTED_FILES =
  "image/*,.txt,.md,.json,.yaml,.yml,.toml,.sh,.zsh,.bash,.py,.js,.jsx,.ts,.tsx,.rs,.go,.java,.c,.cpp,.h,.hpp,.html,.css,.csv,.log,.env,.config,.conf,.ini,Dockerfile,.dockerfile";

type Voice = ReturnType<typeof useWhisperRecording>;

type ComposerCtx = {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  value: string;
  setValue: React.Dispatch<React.SetStateAction<string>>;
  files: FileAttachment[];
  addFiles: (list: FileList | null) => Promise<void>;
  /** Attach a file by absolute path — used by the file explorer's "Attach to Agent". */
  attachFileByPath: (path: string) => Promise<void>;
  removeFile: (id: string) => void;
  pickedSnippets: Snippet[];
  addSnippet: (s: Snippet) => void;
  removeSnippet: (id: string) => void;
  pickedCommands: SlashCommandMeta[];
  addCommand: (c: SlashCommandMeta) => void;
  removeCommand: (name: string) => void;
  isBusy: boolean;
  /** Send the composer. `overrideText` replaces the typed value (voice auto-submit). */
  submit: (overrideText?: string) => void;
  stop: () => void;
  voice: Voice;
  /**
   * Mic affordance: starts a capture when idle, stops + transcribes when
   * listening (a manual stop also hard-stops the hands-free loop). Hotkey
   * origin starts a MANUAL take (no silence auto-stop — Wispr-style); mic
   * origin keeps the conversational auto mode.
   */
  voiceToggle: (origin?: Extract<VoiceOrigin, "mic" | "hotkey">) => void;
  /** Lane A's hands-free pref (voice captures auto-submit while armed). */
  handsFreeArmed: boolean;
  /** The continuous loop was hard-stopped (Esc / mic click) for now. */
  voiceSuspended: boolean;
  /**
   * Always-on VOICE SESSION (session-scoped state, never persisted): the mic
   * re-arms after every assistant turn until ended. Orthogonal to
   * handsFreeArmed, which keeps governing ONLY terminal-submit approvals
   * (ADR-017 addendum): listen-always ≠ approve-always.
   */
  voiceSessionActive: boolean;
  /**
   * Header mic toggle — the ONLY way the session starts (the hotkey tap is a
   * one-take gesture now): ON opens the Librarian window + listens
   * immediately. A take already recording is stopped + delivered instead of
   * arming the session (single-capture invariant).
   */
  voiceSessionToggle: () => void;
  /** End the session (Esc tier 2, window close, toggle off): tear down, no re-arm. */
  voiceSessionEnd: () => void;
  canSend: boolean;
};

const Ctx = createContext<ComposerCtx | null>(null);

export function useComposer(): ComposerCtx {
  const ctx = useContext(Ctx);
  if (!ctx)
    throw new Error("useComposer must be used inside <AiComposerProvider>");
  return ctx;
}

type ProviderProps = {
  children: React.ReactNode;
};

export function AiComposerProvider({ children }: ProviderProps) {
  const sessionId = useChatStore((s) => s.activeSessionId);
  const status = useChatStore((s) => s.agentMeta.status);
  const isBusy = status === "thinking" || status === "streaming";

  const [value, setValue] = useState("");
  const [files, setFiles] = useState<FileAttachment[]>([]);
  const [pickedSnippets, setPickedSnippets] = useState<Snippet[]>([]);
  const [pickedCommands, setPickedCommands] = useState<SlashCommandMeta[]>([]);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const focusSignal = useChatStore((s) => s.focusSignal);
  const pendingPrefill = useChatStore((s) => s.pendingPrefill);
  const consumePrefill = useChatStore((s) => s.consumePrefill);
  const pendingSelections = useChatStore((s) => s.pendingSelections);
  const consumeSelections = useChatStore((s) => s.consumeSelections);

  useEffect(() => {
    if (focusSignal === 0) return;
    textareaRef.current?.focus();
    if (pendingPrefill != null) {
      const text = consumePrefill();
      if (text) setValue((v) => (v ? `${text}${v}` : text));
    }
  }, [focusSignal, pendingPrefill, consumePrefill]);

  // Re-focus the textarea whenever the agent finishes a response
  const prevIsBusyRef = useRef(false);
  useEffect(() => {
    if (prevIsBusyRef.current && !isBusy) {
      requestAnimationFrame(() => textareaRef.current?.focus());
    }
    prevIsBusyRef.current = isBusy;
  }, [isBusy, textareaRef]);

  // Listen for explorer's "Attach to Agent" event.
  useEffect(() => {
    const onAttach = (e: Event) => {
      const path = (e as CustomEvent<string>).detail;
      if (typeof path === "string" && path.length > 0) {
        void attachFileByPath(path);
      }
    };
    window.addEventListener("koden:ai-attach-file", onAttach);
    return () => window.removeEventListener("koden:ai-attach-file", onAttach);
    // attachFileByPath is stable for our purposes (closes over setFiles only)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (pendingSelections.length === 0) return;
    const drained = consumeSelections();
    if (drained.length === 0) return;
    setFiles((prev) => {
      const existing = new Set(prev.map((f) => f.id));
      const next: FileAttachment[] = [];
      for (const sel of drained) {
        if (existing.has(sel.id)) continue;
        next.push({
          id: sel.id,
          name:
            sel.source === "editor"
              ? "Editor selection"
              : "Terminal selection",
          kind: "selection",
          mediaType: "text/plain",
          text: sel.text,
          size: sel.text.length,
          source: sel.source,
        });
      }
      return next.length ? [...prev, ...next] : prev;
    });
  }, [pendingSelections, consumeSelections]);

  const addFiles = async (list: FileList | null) => {
    if (!list) return;
    const next: FileAttachment[] = [];
    for (const f of Array.from(list)) {
      const att = await readAttachment(f);
      if (att) next.push(att);
    }
    if (next.length) setFiles((prev) => [...prev, ...next]);
  };

  const removeFile = (id: string) =>
    setFiles((prev) => prev.filter((f) => f.id !== id));

  const addSnippet = (s: Snippet) =>
    setPickedSnippets((prev) =>
      prev.some((p) => p.id === s.id) ? prev : [...prev, s],
    );
  const removeSnippet = (id: string) =>
    setPickedSnippets((prev) => prev.filter((s) => s.id !== id));

  const addCommand = (cmd: SlashCommandMeta) =>
    setPickedCommands((prev) =>
      prev.some((p) => p.name === cmd.name) ? prev : [...prev, cmd],
    );
  const removeCommand = (name: string) =>
    setPickedCommands((prev) => prev.filter((c) => c.name !== name));

  const attachFileByPath = async (path: string) => {
    try {
      type ReadResult =
        | { kind: "text"; content: string; size: number }
        | { kind: "binary"; size: number }
        | { kind: "toolarge"; size: number; limit: number };
      const result = await invoke<ReadResult>("fs_read_file", {
        path,
        workspace: currentWorkspaceEnv(),
      });
      if (result.kind !== "text") {
        // Binary/oversize files: skip (could surface a toast in future).
        console.warn("attachFileByPath: skipped non-text file", path, result);
        return;
      }
      const name = path.split("/").pop() || path;
      const id = `path-${path}`;
      setFiles((prev) => {
        if (prev.some((f) => f.id === id)) return prev;
        const att: FileAttachment = {
          id,
          name,
          kind: "text",
          mediaType: "text/plain",
          text: result.content,
          size: result.size,
        };
        return [...prev, att];
      });
      // Open the AI panel & focus the input so the user sees the chip.
      useChatStore.getState().focusInput();
    } catch (e) {
      console.error("attachFileByPath failed:", e);
    }
  };

  const submit = (overrideText?: string) => {
    if (isBusy) return;
    // Guard against event objects when passed as a handler directly.
    const source = typeof overrideText === "string" ? overrideText : value;
    const trimmed = source.trim();
    if (
      !trimmed &&
      files.length === 0 &&
      pickedSnippets.length === 0 &&
      pickedCommands.length === 0
    )
      return;

    // Slash-command interception. `/plan` toggles plan mode; `/init` rewrites
    // the prompt to the KODEN.md scan template before sending.
    let effectiveText = trimmed;
    let commandMarker: string | null = null;
    let commandSource = trimmed;
    if (pickedCommands.length > 0 && !trimmed.startsWith("/") && !trimmed.startsWith("#")) {
      commandSource = `#${pickedCommands[0].name} ${trimmed}`.trim();
    }
    if (commandSource.startsWith("/") || commandSource.startsWith("#")) {
      const outcome = tryRunSlashCommand(commandSource);
      if (outcome.kind === "handled") {
        setValue("");
        if (outcome.toast) console.info(outcome.toast);
        return;
      }
      if (outcome.kind === "send-prompt") {
        effectiveText = outcome.prompt;
        if (outcome.commandName) {
          commandMarker = `<koden-command name="${outcome.commandName}" />`;
        }
      }
    }

    const parts: MessagePart[] = [];
    const fileBlocks = files
      .filter((f) => f.kind === "text")
      .map(
        (f) =>
          `<file name="${f.name}" mediaType="${f.mediaType}">\n${f.text ?? ""}\n</file>`,
      );
    const selectionBlocks = files
      .filter((f) => f.kind === "selection")
      .map(
        (f) =>
          `<selection source="${f.source ?? "terminal"}">\n${f.text ?? ""}\n</selection>`,
      );
    const { body: bodyAfterTokens, blocks: snippetBlocks } = expandSnippetTokens(
      effectiveText,
      useSnippetsStore.getState().snippets,
    );
    const seenHandles = new Set<string>();
    const allSnippetBlocks: string[] = [];
    for (const s of pickedSnippets) {
      if (seenHandles.has(s.handle)) continue;
      seenHandles.add(s.handle);
      allSnippetBlocks.push(
        `<snippet name="${s.handle}">\n${s.content}\n</snippet>`,
      );
    }
    for (const block of snippetBlocks) {
      const m = block.match(/^<snippet name="([^"]+)"/);
      if (m && seenHandles.has(m[1])) continue;
      if (m) seenHandles.add(m[1]);
      allSnippetBlocks.push(block);
    }
    const composed = [
      commandMarker ?? "",
      allSnippetBlocks.join("\n\n"),
      selectionBlocks.join("\n\n"),
      fileBlocks.join("\n\n"),
      bodyAfterTokens,
    ]
      .filter(Boolean)
      .join("\n\n");
    if (composed) parts.push({ type: "text", text: composed });

    for (const f of files) {
      if (f.kind === "image" && f.url) {
        parts.push({
          type: "file",
          mediaType: f.mediaType,
          url: f.url,
          filename: f.name,
        });
      }
    }

    if (!sessionId) return;
    const store = useChatStore.getState();
    store.patchAgentMeta({ hitStepCap: false, compactionNotice: null });
    if (!store.mini.open) store.openMini();
    void (async () => {
      const { getOrCreateChat } = await import("../store/chatRuntime");
      const chat = getOrCreateChat(sessionId);
      void chat.sendMessage({ role: "user", parts } as Parameters<
        typeof chat.sendMessage
      >[0]);
    })();
    setValue("");
    setFiles([]);
    setPickedSnippets([]);
    setPickedCommands([]);
    // Re-focus immediately after submit so the user can type a follow-up
    requestAnimationFrame(() => textareaRef.current?.focus());
  };

  const stop = () => {
    if (!sessionId) return;
    void getChat(sessionId)?.stop();
  };

  // ── Voice ──────────────────────────────────────────────────────────────
  const handsFreeArmed = useHandsFreeMode();
  const miniOpen = useChatStore((s) => s.mini.open);
  const agentStatus = useChatStore((s) => s.agentMeta.status);
  // Hard-stop latch for the continuous loop (Esc / mic click). Resets when
  // the user starts voice again, re-opens the window, or re-arms the pref.
  const [voiceSuspended, setVoiceSuspended] = useState(false);
  // VOICE SESSION: session-scoped listen loop, deliberately NOT a persisted
  // pref — listening is convenience, approvals are policy (handsFreeArmed).
  const [voiceSessionActive, setVoiceSessionActive] = useState(false);
  const voiceSessionRef = useRef(voiceSessionActive);
  voiceSessionRef.current = voiceSessionActive;
  // Mirror for close-transition effects: reading the live flag through a ref
  // keeps those effects keyed on the transition, not on recording churn.
  const voiceRecordingRef = useRef(false);

  const voice = useWhisperRecording({
    // useVoiceCapture routes results through a latest-ref, so `value`,
    // `isBusy` and `submit` here are the current render's.
    onResult: (transcript: string, meta: VoiceCaptureMeta) => {
      if (meta.autoSubmit && !isBusy && sessionId) {
        const drafted = value.trim();
        submit(drafted ? `${drafted} ${transcript}` : transcript);
        return;
      }
      setValue((v) => (v ? `${v} ${transcript}` : transcript));
      requestAnimationFrame(() => textareaRef.current?.focus());
    },
  });
  const {
    recording: voiceRecording,
    transcribing: voiceTranscribing,
    cancel: voiceCancel,
    clearError: voiceClearError,
    error: voiceError,
  } = voice;
  voiceRecordingRef.current = voiceRecording;

  const voiceToggle = (origin: "mic" | "hotkey" = "mic") => {
    if (voice.recording) {
      // Manual stop = hard stop for the hands-free loop; the take still
      // transcribes (Esc is the discard path). A live session ignores the
      // latch and re-arms after the turn.
      setVoiceSuspended(true);
      voice.stop();
      return;
    }
    if (voice.transcribing) return;
    setVoiceSuspended(false);
    void voice.start({
      origin,
      // PTT is the hands-free gesture (speak → she acts). Mic clicks only
      // auto-submit while the hands-free pref or a voice session is armed;
      // otherwise they keep the legacy dictate-into-composer behavior.
      autoSubmit: origin === "hotkey" || handsFreeArmed || voiceSessionActive,
      // The hotkey starts a Wispr-style MANUAL take: pauses never end it —
      // the second tap (or a hold's release) does. Mic clicks and the
      // session loop keep the conversational silence auto-stop.
      mode: origin === "hotkey" ? "manual" : "auto",
    });
  };

  const voiceSessionEnd = () => {
    setVoiceSessionActive(false);
    // Unconditional: a start() awaiting getUserMedia (permission prompt)
    // still reads "idle" — cancel sets the flag its raced-start branch
    // checks, so the pending capture never goes hot. No-op when truly idle
    // (the next start resets the flag).
    voice.cancel();
  };

  const voiceSessionStart = () => {
    if (!voice.supported || !voice.hasKey) return;
    // Single-capture invariant: a take already in flight (hotkey manual take,
    // mic dictation) means this click stops + delivers THAT take — it neither
    // arms the session nor opens a second capture.
    if (voice.recording) {
      voice.stop();
      return;
    }
    setVoiceSessionActive(true);
    setVoiceSuspended(false);
    // Reach: the toggle works with the Librarian window closed — open it
    // (the header button's openMini path; Mod+I drives the docked panel).
    const store = useChatStore.getState();
    if (!store.mini.open) store.openMini();
    // Listen immediately. When a capture is live or the agent is mid-turn,
    // the post-turn re-arm picks the loop up instead — a capture started
    // while she talks would just no-speech-cancel or leave a stray draft.
    if (voice.state === "idle" && !isBusy) {
      void voice.start({ origin: "auto", autoSubmit: true });
    }
  };

  const voiceSessionToggle = () => {
    if (voiceSessionActive) voiceSessionEnd();
    else voiceSessionStart();
  };

  useVoiceHotkey({
    enabled: voice.supported && voice.hasKey,
    recording: voiceRecording,
    transcribing: voiceTranscribing,
    onStart: () => {
      // Reach: the chord works with the Librarian window closed — open it so
      // the capture has a surface. No mount handshake needed: the capture
      // machinery + these listeners live in this app-level provider, not the
      // lazy mini window, so starting in the same tick is race-free.
      const store = useChatStore.getState();
      if (!store.mini.open) store.openMini();
      voiceToggle("hotkey");
    },
    // PTT release / second tap ends the utterance without suspending the
    // loop. A quick tap needs no wiring of its own: its manual take simply
    // keeps recording (the voice SESSION is the header mic's job only now).
    onStop: voice.stop,
  });

  // Unified Esc tiering (capture phase beats AiMiniWindow's close handler):
  // Esc while a capture is live discards the TAKE (and pauses the legacy
  // loop); the next Esc — or Esc between captures — ends the SESSION; with
  // neither, Esc falls through to the window's own close handler.
  useEffect(() => {
    const capturing = voiceRecording || voiceTranscribing;
    if (!capturing && !voiceSessionActive) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const action = escActionFor({
        capturing,
        sessionActive: voiceSessionActive,
      });
      if (action === "cancel-capture") {
        e.preventDefault();
        e.stopImmediatePropagation();
        setVoiceSuspended(true);
        voiceCancel();
        return;
      }
      if (action !== "end-session") return;
      // Inputs keep their own Esc meaning (pickers, search); the composer
      // textarea handles this tier itself in AiComposerInput.
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      setVoiceSessionActive(false);
      // Same as voiceSessionEnd: catch a start() still awaiting the mic.
      voiceCancel();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, { capture: true });
  }, [voiceRecording, voiceTranscribing, voiceSessionActive, voiceCancel]);

  // Continuous mode: re-arm the mic after each assistant turn completes
  // (talk → she acts → talk again). Session lane re-arms regardless of the
  // hands-free pref; legacy lane preserves armed + open + not-suspended.
  // No dep array: the transition guard makes it fire once per completion.
  const prevAgentStatusRef = useRef(agentStatus);
  useEffect(() => {
    const prev = prevAgentStatusRef.current;
    prevAgentStatusRef.current = agentStatus;
    const rearm = shouldRearmVoice({
      prevStatus: prev,
      status: agentStatus,
      sessionActive: voiceSessionActive,
      handsFreeArmed,
      miniOpen,
      suspended: voiceSuspended,
      captureState: voice.state,
      supported: voice.supported,
      hasKey: voice.hasKey,
      hasDraft: value.trim().length > 0,
      windowFocused:
        typeof document === "undefined" ? false : document.hasFocus(),
    });
    if (rearm) void voice.start({ origin: "auto", autoSubmit: true });
  });

  useEffect(() => {
    if (miniOpen) {
      setVoiceSuspended(false);
      return;
    }
    // Closing the Librarian window ends the session: stop capture, tear
    // down, no re-arm.
    if (voiceSessionRef.current) {
      setVoiceSessionActive(false);
      voiceCancel();
      return;
    }
    // A live non-session take (manual tap / mic dictation) is stopped and
    // DELIVERED on panel close — a hot mic with no panel is ambiguous even
    // with the header pulse, and the user's words shouldn't be discarded.
    if (voiceRecordingRef.current) voice.stop();
  }, [miniOpen, voiceCancel, voice.stop]);
  useEffect(() => {
    if (handsFreeArmed) setVoiceSuspended(false);
  }, [handsFreeArmed]);

  // A denied mic would otherwise retry every turn — suspend the legacy loop
  // and end the session until re-armed.
  useEffect(() => {
    if (voiceError?.kind !== "permission") return;
    setVoiceSuspended(true);
    setVoiceSessionActive(false);
  }, [voiceError]);

  // Voice errors are transient UI — clear after a beat.
  useEffect(() => {
    if (!voiceError) return;
    const t = window.setTimeout(() => voiceClearError(), 6000);
    return () => window.clearTimeout(t);
  }, [voiceError, voiceClearError]);
  // ───────────────────────────────────────────────────────────────────────

  const canSend =
    !isBusy &&
    (value.trim().length > 0 ||
      files.length > 0 ||
      pickedSnippets.length > 0 ||
      pickedCommands.length > 0);

  const ctx: ComposerCtx = {
    textareaRef,
    value,
    setValue,
    files,
    addFiles,
    attachFileByPath,
    removeFile,
    pickedSnippets,
    addSnippet,
    removeSnippet,
    pickedCommands,
    addCommand,
    removeCommand,
    isBusy,
    submit,
    stop,
    voice,
    voiceToggle,
    handsFreeArmed,
    voiceSuspended,
    voiceSessionActive,
    voiceSessionToggle,
    voiceSessionEnd,
    canSend,
  };

  return <Ctx.Provider value={ctx}>{children}</Ctx.Provider>;
}

async function readAttachment(file: File): Promise<FileAttachment | null> {
  const id = `${file.name}-${file.size}-${file.lastModified}`;
  if (file.type.startsWith("image/")) {
    const url = await readAsDataURL(file);
    return {
      id,
      name: file.name,
      kind: "image",
      mediaType: file.type || "image/png",
      url,
      size: file.size,
    };
  }
  if (file.size > MAX_TEXT_INLINE) return null;
  const text = await file.text();
  return {
    id,
    name: file.name,
    kind: "text",
    mediaType: file.type || "text/plain",
    text,
    size: file.size,
  };
}

function readAsDataURL(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}
