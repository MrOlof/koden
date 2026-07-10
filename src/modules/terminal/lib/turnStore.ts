// Session-lifetime storage for bus-delivered user turns (the UserPromptSubmit
// hook channel). Lives at module scope keyed by leafId, NOT inside a
// CommandMarks instance: CommandMarks is created/disposed with the renderer
// slot bind, so parking a hidden pane or a pool rebind used to wipe the whole
// Inputs history, and turns delivered while a leaf had no bound slot were
// silently dropped. Entries survive until the session itself is disposed.

export type BusTurn = { id: number; text: string };

// Bus turns live in their own high id band, above OSC-133 mark ids (a small
// counter) and scanned-line ids (SCAN_ID_BASE + line), so they never collide
// and, unanchored, sort after real buffer lines in arrival order.
export const TURN_LINE_BASE = 2_000_000_000;

const MAX_TURNS = 500;

const turnsByLeaf = new Map<number, BusTurn[]>();
let seq = 0;

export function addBusTurn(leafId: number, text: string): boolean {
  const t = text.trim().slice(0, 400);
  if (!t) return false;
  let arr = turnsByLeaf.get(leafId);
  if (!arr) {
    arr = [];
    turnsByLeaf.set(leafId, arr);
  }
  arr.push({ id: TURN_LINE_BASE + ++seq, text: t });
  while (arr.length > MAX_TURNS) arr.shift();
  return true;
}

export function busTurnsForLeaf(leafId: number): readonly BusTurn[] {
  return turnsByLeaf.get(leafId) ?? [];
}

export function clearBusTurns(leafId: number): void {
  turnsByLeaf.delete(leafId);
}
