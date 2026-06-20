export type AgentStatus = "working" | "waiting";

export type AgentSource = "terminal" | "local";

export type AgentSignalKind =
  | "started"
  | "working"
  | "attention"
  | "finished"
  | "exited";

export type AgentSignal = {
  id: number;
  kind: AgentSignalKind;
  agent: string | null;
};

// Emitted by the Rust retry detector (pty/retry_detect.rs) when an armed
// claude session prints a usage-limit banner. `resetEpochMs` already includes
// the safety margin; RetryBridge waits until then, then resubmits.
export type RetrySignal = {
  id: number;
  resetEpochMs: number;
};

// Emitted by the Rust usage guard (pty/usage_detect.rs) when account-wide
// Claude usage crosses a configured threshold. Account-scoped, not per-leaf:
// `percentUsed` is the fraction of the current window consumed (0..100, or
// null when telemetry is unavailable), `resetEpochMs` is when the window
// resets, and `source` says whether the read came from a real endpoint or a
// time-based estimate. UsageBridge fires a one-shot warn/pause notification.
export type UsageSignal = {
  percentUsed: number | null;
  resetEpochMs: number | null;
  thresholdCrossed: "warn" | "pause" | null;
  source: "endpoint" | "time";
  telemetryLost: boolean;
};

export type AgentSession = {
  leafId: number;
  tabId: number;
  agent: string;
  status: AgentStatus;
  startedAt: number;
  lastActivityAt: number;
  attentionSince: number | null;
};

export type AgentNotification = {
  id: string;
  source: AgentSource;
  leafId: number;
  tabId: number;
  agent: string;
  kind: NotificationKind;
  at: number;
  read: boolean;
};

export type NotificationKind = "attention" | "finished" | "error";

export type LocalAgentState = {
  agent: string;
  status: AgentStatus;
} | null;
