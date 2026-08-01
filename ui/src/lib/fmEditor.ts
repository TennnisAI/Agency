// The frontmatter properties editor (Obsidian-style, in the note's flow).
// A StateField replaces the raw `---` block with a block widget: one row per
// property with key/value inputs, hover tools (filter the vault by the pair,
// delete it), key/value autocomplete drawn from the whole corpus, and an add
// row. Edits are collected in the DOM and committed as one document change
// when focus leaves the card (or on Enter), so Tab flows between inputs
// without rebuilds, and undo reverts a whole edit at once. The document
// stays the source of truth: the widget rebuilds only when the block's text
// actually changes.

import { EditorState, Extension, StateField } from "@codemirror/state";
import { Decoration, DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { isValidFmKey, parseFrontmatter, serializeFrontmatter, unquote } from "./docsIndex";
import { docsIndexFacet, docsNavFacet } from "./livePreview";

interface FmBlock {
  /** Raw text of the block, fences included, no trailing newline. */
  text: string;
  pairs: [string, string][];
  /** Each value's raw scalar as written (quotes intact), parallel to pairs. */
  raws: string[];
  /** Doc position just past the closing fence line (its line end). */
  to: number;
}

function fmBlock(state: EditorState): FmBlock | null {
  const doc = state.doc;
  if (doc.lines < 2 || doc.line(1).text.trim() !== "---") return null;
  const head: string[] = [];
  const max = Math.min(doc.lines, 100);
  for (let n = 1; n <= max; n++) head.push(doc.line(n).text);
  const fm = parseFrontmatter(head);
  if (!fm) return null;
  const to = doc.line(fm.end).to;
  return { text: doc.sliceString(0, to), pairs: fm.pairs, raws: fm.raws, to };
}

// Set by requestAddProperty before a transaction creates/rebuilds the widget;
// consumed in toDOM to focus a fresh row.
let pendingAdd = false;

/**
 * The side panel's "+ add" entry point. With no frontmatter, inserts an empty
 * block (self-cleaning: committing zero rows deletes it again); with one,
 * asks the live card to grow a row.
 */
export function requestAddProperty(view: EditorView) {
  const existing = view.dom.querySelector(".fmw");
  if (existing) {
    (existing as HTMLElement & { _fmwAddRow?: () => void })._fmwAddRow?.();
    return;
  }
  pendingAdd = true;
  view.dispatch({ changes: { from: 0, to: 0, insert: "---\n---\n" } });
}

class FmWidget extends WidgetType {
  constructor(readonly block: FmBlock) {
    super();
  }
  eq(other: FmWidget) {
    return other.block.text === this.block.text;
  }
  ignoreEvent() {
    return true; // the card owns its events; CM must not interpret them
  }
  get estimatedHeight() {
    return 42 + this.block.pairs.length * 28;
  }
  toDOM(view: EditorView) {
    return buildCard(view, this.block);
  }
}

function ghostButton(cls: string, glyph: string, title: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls;
  b.textContent = glyph;
  b.title = title;
  b.tabIndex = -1; // Tab moves between inputs; tools are mouse targets
  return b;
}

function buildCard(view: EditorView, block: FmBlock): HTMLElement {
  const card = document.createElement("div");
  card.className = "fmw";
  // CM sets contenteditable on content; inputs need it off to behave.
  card.setAttribute("contenteditable", "false");

  const head = document.createElement("div");
  head.className = "fmw-head";
  const headLabel = document.createElement("span");
  headLabel.textContent = "properties";
  const headAdd = ghostButton("fmw-head-add", "+", "Add property");
  headAdd.addEventListener("mousedown", (e) => {
    e.preventDefault();
    addRow("", "", "key", true);
  });
  head.append(headLabel, headAdd);
  card.appendChild(head);

  const rows = document.createElement("div");
  rows.className = "fmw-rows";
  card.appendChild(rows);

  const uid = Math.random().toString(36).slice(2, 8);
  const keyList = document.createElement("datalist");
  keyList.id = `fmw-keys-${uid}`;
  const valList = document.createElement("datalist");
  valList.id = `fmw-vals-${uid}`;
  card.append(keyList, valList);

  // Corpus-wide suggestions, read lazily so they're always current.
  const fillKeySuggestions = () => {
    const index = view.state.facet(docsIndexFacet);
    if (!index) return;
    const keys = new Set<string>();
    for (const d of index.docs.values()) for (const [k] of d.frontmatter) keys.add(k);
    keyList.replaceChildren(
      ...[...keys].sort().map((k) => Object.assign(document.createElement("option"), { value: k })),
    );
  };
  const fillValueSuggestions = (key: string) => {
    const index = view.state.facet(docsIndexFacet);
    if (!index) return;
    const k = key.toLowerCase();
    const values = new Set<string>();
    for (const d of index.docs.values()) {
      for (const [pk, pv] of d.frontmatter) if (pk.toLowerCase() === k && pv) values.add(pv);
    }
    valList.replaceChildren(
      ...[...values].sort().map((v) => Object.assign(document.createElement("option"), { value: v })),
    );
  };

  const readRows = (): [string, string][] =>
    [...rows.querySelectorAll<HTMLElement>(".fmw-row")].map((r) => {
      const key = (r.querySelector<HTMLInputElement>(".fmw-key")!).value.trim();
      const val = (r.querySelector<HTMLInputElement>(".fmw-val")!).value.trim();
      // An untouched value keeps its original scalar (quotes intact), so
      // clicking through the card never rewrites the document.
      const raw = (r as HTMLElement & { _fmwRaw?: string })._fmwRaw;
      return [key, raw !== undefined && unquote(raw) === val ? raw : val];
    });

  // One document change per editing session. Refuses while an invalid key is
  // present (committing it would demote the whole block to body text) — the
  // row stays marked red until fixed or emptied.
  const commit = () => {
    const all = readRows();
    const kept = all.filter(([k, v]) => k !== "" || v !== "");
    if (kept.some(([k]) => !isValidFmKey(k))) return;
    const cur = fmBlock(view.state);
    if (!cur || cur.text !== block.text) return; // stale card: the doc moved on
    const next = serializeFrontmatter(kept);
    if (next === "") {
      // Last property removed: the block goes, taking its trailing newline.
      const hasNewline = block.to < view.state.doc.length;
      view.dispatch({ changes: { from: 0, to: block.to + (hasNewline ? 1 : 0), insert: "" } });
      return;
    }
    const nextText = next.slice(0, -1); // block text carries no trailing \n
    if (nextText !== block.text) {
      view.dispatch({ changes: { from: 0, to: block.to, insert: nextText } });
    } else if (all.length !== kept.length) {
      // Nothing changed on disk but empty draft rows linger in the DOM.
      for (const r of [...rows.querySelectorAll<HTMLElement>(".fmw-row")]) {
        const k = r.querySelector<HTMLInputElement>(".fmw-key")!.value.trim();
        const v = r.querySelector<HTMLInputElement>(".fmw-val")!.value.trim();
        if (k === "" && v === "") r.remove();
      }
    }
  };

  const markValidity = (input: HTMLInputElement) => {
    const k = input.value.trim();
    input.classList.toggle("invalid", k !== "" && !isValidFmKey(k));
  };

  const addRow = (key: string, value: string, focus?: "key" | "value", draft = false, raw?: string) => {
    const row = document.createElement("div");
    row.className = "fmw-row";
    (row as HTMLElement & { _fmwRaw?: string })._fmwRaw = raw;

    const keyInput = document.createElement("input");
    keyInput.className = "fmw-key";
    keyInput.value = key;
    keyInput.placeholder = "name";
    keyInput.spellcheck = false;
    keyInput.setAttribute("list", keyList.id);
    keyInput.addEventListener("focus", fillKeySuggestions);
    keyInput.addEventListener("input", () => markValidity(keyInput));

    const valInput = document.createElement("input");
    valInput.className = "fmw-val";
    valInput.value = value;
    valInput.placeholder = "value";
    valInput.spellcheck = false;
    valInput.setAttribute("list", valList.id);
    valInput.addEventListener("focus", () => fillValueSuggestions(keyInput.value.trim()));

    for (const input of [keyInput, valInput]) {
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          input.blur(); // focusout commits once focus leaves the card
        } else if (e.key === "Escape") {
          // Revert this row (drafts vanish), then let focusout no-op.
          e.preventDefault();
          if (draft) {
            row.remove();
          } else {
            keyInput.value = key;
            valInput.value = value;
            markValidity(keyInput);
          }
          input.blur();
        }
      });
    }

    const filterBtn = ghostButton("fmw-tool", "⌕", "Find notes with this property");
    filterBtn.addEventListener("mousedown", (e) => {
      e.preventDefault();
      const nav = view.state.facet(docsNavFacet);
      nav?.onFilter?.(keyInput.value.trim(), valInput.value.trim());
    });

    const delBtn = ghostButton("fmw-tool fmw-del", "✕", "Remove property");
    delBtn.addEventListener("mousedown", (e) => {
      e.preventDefault();
      row.remove();
      commit();
    });

    row.append(keyInput, valInput, filterBtn, delBtn);
    rows.appendChild(row);
    if (focus) (focus === "key" ? keyInput : valInput).focus();
  };

  block.pairs.forEach(([k, v], i) => addRow(k, v, undefined, false, block.raws[i]));

  // Commit when focus leaves the card entirely (Tab between inputs stays in).
  card.addEventListener("focusout", (e) => {
    const to = (e as FocusEvent).relatedTarget as Node | null;
    if (to && card.contains(to)) return;
    commit();
  });

  (card as HTMLElement & { _fmwAddRow?: () => void })._fmwAddRow = () => addRow("", "", "key", true);

  if (pendingAdd) {
    pendingAdd = false;
    // The card isn't in the document yet; focus once it lands.
    setTimeout(() => addRow("", "", "key", true), 0);
  }

  return card;
}

function buildDeco(state: EditorState): DecorationSet {
  const block = fmBlock(state);
  if (!block) return Decoration.none;
  return Decoration.set([
    Decoration.replace({ widget: new FmWidget(block), block: true }).range(0, block.to),
  ]);
}

const fmField = StateField.define<DecorationSet>({
  create: buildDeco,
  update(value, tr) {
    return tr.docChanged ? buildDeco(tr.state) : value;
  },
  provide: (f) => [
    EditorView.decorations.from(f),
    // Atomic so arrow keys hop over the block instead of stranding the
    // cursor inside replaced text.
    EditorView.atomicRanges.of((view) => view.state.field(f)),
  ],
});

/** The properties-card extension for the docs editor. */
export const frontmatterEditor: Extension = [fmField];
