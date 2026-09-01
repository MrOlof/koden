import { describe, expect, it } from "vitest";
import { mergeDocs } from "./mergeDocs";

const note = (content: string, updatedAt: number) => ({ content, updatedAt });
const empty = { notes: {}, boards: {}, tasks: {} };

describe("mergeDocs", () => {
  it("adopts entries missing locally and newer remotely", () => {
    const local = {
      ...empty,
      notes: { a: note("old", 100), b: note("mine", 500) },
    };
    const remote = {
      ...empty,
      notes: { a: note("new", 200), c: note("theirs", 50) },
    };
    const r = mergeDocs(local, remote);
    expect(r.notes.a).toEqual(note("new", 200));
    expect(r.notes.b).toEqual(note("mine", 500));
    expect(r.notes.c).toEqual(note("theirs", 50));
    expect(r.adopted.notes.sort()).toEqual(["a", "c"]);
  });

  it("keeps local on ties (no ping-pong between equal clocks)", () => {
    const local = { ...empty, notes: { a: note("L", 100) } };
    const remote = { ...empty, notes: { a: note("R", 100) } };
    const r = mergeDocs(local, remote);
    expect(r.notes.a.content).toBe("L");
    expect(r.adopted.notes).toEqual([]);
  });

  it("flags pushNeeded when local has anything remote lacks or trails", () => {
    const both = { ...empty, notes: { a: note("same", 100) } };
    expect(mergeDocs(both, both).pushNeeded).toBe(false);
    expect(
      mergeDocs(
        { ...empty, notes: { a: note("x", 200) } },
        { ...empty, notes: { a: note("y", 100) } },
      ).pushNeeded,
    ).toBe(true);
    expect(
      mergeDocs({ ...empty, tasks: { t: { items: [], updatedAt: 1 } } }, empty)
        .pushNeeded,
    ).toBe(true);
    expect(
      mergeDocs(empty, { ...empty, notes: { a: note("z", 1) } }).pushNeeded,
    ).toBe(false);
  });

  it("merges the three kinds independently", () => {
    const local = { ...empty, tasks: { t1: { items: [], updatedAt: 300 } } };
    const remote = {
      ...empty,
      boards: { b1: { columns: [], cards: {}, updatedAt: 100 } },
      tasks: {
        t1: {
          items: [{ id: "i", text: "x", done: false, createdAt: 1 }],
          updatedAt: 200,
        },
      },
    };
    const r = mergeDocs(local, remote);
    expect(r.boards.b1).toBeDefined();
    expect(r.tasks.t1.updatedAt).toBe(300);
    expect(r.adopted.boards).toEqual(["b1"]);
    expect(r.adopted.tasks).toEqual([]);
  });
});
