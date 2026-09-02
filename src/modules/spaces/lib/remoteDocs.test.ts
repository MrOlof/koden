import { describe, expect, it } from "vitest";
import {
  buildDocsManifest,
  docKeys,
  parseDocsManifest,
  planDocsApply,
  type RemoteDoc,
} from "./remoteDocs";

const note = (id: string, updatedAt: number, title = "Notes"): RemoteDoc => ({
  kind: "notes",
  id,
  title,
  payload: { content: `c-${id}`, updatedAt },
  updatedAt,
});

describe("parseDocsManifest", () => {
  it("round-trips through buildDocsManifest", () => {
    const docs = [note("n1", 100), { ...note("t1", 200), kind: "tasks" as const }];
    expect(parseDocsManifest(buildDocsManifest(docs))).toEqual(docs);
  });

  it("returns null (not empty) for absent or garbled input", () => {
    expect(parseDocsManifest("")).toBeNull();
    expect(parseDocsManifest("not json")).toBeNull();
    expect(parseDocsManifest("{}")).toBeNull();
  });

  it("drops malformed entries but keeps good ones", () => {
    const json = JSON.stringify({
      v: 1,
      docs: [note("ok", 1), { kind: "notes", id: "", title: "x", updatedAt: 1 }, { kind: "bogus", id: "b", title: "x", updatedAt: 1 }],
    });
    expect(parseDocsManifest(json)).toEqual([note("ok", 1)]);
  });
});

describe("planDocsApply", () => {
  it("creates missing tabs and applies newer payloads", () => {
    const remote = [note("n1", 200), note("n2", 50)];
    const plan = planDocsApply(
      remote,
      [{ kind: "notes", id: "n2" }],
      (_k, id) => (id === "n2" ? 100 : undefined),
      new Set(),
    );
    expect(plan.create.map((d) => d.id)).toEqual(["n1"]);
    // n1 unknown locally -> apply; n2 local (100) newer than remote (50) -> skip.
    expect(plan.apply.map((d) => d.id)).toEqual(["n1"]);
    expect(plan.close).toEqual([]);
  });

  it("closes only docs previously seen remotely — never fresh local work", () => {
    const plan = planDocsApply(
      [],
      [
        { kind: "notes", id: "was-remote" },
        { kind: "notes", id: "brand-new-local" },
      ],
      () => 1,
      new Set(["notes:was-remote"]),
    );
    expect(plan.close).toEqual([{ kind: "notes", id: "was-remote" }]);
  });

  it("docKeys feeds the seenBefore set", () => {
    expect(docKeys([note("a", 1)])).toEqual(new Set(["notes:a"]));
  });
});
