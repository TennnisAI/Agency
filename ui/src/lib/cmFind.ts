/**
 * The CodeMirror half of find/replace: the extensions an editor installs, and
 * a [`FindEngine`] that drives them from the app's own find bar.
 *
 * CodeMirror ships a search panel, and we deliberately never open it — the
 * notes editor and the file editor would then look nothing like the issue
 * description or the terminal, which have no panel to open. Instead we keep
 * CodeMirror's *matcher* (`SearchQuery`, `findNext`, `replaceAll` — the parts
 * that get regexp, wrapping and change-mapping right) and supply our own
 * highlighting, because the built-in highlighter draws nothing while its panel
 * is closed.
 */

import { EditorState, Extension, RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import { Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate } from "@codemirror/view";
import {
  SearchQuery, findNext, findPrevious, getSearchQuery, replaceAll, replaceNext, search, setSearchQuery,
} from "@codemirror/search";
import { FindEngine, FindQuery, FindState, MATCH_CAP, NO_FIND_STATE, queryInvalid } from "./find";

/** Turns the app's highlighting on while a find bar is open on this editor. */
const setFindActive = StateEffect.define<boolean>();

const findActive = StateField.define<boolean>({
  create: () => false,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setFindActive)) return e.value;
    return value;
  },
});

const matchMark = Decoration.mark({ class: "cm-find-match" });
const currentMark = Decoration.mark({ class: "cm-find-match cm-find-current" });

function buildHighlights(view: EditorView): DecorationSet {
  if (!view.state.field(findActive)) return Decoration.none;
  const query = getSearchQuery(view.state);
  if (!query.valid) return Decoration.none;
  const sel = view.state.selection.main;
  const builder = new RangeSetBuilder<Decoration>();
  // Visible ranges only: a find in a 20k-line file shouldn't decorate lines
  // nobody is looking at.
  for (const { from, to } of view.visibleRanges) {
    const cursor = query.getCursor(view.state, from, to);
    for (let it = cursor.next(); !it.done; it = cursor.next()) {
      const m = it.value;
      builder.add(m.from, m.to, m.from === sel.from && m.to === sel.to ? currentMark : matchMark);
    }
  }
  return builder.finish();
}

const findHighlighter = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildHighlights(view);
    }
    update(u: ViewUpdate) {
      const toggled = u.startState.field(findActive) !== u.state.field(findActive);
      const requeried = getSearchQuery(u.startState) !== getSearchQuery(u.state);
      if (u.docChanged || u.selectionSet || u.viewportChanged || toggled || requeried) {
        this.decorations = buildHighlights(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

const findTheme = EditorView.theme({
  ".cm-find-match": {
    backgroundColor: "var(--find-match)",
    borderRadius: "2px",
  },
  ".cm-find-match.cm-find-current": {
    backgroundColor: "var(--find-current)",
    outline: "1px solid var(--find-current-edge)",
  },
});

/**
 * Everything an editor needs to be searchable. `search()` is here for its state
 * field — the panel it can build is never asked for.
 */
export const cmFindExtensions: Extension = [search(), findActive, findHighlighter, findTheme];

function toSearchQuery(q: FindQuery): SearchQuery {
  return new SearchQuery({
    search: q.search,
    replace: q.replace,
    caseSensitive: q.caseSensitive,
    regexp: q.regexp,
    wholeWord: q.wholeWord,
  });
}

/** Push the query at the editor without touching the selection. */
function pushQuery(view: EditorView, q: FindQuery): SearchQuery {
  const query = toSearchQuery(q);
  view.dispatch({ effects: [setSearchQuery.of(query), setFindActive.of(true)] });
  return query;
}

function count(state: EditorState, query: SearchQuery, invalid: boolean): FindState {
  if (invalid || !query.valid) return { ...NO_FIND_STATE, invalid };
  const sel = state.selection.main;
  const cursor = query.getCursor(state);
  let total = 0;
  let current = 0;
  for (let it = cursor.next(); !it.done; it = cursor.next()) {
    if (total >= MATCH_CAP) return { total, current, capped: true, invalid: false };
    total++;
    if (it.value.from === sel.from && it.value.to === sel.to) current = total;
  }
  return { total, current, capped: false, invalid: false };
}

/**
 * A find bar's handle on one CodeMirror view. `getView` is a thunk because the
 * editors rebuild their view whenever the open file changes — the engine
 * outlives any single one.
 */
export function cmFindEngine(getView: () => EditorView | null): FindEngine {
  const run = (fn: (view: EditorView, query: SearchQuery) => void, q: FindQuery): FindState => {
    const view = getView();
    if (!view) return NO_FIND_STATE;
    const invalid = queryInvalid(q);
    const query = pushQuery(view, q);
    if (!invalid && query.valid) fn(view, query);
    // Re-read the query: a replace remaps the document under it.
    return count(view.state, getSearchQuery(view.state), invalid);
  };

  return {
    sync: (q) =>
      run((view, query) => {
        // Find-as-you-type: land on the first match at or after the cursor so
        // the count's "1 of 9" refers to something the user can see. Searching
        // from `from` (not `to`) keeps an already-selected match selected
        // instead of skipping past it on every keystroke.
        const sel = view.state.selection.main;
        const first = query.getCursor(view.state, sel.from).next();
        const m = first.done ? query.getCursor(view.state, 0).next() : first;
        if (m.done) return;
        view.dispatch({
          selection: { anchor: m.value.from, head: m.value.to },
          effects: EditorView.scrollIntoView(m.value.from, { y: "nearest", yMargin: 40 }),
        });
      }, q),
    recount: (q) => run(() => {}, q),
    step: (q, back) => run((view) => { (back ? findPrevious : findNext)(view); }, q),
    replaceOne: (q) => run((view) => { replaceNext(view); }, q),
    replaceAll: (q) => run((view) => { replaceAll(view); }, q),
    selectedText: () => {
      const view = getView();
      if (!view) return "";
      const sel = view.state.selection.main;
      // A sprawling multi-line selection is a region someone wants to search
      // *within*, not a query — seeding from it would be nonsense.
      if (sel.empty || sel.to - sel.from > 100) return "";
      const text = view.state.sliceDoc(sel.from, sel.to);
      return text.includes("\n") ? "" : text;
    },
    refocus: () => getView()?.focus(),
    dismiss: () => {
      getView()?.dispatch({ effects: setFindActive.of(false) });
    },
  };
}
