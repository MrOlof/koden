import { beforeEach, describe, expect, it } from "vitest";
import { useDocsStore } from "./docsStore";

const get = () => useDocsStore.getState();

beforeEach(() => {
  useDocsStore.setState({ notes: {}, boards: {}, tasks: {}, hydrated: false });
});

describe("docsStore notes", () => {
  it("stores note content with a timestamp", () => {
    get().setNote("n1", "hello world");
    const doc = get().notes.n1;
    expect(doc.content).toBe("hello world");
    expect(typeof doc.updatedAt).toBe("number");
  });
});

describe("docsStore board", () => {
  it("ensureBoard seeds three columns once and is idempotent", () => {
    get().ensureBoard("b1");
    const first = get().boards.b1;
    expect(first.columns.map((c) => c.title)).toEqual([
      "To Do",
      "In Progress",
      "Done",
    ]);
    get().ensureBoard("b1");
    expect(get().boards.b1).toBe(first);
  });

  it("addCard appends a card to the target column", () => {
    get().ensureBoard("b1");
    const col = get().boards.b1.columns[0];
    get().addCard("b1", col.id, "  write tests  ");
    const board = get().boards.b1;
    const cardId = board.columns[0].cardIds[0];
    expect(board.columns[0].cardIds).toHaveLength(1);
    expect(board.cards[cardId].text).toBe("write tests");
  });

  it("ignores empty cards", () => {
    get().ensureBoard("b1");
    const col = get().boards.b1.columns[0];
    get().addCard("b1", col.id, "   ");
    expect(get().boards.b1.columns[0].cardIds).toHaveLength(0);
  });

  it("moveCard moves a card between columns and is a no-op within the same column", () => {
    get().ensureBoard("b1");
    const [todo, doing] = get().boards.b1.columns;
    get().addCard("b1", todo.id, "task");
    const cardId = get().boards.b1.columns[0].cardIds[0];

    get().moveCard("b1", cardId, doing.id);
    let board = get().boards.b1;
    expect(board.columns[0].cardIds).toHaveLength(0);
    expect(board.columns[1].cardIds).toEqual([cardId]);

    get().moveCard("b1", cardId, doing.id); // same column
    board = get().boards.b1;
    expect(board.columns[1].cardIds).toEqual([cardId]);
  });

  it("removeCard drops the card from its column and the card map", () => {
    get().ensureBoard("b1");
    const col = get().boards.b1.columns[0];
    get().addCard("b1", col.id, "task");
    const cardId = get().boards.b1.columns[0].cardIds[0];
    get().removeCard("b1", col.id, cardId);
    const board = get().boards.b1;
    expect(board.columns[0].cardIds).toHaveLength(0);
    expect(board.cards[cardId]).toBeUndefined();
  });

  it("editCard and renameColumn update text/title", () => {
    get().ensureBoard("b1");
    const col = get().boards.b1.columns[0];
    get().addCard("b1", col.id, "old");
    const cardId = get().boards.b1.columns[0].cardIds[0];
    get().editCard("b1", cardId, "new text");
    get().renameColumn("b1", col.id, "Backlog");
    const board = get().boards.b1;
    expect(board.cards[cardId].text).toBe("new text");
    expect(board.columns[0].title).toBe("Backlog");
  });

  it("mutations on an unknown board are no-ops", () => {
    get().addCard("ghost", "c", "x");
    expect(get().boards.ghost).toBeUndefined();
  });
});

describe("docsStore tasks", () => {
  it("ensureTaskList seeds an empty list once and is idempotent", () => {
    get().ensureTaskList("l1");
    const first = get().tasks.l1;
    expect(first.items).toEqual([]);
    get().ensureTaskList("l1");
    expect(get().tasks.l1).toBe(first);
  });

  it("addTask appends a trimmed, not-done task (auto-creating the list)", () => {
    get().addTask("l1", "  write the report  ");
    const list = get().tasks.l1;
    expect(list.items).toHaveLength(1);
    expect(list.items[0].text).toBe("write the report");
    expect(list.items[0].done).toBe(false);
    expect(typeof list.items[0].createdAt).toBe("number");
  });

  it("ignores empty tasks", () => {
    get().ensureTaskList("l1");
    get().addTask("l1", "   ");
    expect(get().tasks.l1.items).toHaveLength(0);
  });

  it("toggleTask flips done state", () => {
    get().addTask("l1", "task");
    const id = get().tasks.l1.items[0].id;
    get().toggleTask("l1", id);
    expect(get().tasks.l1.items[0].done).toBe(true);
    get().toggleTask("l1", id);
    expect(get().tasks.l1.items[0].done).toBe(false);
  });

  it("editTask updates text and ignores empty edits", () => {
    get().addTask("l1", "old");
    const id = get().tasks.l1.items[0].id;
    get().editTask("l1", id, "new text");
    expect(get().tasks.l1.items[0].text).toBe("new text");
    get().editTask("l1", id, "   ");
    expect(get().tasks.l1.items[0].text).toBe("new text");
  });

  it("removeTask drops the task", () => {
    get().addTask("l1", "a");
    get().addTask("l1", "b");
    const id = get().tasks.l1.items[0].id;
    get().removeTask("l1", id);
    expect(get().tasks.l1.items.map((t) => t.text)).toEqual(["b"]);
  });

  it("moveTask reorders and is a no-op at the bounds", () => {
    get().addTask("l1", "a");
    get().addTask("l1", "b");
    get().addTask("l1", "c");
    const [a] = get().tasks.l1.items;
    get().moveTask("l1", a.id, 1);
    expect(get().tasks.l1.items.map((t) => t.text)).toEqual(["b", "a", "c"]);
    get().moveTask("l1", a.id, -1);
    expect(get().tasks.l1.items.map((t) => t.text)).toEqual(["a", "b", "c"]);
    // Moving the first item up is a no-op (no neighbor).
    const ref = get().tasks.l1;
    get().moveTask("l1", get().tasks.l1.items[0].id, -1);
    expect(get().tasks.l1).toBe(ref);
  });

  it("clearCompleted removes only done tasks", () => {
    get().addTask("l1", "a");
    get().addTask("l1", "b");
    const id = get().tasks.l1.items[0].id;
    get().toggleTask("l1", id);
    get().clearCompleted("l1");
    expect(get().tasks.l1.items.map((t) => t.text)).toEqual(["b"]);
  });

  it("non-add mutations on an unknown list are no-ops", () => {
    get().toggleTask("ghost", "x");
    get().clearCompleted("ghost");
    expect(get().tasks.ghost).toBeUndefined();
  });
});
