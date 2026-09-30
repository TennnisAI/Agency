/**
 * Find over rendered HTML: the markdown preview in the Files tab.
 *
 * The preview has no text buffer, so ⌘F used to be switched off there and fell
 * through to nothing at all; the only way to search a rendered README was to
 * flip back to the code. This engine searches the text the reader actually sees
 * (the DOM's text nodes, not the markdown source) and marks matches by wrapping
 * them in spans. Read-only: there is nothing to replace into.
 *
 * Matching is `findMatches` over the concatenated text nodes, so a query means
 * the same thing here as in the editor, and a match that crosses an element
 * boundary (a phrase half in bold) is still found and drawn as one piece per
 * text node it covers.
 */

import { FindEngine, FindQuery, FindState, MATCH_CAP, Match, NO_FIND_STATE, findMatches, queryInvalid } from "./find";

/** One slice of one match inside one text node. */
export interface Segment {
  /** Index into the text nodes, in document order. */
  node: number;
  from: number;
  to: number;
  /** Index of the match this slice belongs to. */
  match: number;
}

/**
 * Cut each match into the per-node slices that draw it, given the length of
 * every text node in document order. Empty slices are skipped: a match that
 * ends exactly at a node boundary has nothing to draw in the next node.
 */
export function matchSegments(nodeLengths: number[], matches: Match[]): Segment[] {
  const out: Segment[] = [];
  let node = 0;
  let start = 0; // offset of `node` in the concatenated text
  matches.forEach((m, match) => {
    while (node < nodeLengths.length && start + nodeLengths[node] <= m.from) {
      start += nodeLengths[node];
      node++;
    }
    let n = node;
    let s = start;
    while (n < nodeLengths.length && s < m.to) {
      const from = Math.max(m.from, s) - s;
      const to = Math.min(m.to, s + nodeLengths[n]) - s;
      if (to > from) out.push({ node: n, from, to, match });
      s += nodeLengths[n];
      n++;
    }
  });
  return out;
}

/** The first match at or after `anchor`, wrapping to the first; -1 for none. */
export function matchAtOrAfter(matches: Match[], anchor: number): number {
  if (matches.length === 0) return -1;
  const i = matches.findIndex((m) => m.from >= anchor);
  return i < 0 ? 0 : i;
}

const MATCH_CLASS = "dom-find-match";
const CURRENT_CLASS = "dom-find-current";

function textNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const out: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n as Text);
  return out;
}

/** Put the text back the way the renderer left it. */
function unmark(root: HTMLElement) {
  const marks = root.querySelectorAll(`span.${MATCH_CLASS}`);
  if (marks.length === 0) return;
  const parents = new Set<Node>();
  marks.forEach((m) => {
    const parent = m.parentNode;
    if (!parent) return;
    while (m.firstChild) parent.insertBefore(m.firstChild, m);
    parent.removeChild(m);
    parents.add(parent);
  });
  // Rejoin the split text nodes, so the next search sees the text nodes the
  // renderer made rather than the fragments the last one left.
  parents.forEach((p) => p.normalize());
}

/**
 * A find bar's handle on one rendered document. `getRoot` is a thunk because
 * the preview element comes and goes with the Code / Preview toggle.
 */
export function domFindEngine(getRoot: () => HTMLElement | null): FindEngine {
  // Where the next find-as-you-type search starts: the match the bar is on, so
  // typing one more letter stays put instead of jumping to the top.
  let anchor = 0;
  let current = -1;

  const paint = (root: HTMLElement, q: FindQuery, pick: (matches: Match[]) => number): FindState => {
    unmark(root);
    const invalid = queryInvalid(q);
    if (invalid || !q.search) {
      current = -1;
      return { ...NO_FIND_STATE, invalid };
    }
    const nodes = textNodes(root);
    const text = nodes.map((n) => n.data).join("");
    const matches = findMatches(text, q, MATCH_CAP);
    current = pick(matches);
    if (current >= 0) anchor = matches[current].from;
    const segments = matchSegments(nodes.map((n) => n.data.length), matches);
    // Last to first: splitting a node keeps its head in place, so the offsets
    // of every earlier slice in the same node stay good.
    let first: HTMLElement | null = null;
    for (let i = segments.length - 1; i >= 0; i--) {
      const seg = segments[i];
      const mid = nodes[seg.node].splitText(seg.from);
      mid.splitText(seg.to - seg.from);
      const span = document.createElement("span");
      span.className = seg.match === current ? `${MATCH_CLASS} ${CURRENT_CLASS}` : MATCH_CLASS;
      mid.parentNode?.insertBefore(span, mid);
      span.appendChild(mid);
      if (seg.match === current) first = span;
    }
    first?.scrollIntoView({ block: "nearest" });
    return {
      total: matches.length,
      current: current + 1,
      capped: matches.length >= MATCH_CAP,
      invalid: false,
    };
  };

  const run = (q: FindQuery, pick: (matches: Match[]) => number): FindState => {
    const root = getRoot();
    return root ? paint(root, q, pick) : NO_FIND_STATE;
  };

  const readOnly = (q: FindQuery) => run(q, (ms) => matchAtOrAfter(ms, anchor));

  return {
    sync: readOnly,
    recount: readOnly,
    step: (q, back) =>
      run(q, (ms) => {
        if (ms.length === 0) return -1;
        if (current < 0) return matchAtOrAfter(ms, anchor);
        return (current + (back ? ms.length - 1 : 1)) % ms.length;
      }),
    replaceOne: readOnly,
    replaceAll: readOnly,
    selectedText: () => {
      const root = getRoot();
      const sel = window.getSelection();
      if (!root || !sel || sel.isCollapsed || !root.contains(sel.anchorNode)) return "";
      const text = sel.toString();
      return text.length > 100 || text.includes("\n") ? "" : text;
    },
    refocus: () => {},
    dismiss: () => {
      const root = getRoot();
      if (root) unmark(root);
      current = -1;
    },
  };
}
