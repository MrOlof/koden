import { describe, expect, it } from "vitest";
import { formatRelativeTime, formatTokens } from "./roleMeta";

describe("formatTokens", () => {
  it("formats raw, thousands and millions", () => {
    expect(formatTokens(42)).toBe("42");
    expect(formatTokens(1500)).toBe("1.5k");
    expect(formatTokens(2_300_000)).toBe("2.3M");
  });
});

describe("formatRelativeTime", () => {
  const now = 1_000_000_000;
  it("buckets recent times", () => {
    expect(formatRelativeTime(now, now)).toBe("just now");
    expect(formatRelativeTime(now - 30_000, now)).toBe("30s ago");
    expect(formatRelativeTime(now - 5 * 60_000, now)).toBe("5m ago");
    expect(formatRelativeTime(now - 3 * 3_600_000, now)).toBe("3h ago");
    expect(formatRelativeTime(now - 2 * 86_400_000, now)).toBe("2d ago");
  });
});
