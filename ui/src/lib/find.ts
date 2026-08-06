/**
 * The vocabulary every find/replace surface in the app shares.
 *
 * Four very different things can host a find bar — the notes editor, the file
 * editor (both CodeMirror), the issue description (a plain textarea) and the
 * issue list (a filter, not a text scan) — so the query, the counts and the
 * commands live here and each surface supplies a [`FindEngine`] that knows how
 * to run them against its own content. That keeps one bar, one set of keys and
 * one set of semantics across all of them.
 *
 * Matching deliberately mirrors CodeMirror's own search: a non-regexp query
 * still resolves `\n`, `\r` and `\t`, and "whole word" is decided by the
 * characters *around* a match rather than by `\b` (which misbehaves as soon as
 * the query itself starts or ends with punctuation).
 */

/** Whether the bar opens on the find field alone or with replace showing. */
export type FindMode = "find" | "replace";

export interface FindQuery {
  search: string;
  replace: string;
  caseSensitive: boolean;
  /** Interpret `search` as a regular expression (and `$1`/`$&` in `replace`). */
  regexp: boolean;
  wholeWord: boolean;
}

export const EMPTY_FIND_QUERY: FindQuery = {
  search: "",
  replace: "",
  caseSensitive: false,
  regexp: false,
  wholeWord: false,
};

/** What the bar reports back after any command. */
export interface FindState {
  /** Matches in the document, capped at [`MATCH_CAP`]. */
  total: number;
  /** 1-based position of the match the cursor is on; 0 when none is. */
  current: number;
  /** Counting stopped at the cap, so `total` is a floor ("500+"). */
  capped: boolean;
  /** A regexp query that doesn't compile — the field is flagged, nothing runs. */
  invalid: boolean;
}

export const NO_FIND_STATE: FindState = { total: 0, current: 0, capped: false, invalid: false };

/**
 * Counting every match in a multi-megabyte file on each keystroke is the one
 * way a find bar can make an editor feel slow, so counting stops here and the
 * bar says so ("500+"). Navigation is unaffected: next/previous scan from the
 * cursor, never from a full match list.
 */
export const MATCH_CAP = 500;

export interface Match {
  from: number;
  to: number;
}

/**
 * One surface's ability to run a query. Every method takes the query rather
 * than storing it, so the bar stays the single owner of what is being searched
 * for and an engine can be rebuilt (a note switch, a re-render) without losing
 * it.
 */
export interface FindEngine {
  /**
   * Push the query at the surface and land on the first match at or after the
   * cursor — find-as-you-type.
   */
  sync(q: FindQuery): FindState;
  /** Re-read the counts after the content changed underneath, without moving. */
  recount(q: FindQuery): FindState;
  /** Move to the next (or previous) match, wrapping at the ends. */
  step(q: FindQuery, back: boolean): FindState;
  /** Replace the match the cursor is on, then move to the next. */
  replaceOne(q: FindQuery): FindState;
  replaceAll(q: FindQuery): FindState;
  /** Seed the find field from the selection when the bar opens. */
  selectedText(): string;
  /** Hand focus back to the content (Escape, or after a Replace All). */
  refocus(): void;
  /** Drop highlights when the bar closes. */
  dismiss(): void;
}

const WORD = /[\p{L}\p{N}_$]/u;

function isWordChar(ch: string | undefined): boolean {
  return !!ch && WORD.test(ch);
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Resolve the escapes a literal query is allowed to carry. CodeMirror does this
 * for string searches, and the textarea engine has to agree with it or the same
 * query would mean two things in two panes.
 */
function unescapeLiteral(s: string): string {
  return s.replace(/\\([nrt\\])/g, (_, c: string) =>
    c === "n" ? "\n" : c === "r" ? "\r" : c === "t" ? "\t" : "\\");
}

/** The text a query actually looks for, escapes resolved. */
export function searchText(q: FindQuery): string {
  return q.regexp ? q.search : unescapeLiteral(q.search);
}

/**
 * The query as a sticky-free global RegExp, or null when it can't match: an
 * empty search, or a regexp that doesn't compile. Callers treat null as "do
 * nothing" and use [`queryInvalid`] to tell the two apart.
 */
export function compileFind(q: FindQuery): RegExp | null {
  if (!q.search) return null;
  const source = q.regexp ? q.search : escapeRegExp(unescapeLiteral(q.search));
  try {
    return new RegExp(source, q.caseSensitive ? "gu" : "giu");
  } catch {
    // `u` rejects patterns older engines tolerate (a bare `\d` inside a class,
    // stray `{`). Fall back to a non-unicode regexp before calling it invalid,
    // so a working pattern isn't rejected over a flag the user never asked for.
    try {
      return new RegExp(source, q.caseSensitive ? "g" : "gi");
    } catch {
      return null;
    }
  }
}

/** A non-empty regexp query that doesn't compile — the field is flagged. */
export function queryInvalid(q: FindQuery): boolean {
  return !!q.search && q.regexp && compileFind(q) === null;
}

/** True when a match at [from, to) has no word characters butting against it. */
export function wholeWordAt(text: string, from: number, to: number): boolean {
  return !isWordChar(text[from - 1]) && !isWordChar(text[to]);
}

/**
 * Every non-overlapping match of `q` in `text`, up to `cap`. A zero-length
 * match (`a*`, `^`) advances the scan by one so an empty pattern can't spin
 * forever.
 */
export function findMatches(text: string, q: FindQuery, cap = MATCH_CAP): Match[] {
  const re = compileFind(q);
  if (!re) return [];
  const out: Match[] = [];
  re.lastIndex = 0;
  for (let m = re.exec(text); m !== null && out.length < cap; m = re.exec(text)) {
    const from = m.index;
    const to = from + m[0].length;
    if (!q.wholeWord || wholeWordAt(text, from, to)) out.push({ from, to });
    re.lastIndex = to > from ? to : from + 1;
    if (re.lastIndex > text.length) break;
  }
  return out;
}

/**
 * The text a match is replaced with: a regexp query expands `$1`/`$&`/`$$` the
 * way `String.replace` does, a literal query inserts its replacement verbatim
 * (so a `$` in a plain string search survives).
 */
export function replacementFor(text: string, m: Match, q: FindQuery): string {
  if (!q.regexp) return q.replace;
  const re = compileFind(q);
  if (!re) return q.replace;
  return text.slice(m.from, m.to).replace(new RegExp(re.source, re.flags.replace("g", "")), q.replace);
}

/** `text` with every match replaced, and how many were. */
export function replaceAllIn(text: string, q: FindQuery): { text: string; count: number } {
  // No cap here: "replace all" that quietly stopped at 500 would be a data bug.
  const matches = findMatches(text, q, Number.MAX_SAFE_INTEGER);
  if (matches.length === 0) return { text, count: 0 };
  let out = "";
  let at = 0;
  for (const m of matches) {
    out += text.slice(at, m.from) + replacementFor(text, m, q);
    at = m.to;
  }
  return { text: out + text.slice(at), count: matches.length };
}
