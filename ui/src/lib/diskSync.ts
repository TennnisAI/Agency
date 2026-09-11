// An open editor tab following a file that something else is writing.
//
// A tab held whatever it read until it was closed and the file reopened: an
// agent editing the very file the user had open showed nothing (AGE-228). The
// window-focus re-read of AGE-162 does not fire while the user sits in the app
// watching the agent work, which is exactly when they want to see the edit.
// So each open tab polls the file's change signature (mtime and size, the same
// pair the docs poll keys on) and re-reads only when that moves; the re-read
// is then applied as the smallest edit that turns the held text into the disk
// text, so the lines above the change, the cursor and the scroll position all
// stay where they were.

import type { FileStat } from "../api";

/** The signature of what a tab holds, as returned by the read that filled it. */
export const statKey = (s: FileStat): string => `${s.mtimeMs}:${s.size}`;

/**
 * Whether the disk may hold something other than what was read under `seen`.
 *
 * Nothing seen yet (a save just landed and nothing has stat'ed the result)
 * reads as "maybe": the re-read that follows compares the text and is a no-op
 * when it matches. So does an mtime of 0, which is the backend for "the
 * platform won't say": with no clock to compare, a same-size rewrite would
 * otherwise never show.
 */
export function mayHaveChanged(seen: FileStat | null, now: FileStat): boolean {
  if (seen === null) return true;
  if (now.mtimeMs === 0 || seen.mtimeMs === 0) return true;
  return seen.mtimeMs !== now.mtimeMs || seen.size !== now.size;
}

/**
 * The single replacement that turns `from` into `to`: the span between the
 * longest common prefix and the longest common suffix. `null` when they are
 * the same text. Offsets are UTF-16 code units, which is what CodeMirror's
 * document positions are.
 *
 * One range, not a real diff: an agent's edit is usually one hunk, and even
 * when it is several, everything before the first and after the last stays
 * untouched, which is what keeps the reader's place.
 */
export function minimalChange(
  from: string,
  to: string,
): { from: number; to: number; insert: string } | null {
  if (from === to) return null;
  const max = Math.min(from.length, to.length);
  let head = 0;
  while (head < max && from.charCodeAt(head) === to.charCodeAt(head)) head++;
  // The suffix must not reach back into the prefix, or "ab" -> "aab" would
  // count the shared "a" twice and produce a negative span.
  let tail = 0;
  while (
    tail < max - head &&
    from.charCodeAt(from.length - 1 - tail) === to.charCodeAt(to.length - 1 - tail)
  ) tail++;
  // Never split a surrogate pair: a boundary that lands between the halves of
  // one astral character would hand CodeMirror an insert that starts or ends
  // on a lone surrogate.
  if (head > 0 && isHighSurrogate(from.charCodeAt(head - 1))) head--;
  if (tail > 0 && isLowSurrogate(from.charCodeAt(from.length - tail))) tail--;
  return { from: head, to: from.length - tail, insert: to.slice(head, to.length - tail) };
}

const isHighSurrogate = (c: number) => c >= 0xd800 && c <= 0xdbff;
const isLowSurrogate = (c: number) => c >= 0xdc00 && c <= 0xdfff;
