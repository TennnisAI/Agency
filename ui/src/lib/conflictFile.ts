// Conflict markers in a file git left unmerged, and what taking a side does to
// the text.
//
// This exists because there was no way to resolve a conflict in Agency by hand
// (AGE-199). The diff pane offered the ordinary stage/revert-by-line buttons on
// a conflicted file, but `git diff` answers for one of those with a *combined*
// diff — `diff --cc`, `@@@` hunk headers, two columns of prefixes — whose lines
// are not the file's lines. Selecting the side you wanted and staging it fed
// that back through `git apply`, which is how `<<<<<<< HEAD` and the branch
// name ended up written into the file itself.
//
// So the file, not the diff, is what the conflict view reads, and every
// resolution here is a pure rewrite of it: parse into blocks, drop the markers
// and the side you did not pick, put the rest back untouched. The caller writes
// the result and stages it.

/** Which side of a conflict to keep. `both` keeps them in the order git wrote. */
export type Side = "current" | "incoming" | "both";

export interface ConflictBlock {
  /** Line index of the `<<<<<<<` marker. */
  start: number;
  /** Line index of the `>>>>>>>` marker. */
  end: number;
  /** What git wrote after `<<<<<<<`: the branch being merged into (usually `HEAD`). */
  currentLabel: string;
  /** What git wrote after `>>>>>>>`: the branch being merged in. */
  incomingLabel: string;
  current: string[];
  incoming: string[];
}

const OURS = "<<<<<<<";
const BASE = "|||||||";
const SPLIT = "=======";
const THEIRS = ">>>>>>>";

// A marker is the seven characters at the start of a line, followed by end of
// line or a space. Anything else — a row of equals signs under a heading, a
// shell heredoc — is ordinary text, and treating it as a marker would carve up
// a file that has no conflict in it at all.
function marker(line: string, m: string): boolean {
  return line.startsWith(m) && (line.length === m.length || line[m.length] === " ");
}

function label(line: string, m: string): string {
  return line.slice(m.length).trim();
}

/**
 * The conflicts in `text`, in the order they appear.
 *
 * Conservative by design: only well-formed blocks are returned, and anything
 * unparseable (a `<<<<<<<` with no `=======` after it, a second `<<<<<<<`
 * inside a block) yields no block at all, so the buttons this feeds are simply
 * not offered rather than offered for a rewrite that would lose text. diff3
 * conflicts carry a `|||||||` base section, which belongs to neither side and
 * is dropped with the markers.
 */
export function parseConflicts(text: string): ConflictBlock[] {
  const lines = text.split("\n");
  const out: ConflictBlock[] = [];
  let i = 0;
  while (i < lines.length) {
    if (!marker(lines[i], OURS)) {
      i++;
      continue;
    }
    const start = i;
    const current: string[] = [];
    const incoming: string[] = [];
    // "ours" until the base or the split, "theirs" after the split.
    let phase: "ours" | "base" | "theirs" = "ours";
    let end = -1;
    for (let j = start + 1; j < lines.length; j++) {
      const line = lines[j];
      if (marker(line, OURS)) break; // nested: malformed, leave the file alone
      if (phase !== "theirs" && marker(line, BASE)) {
        phase = "base";
        continue;
      }
      if (phase !== "theirs" && marker(line, SPLIT)) {
        phase = "theirs";
        continue;
      }
      if (marker(line, THEIRS)) {
        // A `>>>>>>>` before the `=======` is not the end of a conflict.
        if (phase === "theirs") end = j;
        break;
      }
      if (phase === "ours") current.push(line);
      else if (phase === "theirs") incoming.push(line);
    }
    // A marker we cannot pair up means the file is not shaped the way this
    // module thinks. Refusing all of it, rather than the blocks around it, is
    // what keeps a rewrite from leaving the odd `<<<<<<<` behind — which is the
    // failure this whole view exists to stop.
    if (end === -1) return [];
    out.push({
      start,
      end,
      currentLabel: label(lines[start], OURS),
      incomingLabel: label(lines[end], THEIRS),
      current,
      incoming,
    });
    i = end + 1;
  }
  return out;
}

/** Whether `text` still holds a conflict this module can resolve. */
export function hasConflicts(text: string): boolean {
  return parseConflicts(text).length > 0;
}

function chosen(block: ConflictBlock, side: Side): string[] {
  if (side === "current") return block.current;
  if (side === "incoming") return block.incoming;
  return [...block.current, ...block.incoming];
}

/**
 * `text` with the `index`th conflict replaced by the side chosen. Every other
 * line, conflict markers of later blocks included, comes through unchanged —
 * which is what makes resolving one block at a time safe.
 *
 * An index that is not a block is returned unchanged rather than throwing: the
 * view re-reads the file after every write, and a stale click is a no-op, not
 * a rewrite of the wrong region.
 */
export function resolveBlock(text: string, index: number, side: Side): string {
  const blocks = parseConflicts(text);
  const block = blocks[index];
  if (!block) return text;
  const lines = text.split("\n");
  return [...lines.slice(0, block.start), ...chosen(block, side), ...lines.slice(block.end + 1)]
    .join("\n");
}

/**
 * `text` with every conflict resolved the same way. Applied back to front so
 * each block's recorded line numbers still describe the text being cut.
 */
export function resolveAll(text: string, side: Side): string {
  const blocks = parseConflicts(text);
  let lines = text.split("\n");
  for (const block of [...blocks].reverse()) {
    lines = [...lines.slice(0, block.start), ...chosen(block, side), ...lines.slice(block.end + 1)];
  }
  return lines.join("\n");
}
