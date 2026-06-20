import {
  CommandLineIcon,
  HierarchySquare01Icon,
  Layout01Icon,
  Robot01Icon,
  Settings02Icon,
  ShieldEnergyIcon,
  SourceCodeIcon,
  TestTube01Icon,
} from "@hugeicons/core-free-icons";
import type { HugeiconsIcon } from "@hugeicons/react";
import type { AgentRole, AgentStatus } from "./types";

type IconRef = Parameters<typeof HugeiconsIcon>[0]["icon"];

/** Tier governs vertical placement in the topology view (0 = top). */
export const ROLE_META: Record<
  AgentRole,
  { icon: IconRef; tier: number; accent: string }
> = {
  director: { icon: CommandLineIcon, tier: 0, accent: "var(--primary)" },
  architect: { icon: Layout01Icon, tier: 1, accent: "#a78bfa" },
  coder: { icon: SourceCodeIcon, tier: 2, accent: "#60a5fa" },
  reviewer: { icon: HierarchySquare01Icon, tier: 2, accent: "#34d399" },
  auditor: { icon: ShieldEnergyIcon, tier: 2, accent: "#fbbf24" },
  qa: { icon: TestTube01Icon, tier: 2, accent: "#f472b6" },
  devops: { icon: Settings02Icon, tier: 2, accent: "#22d3ee" },
  worker: { icon: Robot01Icon, tier: 3, accent: "#94a3b8" },
};

// Distinct hue per state so "working" never reads like "done": in-progress
// states are cool/pulsing (cyan spawning, sky working, violet reviewing),
// attention is amber, and the settled "done" is the only green.
export const STATUS_META: Record<
  AgentStatus,
  { label: string; dot: string; pulse: boolean }
> = {
  spawning: { label: "Spawning", dot: "#22d3ee", pulse: true },
  // Nothing running in this terminal (a plain shell, or just opened): grey.
  idle: { label: "Idle", dot: "#94a3b8", pulse: false },
  // The agent finished its turn and is waiting for YOU to type: green = ready.
  ready: { label: "Ready", dot: "#22c55e", pulse: false },
  working: { label: "Working", dot: "#38bdf8", pulse: true },
  reviewing: { label: "Reviewing", dot: "#a78bfa", pulse: true },
  // Needs a decision (a permission prompt / question): orange + pulsing.
  waiting: { label: "Needs you", dot: "#f97316", pulse: true },
  blocked: { label: "Blocked", dot: "#fb923c", pulse: false },
  done: { label: "Done", dot: "#22c55e", pulse: false },
  // Red is reserved for an actual error now, nothing else.
  error: { label: "Error", dot: "#ef4444", pulse: false },
};

export function roleAccent(role: AgentRole): string {
  return ROLE_META[role].accent;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatRelativeTime(ts: number, nowMs: number): string {
  const s = Math.max(0, Math.round((nowMs - ts) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}
