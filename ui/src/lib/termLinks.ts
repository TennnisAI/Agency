// Clickable links in terminal output (AGE-94). Agents print URLs and file
// paths constantly; a bare terminal makes those clickable, so a pane in Agency
// should too. This module is the pure half: finding the candidates on a line
// and mapping them back onto buffer cells. Whether a path-shaped word is really
// a file is decided on disk (`resolve_term_paths`), and the wiring — the xterm
// link provider, the ⌘ gate, what a click opens — lives in FocusTerminal.

export interface TermLinkCandidate {
  kind: "url" | "path";
  /** Offsets into the logical (unwrapped) line: [start, end). */
  start: number;
  end: number;
  /** What a click acts on: a normalized URL, or the path minus its ":12" suffix. */
  target: string;
  /** 1-based line to land on, from a trailing ":12" or ":12:3". */
  line?: number;
}

// Punctuation and TUI chrome that commonly hugs a path or URL: brackets and
// quotes on both sides, sentence punctuation and box-drawing rules on the
// right. Trimmed off the token before anything is judged.
const OPENERS = `([{<"'\`«“‘|│┃*_`;
const CLOSERS = `)]}>"'\`»”’|│┃,.;:!?*_`;
const PAIRS: Record<string, string> = { ")": "(", "]": "[", "}": "{", ">": "<" };

// Schemes worth handing to the browser. Deliberately short: terminal output is
// program output, and "clickable" shouldn't mean any scheme the OS registered.
const URL_SCHEME = /^(https?:\/\/|file:\/\/|mailto:)/i;
// Bare hostnames as people write them in prose.
const BARE_WWW = /^www\.[^\s/]+\.[^\s/]/i;
// The "src/main.rs:42" and "src/main.rs:42:9" forms every compiler prints.
const LINE_SUFFIX = /:(\d+)(?::\d+)?$/;
// A leading /, ./, ../ or ~/ — unambiguously a path, extension or not.
const ROOTED = /^(\/|\.{1,2}\/|~\/)/;
// A bare filename ("package.json", "Cargo.toml"): a real extension is the only
// thing separating one from an ordinary word, and the on-disk check is what
// finally settles it ("e.g" gets this far and dies there). No `@`, which at
// this point means an email address rather than a scoped package directory —
// those carry a slash and never reach here.
const FILENAME = /^\w[\w.\-+]*\.[A-Za-z][A-Za-z0-9]{0,7}$/;

// A token longer than this is not a path anyone printed on purpose (a base64
// blob, a minified line), and running the checks over it is wasted work.
const MAX_TOKEN = 512;

function occurrences(text: string, ch: string): number {
  let n = 0;
  for (const c of text) if (c === ch) n++;
  return n;
}

/**
 * Strip wrapping punctuation, returning the inner span. A closing bracket is
 * kept when the token opens one of its own, so the tail of a URL like
 * `…/Foo_(bar)` survives while `(src/main.rs)` loses its parentheses.
 */
function trimEdges(raw: string): { start: number; end: number } {
  let start = 0;
  let end = raw.length;
  while (start < end && OPENERS.includes(raw[start])) start++;
  while (end > start && CLOSERS.includes(raw[end - 1])) {
    const opener = PAIRS[raw[end - 1]];
    if (opener) {
      const inner = raw.slice(start, end);
      if (occurrences(inner, opener) >= occurrences(inner, raw[end - 1])) break;
    }
    end--;
  }
  return { start, end };
}

function looksLikePath(text: string): boolean {
  if (!text || text.includes("://")) return false;
  if (ROOTED.test(text)) return true;
  if (text.includes("/")) return true;
  return FILENAME.test(text);
}

/**
 * The URLs and path-shaped words on one logical line, in order and never
 * overlapping. Paths are candidates only: they carry no promise that anything
 * is there, which is why the caller resolves them before offering a link.
 */
export function scanLine(line: string): TermLinkCandidate[] {
  const found: TermLinkCandidate[] = [];
  for (const match of line.matchAll(/\S+/g)) {
    const raw = match[0];
    const at = match.index ?? 0;
    if (raw.length > MAX_TOKEN) continue;
    const { start, end } = trimEdges(raw);
    const token = raw.slice(start, end);
    if (!token) continue;

    if (URL_SCHEME.test(token)) {
      // A scheme with nothing after it ("https://") is not a link.
      if (!token.replace(URL_SCHEME, "")) continue;
      found.push({ kind: "url", start: at + start, end: at + end, target: token });
      continue;
    }
    if (BARE_WWW.test(token)) {
      found.push({ kind: "url", start: at + start, end: at + end, target: `https://${token}` });
      continue;
    }

    const suffix = LINE_SUFFIX.exec(token);
    const path = suffix ? token.slice(0, suffix.index) : token;
    if (!looksLikePath(path)) continue;
    found.push({
      kind: "path",
      start: at + start,
      end: at + end,
      target: path,
      ...(suffix ? { line: Number(suffix[1]) } : {}),
    });
  }
  return found;
}

// The slice of xterm's IBuffer this module needs, so the line walk can be
// tested against a plain object as well as a live terminal.
export interface BufferLike {
  length: number;
  getLine(index: number): { isWrapped: boolean; translateToString(trimRight?: boolean): string } | undefined;
}

export interface LogicalLine {
  /** The whole wrapped line, joined at exactly `cols` characters per row. */
  text: string;
  /** 0-based buffer index of the row the line starts on. */
  firstRow: number;
}

// A "line" that wraps further than this is a dump, not a sentence with a path
// in it; joining thousands of rows on every hover would cost more than the
// links are worth.
const MAX_WRAPPED_ROWS = 32;

/**
 * The full logical line through buffer row `y` (xterm's 1-based line number),
 * unwrapped, so a path split across the pane's right edge is still one string.
 * Rows other than the last keep their padding, which is what lets an offset be
 * divided straight back into a row and a column.
 */
export function logicalLine(buffer: BufferLike, y: number): LogicalLine | null {
  let first = y - 1;
  if (first < 0 || first >= buffer.length) return null;
  while (first > 0 && buffer.getLine(first)?.isWrapped) first--;
  let last = y - 1;
  while (last + 1 < buffer.length && buffer.getLine(last + 1)?.isWrapped) last++;
  if (last - first >= MAX_WRAPPED_ROWS) return null;

  let text = "";
  for (let i = first; i <= last; i++) {
    const line = buffer.getLine(i);
    if (!line) return null;
    text += line.translateToString(i === last);
  }
  return { text, firstRow: first };
}

export interface BufferRange {
  start: { x: number; y: number };
  end: { x: number; y: number };
}

/**
 * Turn a `[start, end)` offset span in a logical line back into the buffer
 * range xterm decorates — 1-based, with `end` on the link's last cell.
 */
export function linkRange(
  cols: number,
  firstRow: number,
  start: number,
  end: number,
): BufferRange {
  const cell = (offset: number) => ({
    x: (offset % cols) + 1,
    y: firstRow + Math.floor(offset / cols) + 1,
  });
  return { start: cell(start), end: cell(end - 1) };
}
