/**
 * A [`FindEngine`] over a plain `<textarea>` — the issue description.
 *
 * A textarea can't paint decorations, so "the current match" is expressed the
 * only way the platform allows: by selecting it. That means revealing a match
 * has to go through focus (nothing else scrolls a text control to its
 * selection), which is why `reveal` borrows focus and hands it straight back to
 * whatever had it — usually the find bar's own input.
 *
 * The textarea is React-controlled, so a replace writes through `onChange` and
 * the selection is re-applied on the next frame, once the new value has
 * rendered.
 */

import {
  FindEngine, FindQuery, FindState, MATCH_CAP, Match, NO_FIND_STATE, findMatches, queryInvalid,
  replaceAllIn, replacementFor,
} from "./find";

export function textareaFindEngine(
  getEl: () => HTMLTextAreaElement | null,
  onChange: (next: string) => void,
): FindEngine {
  // The text we last wrote. A controlled textarea still holds the old value
  // for the rest of the tick, and counting against it would be wrong.
  let pending: string | null = null;

  const textOf = (el: HTMLTextAreaElement): string => {
    if (pending !== null && pending !== el.value) return pending;
    pending = null;
    return el.value;
  };

  const reveal = (el: HTMLTextAreaElement, m: Match) => {
    if (el.selectionStart === m.from && el.selectionEnd === m.to) return;
    const previous = document.activeElement as HTMLElement | null;
    // Focus first: setSelectionRange only scrolls the control it is focused on.
    el.focus();
    el.setSelectionRange(m.from, m.to);
    if (previous && previous !== el) previous.focus();
  };

  const report = (text: string, q: FindQuery, at: { from: number; to: number }): FindState => {
    if (queryInvalid(q)) return { ...NO_FIND_STATE, invalid: true };
    const matches = findMatches(text, q, MATCH_CAP + 1);
    const capped = matches.length > MATCH_CAP;
    const total = capped ? MATCH_CAP : matches.length;
    const idx = matches.findIndex((m) => m.from === at.from && m.to === at.to);
    return { total, current: idx >= 0 && idx < total ? idx + 1 : 0, capped, invalid: false };
  };

  /** Select the match `back`/forward from the current selection, wrapping. */
  const move = (q: FindQuery, from: "cursor" | "selection", back: boolean): FindState => {
    const el = getEl();
    if (!el) return NO_FIND_STATE;
    const text = textOf(el);
    if (queryInvalid(q)) return { ...NO_FIND_STATE, invalid: true };
    const matches = findMatches(text, q, Number.MAX_SAFE_INTEGER);
    if (matches.length === 0) return report(text, q, { from: -1, to: -1 });
    // "cursor" keeps a match already under the selection selected (typing into
    // the find field shouldn't skip past the match it just found); "selection"
    // always leaves it, which is what next/previous mean.
    const anchor = from === "cursor" ? el.selectionStart : back ? el.selectionStart : el.selectionEnd;
    const hit = back
      ? [...matches].reverse().find((m) => m.to <= anchor) ?? matches[matches.length - 1]
      : matches.find((m) => m.from >= anchor) ?? matches[0];
    reveal(el, hit);
    return report(text, q, hit);
  };

  return {
    sync: (q) => move(q, "cursor", false),
    recount: (q) => {
      const el = getEl();
      if (!el) return NO_FIND_STATE;
      return report(textOf(el), q, { from: el.selectionStart, to: el.selectionEnd });
    },
    step: (q, back) => move(q, "selection", back),
    replaceOne: (q) => {
      const el = getEl();
      if (!el) return NO_FIND_STATE;
      const text = textOf(el);
      if (queryInvalid(q)) return { ...NO_FIND_STATE, invalid: true };
      const at = { from: el.selectionStart, to: el.selectionEnd };
      const matches = findMatches(text, q, Number.MAX_SAFE_INTEGER);
      const idx = matches.findIndex((m) => m.from === at.from && m.to === at.to);
      // Nothing selected yet (the bar was just opened): the first press only
      // moves to a match, the second replaces it. Same as every other editor.
      if (idx < 0) return move(q, "cursor", false);
      const m = matches[idx];
      const insert = replacementFor(text, m, q);
      const next = text.slice(0, m.from) + insert + text.slice(m.to);
      pending = next;
      onChange(next);
      // The following match, shifted by what the replacement changed in length.
      const after = matches[idx + 1] ?? matches[0];
      const delta = insert.length - (m.to - m.from);
      const moved = after.from >= m.to
        ? { from: after.from + delta, to: after.to + delta }
        : after;
      const landing = matches.length > 1 ? moved : { from: m.from + insert.length, to: m.from + insert.length };
      requestAnimationFrame(() => {
        const live = getEl();
        if (live) reveal(live, landing);
      });
      return report(next, q, landing);
    },
    replaceAll: (q) => {
      const el = getEl();
      if (!el) return NO_FIND_STATE;
      const text = textOf(el);
      if (queryInvalid(q)) return { ...NO_FIND_STATE, invalid: true };
      const { text: next, count } = replaceAllIn(text, q);
      if (count === 0) return report(text, q, { from: -1, to: -1 });
      pending = next;
      onChange(next);
      return report(next, q, { from: -1, to: -1 });
    },
    selectedText: () => {
      const el = getEl();
      if (!el) return "";
      const text = el.value.slice(el.selectionStart, el.selectionEnd);
      return text.length > 0 && text.length <= 100 && !text.includes("\n") ? text : "";
    },
    refocus: () => getEl()?.focus(),
    dismiss: () => {
      pending = null;
    },
  };
}
