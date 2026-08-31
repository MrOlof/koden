import { describe, expect, it } from "vitest";
import {
  keyFromWindowName,
  livenessHint,
  planAdoption,
  windowNameForKey,
} from "./remoteSessions";

describe("windowNameForKey (parity with Rust tmux_window_name)", () => {
  // Shared vectors with shell_ssh.rs::tmux_window_name_is_sanitised.
  it("maps keys the way the Rust side names windows", () => {
    expect(windowNameForKey("rk-mthk-u0pjur")).toBe("w-rk-mthk-u0pjur");
    expect(windowNameForKey("My Pane: v2")).toBe("w-My-Pane-v2");
    expect(windowNameForKey("...")).toBe("w-pane");
    expect(windowNameForKey("")).toBe("w-pane");
    expect(windowNameForKey("x".repeat(200)).length).toBeLessThanOrEqual(48);
  });

  it("round-trips a real restore key through the window name", () => {
    const key = "rk-mthn5svi-3wk0w9";
    expect(keyFromWindowName(windowNameForKey(key))).toBe(key);
  });
});

describe("keyFromWindowName", () => {
  it("rejects foreign and malformed window names", () => {
    expect(keyFromWindowName("bash")).toBeNull();
    expect(keyFromWindowName("w-")).toBeNull();
    expect(keyFromWindowName("w-has space")).toBeNull();
    expect(keyFromWindowName(`w-${"x".repeat(65)}`)).toBeNull();
  });
});

describe("planAdoption", () => {
  const windows = [
    { name: "w-rk-live1", command: "claude", path: "/home/k/proj" },
    { name: "w-rk-owned", command: "bash", path: "/home/k" },
    { name: "manual-window", command: "htop", path: "/tmp" },
    { name: "w-rk-noPath", command: "", path: "relative/junk" },
  ];

  it("adopts unowned koden windows only, with command title and posix cwd", () => {
    const plan = planAdoption(windows, new Set(["rk-owned"]));
    expect(plan).toEqual([
      { key: "rk-live1", title: "claude", cwd: "/home/k/proj" },
      { key: "rk-noPath", title: "session" },
    ]);
  });

  it("adopts nothing when every window is owned locally", () => {
    const plan = planAdoption(
      [windows[0]],
      new Set(["rk-live1"]),
    );
    expect(plan).toEqual([]);
  });
});

describe("livenessHint", () => {
  it("shows only positive counts", () => {
    expect(livenessHint(null)).toBeNull();
    expect(livenessHint(undefined)).toBeNull();
    expect(livenessHint(0)).toBeNull();
    expect(livenessHint(1)).toBe("● 1 live session");
    expect(livenessHint(4)).toBe("● 4 live sessions");
  });
});
