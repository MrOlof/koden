import { describe, expect, it } from "vitest";
import { DormantRing } from "./dormantRing";
import { RESTORE_SEPARATOR, replayFirstBind } from "./restoreReplay";

function fakeTerm() {
  const writes: string[] = [];
  const dec = new TextDecoder();
  return {
    writes,
    write(data: string | Uint8Array) {
      writes.push(typeof data === "string" ? data : dec.decode(data));
    },
  };
}

function ringWith(...chunks: string[]): DormantRing {
  const ring = new DormantRing();
  const enc = new TextEncoder();
  for (const c of chunks) ring.push(enc.encode(c));
  return ring;
}

describe("replayFirstBind", () => {
  it("writes restored scrollback, then the separator, then ring bytes from the PTY", () => {
    const term = fakeTerm();
    const ring = ringWith("PS C:\\proj> ", "dir\r\n");
    replayFirstBind(term, {
      restored: "old line 1\r\nold line 2",
      snapshot: null,
      altScreen: false,
      drainRing: (w) => ring.drain(w),
    });
    expect(term.writes[0]).toBe(`old line 1\r\nold line 2${RESTORE_SEPARATOR}`);
    expect(term.writes.slice(1).join("")).toBe("PS C:\\proj> dir\r\n");
    // The PTY's first output never precedes the restored text.
    expect(term.writes.join("").indexOf("old line 1")).toBeLessThan(
      term.writes.join("").indexOf("PS C:"),
    );
  });

  it("puts a same-launch snapshot after the restored text and before the ring", () => {
    const term = fakeTerm();
    const ring = ringWith("tail");
    replayFirstBind(term, {
      restored: "restored",
      snapshot: "snap",
      altScreen: false,
      drainRing: (w) => ring.drain(w),
    });
    expect(term.writes).toEqual([`restored${RESTORE_SEPARATOR}`, "snap", "tail"]);
  });

  it("emits no separator when there is nothing restored", () => {
    const term = fakeTerm();
    const ring = ringWith("prompt> ");
    replayFirstBind(term, {
      restored: null,
      snapshot: null,
      altScreen: false,
      drainRing: (w) => ring.drain(w),
    });
    expect(term.writes).toEqual(["prompt> "]);
    expect(term.writes.join("")).not.toContain("[restored]");
  });

  it("discards ring bytes in alt-screen so a TUI repaint never replays over the snapshot", () => {
    const term = fakeTerm();
    const ring = ringWith("\x1b[2J\x1b[H incremental repaint");
    replayFirstBind(term, {
      restored: "restored",
      snapshot: null,
      altScreen: true,
      drainRing: (w) => ring.drain(w),
    });
    expect(term.writes).toEqual([`restored${RESTORE_SEPARATOR}`]);
    expect(ring.byteLength()).toBe(0);
  });

  it("separator parks the cursor at the bottom row before the marker line", () => {
    expect(RESTORE_SEPARATOR.startsWith("\x1b[0m\x1b[999B\r\n")).toBe(true);
    expect(RESTORE_SEPARATOR.endsWith("[restored]\x1b[0m\r\n")).toBe(true);
  });
});
