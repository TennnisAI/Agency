// Turning one version of a text into another with the smallest possible edit.
//
// This exists for the editors that follow a file on disk. Replacing the whole
// document (`{ from: 0, to: doc.length, insert: next }`) is the obvious way to
// do it and the wrong one: CodeMirror maps every position inside a replaced
// range to the start of the replacement, so the cursor lands at position 0 and
// the scroll anchor goes with it. Trimming the common prefix and suffix leaves
// a change that touches only what actually differs, and everything outside it
// maps to itself.

export interface Replacement {
  from: number;
  to: number;
  insert: string;
}

/**
 * The single replacement that turns `before` into `after`, with the shared
 * head and tail trimmed off. Null when the two are already equal — there is
 * nothing to dispatch.
 */
export function minimalReplacement(before: string, after: string): Replacement | null {
  if (before === after) return null;
  const max = Math.min(before.length, after.length);
  let start = 0;
  while (start < max && before.charCodeAt(start) === after.charCodeAt(start)) start++;
  // Walk both tails back in step, never past the prefix we already matched.
  let endBefore = before.length;
  let endAfter = after.length;
  while (
    endBefore > start && endAfter > start &&
    before.charCodeAt(endBefore - 1) === after.charCodeAt(endAfter - 1)
  ) {
    endBefore--;
    endAfter--;
  }
  return { from: start, to: endBefore, insert: after.slice(start, endAfter) };
}
