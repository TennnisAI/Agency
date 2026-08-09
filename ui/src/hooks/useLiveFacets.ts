import { RefObject, useCallback, useEffect, useRef } from "react";
import { Compartment, Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { DocsIndex } from "../lib/docsIndex";
import { CrossRefs } from "../lib/links";
import { issueLabel } from "../lib/issues";
import { crossRefsFacet, docsIndexFacet } from "../lib/livePreview";

// The docs index and the cross-project refs reach a live editor through two
// Compartments. Both objects are rebuilt by their polls far more often than
// their *editor-visible* content changes — the corpus index is rebuilt after
// every save, your own autosave included — and each reconfigure makes
// livePreview rebuild every decoration and CodeMirror redraw the viewport,
// which is both wasted work and the kind of thing that can disturb a live
// selection or an in-progress IME composition. So the reconfigure is gated on
// a signature of exactly what the editor's extensions read.
//
// Anything a facet consumer reads has to be in that signature, or the editor
// goes stale. Today's readers:
//   index — wikilink resolution (paths), the [[ completion (paths + titles),
//           the #tag completion (tags), and the properties card's key/value
//           suggestions (frontmatter);
//   cross — typed wikilink resolution and tooltips (project issue keys,
//           labels, run ids) and the [[ completion's issue entries (labels,
//           titles, and the project name they sort by).
// Note bodies are read by neither, which is what makes the gate worth having.

// Fields are joined with a newline: every value below is single-line by
// construction (a path, a heading's text, a tag, one frontmatter scalar), so
// no two different corpora join to the same string.
const FIELD = "\n";

/** Signature of the index data the editor's extensions read. */
export function docsIndexSignature(index: DocsIndex | null): string {
  // A null index means "still loading" and resolves wikilinks differently from
  // an empty corpus, so the count leads: the two never share a signature.
  if (!index) return "";
  const parts: string[] = [String(index.docs.size)];
  for (const d of index.docs.values()) {
    parts.push(d.path, d.title, d.tags.join(","), d.frontmatter.map(([k, v]) => `${k}=${v}`).join(","));
  }
  return parts.join(FIELD);
}

/** Signature of the cross-ref data the editor's extensions read. */
export function crossRefsSignature(cross: CrossRefs | null): string {
  // Same as above: null cross refs resolve typed targets differently from a
  // workspace that simply has no issues.
  if (!cross) return "";
  const parts = [String(cross.issuesById.size), [...cross.keyPrefixes].sort().join(",")];
  for (const ref of cross.issuesById.values()) {
    parts.push(issueLabel(ref.project, ref.issue), ref.issue.title, ref.project.name);
  }
  for (const id of cross.runsById.keys()) parts.push(id);
  return parts.join(FIELD);
}

/**
 * Feeds the docs index and the cross-project refs to a CodeMirror view,
 * skipping the reconfigures that would not change a single decoration or
 * completion (see above). Returns the compartment contents for a state being
 * built now: the callers build theirs inside async callbacks, where a captured
 * prop is a render behind, so the latest data is read from refs at call time.
 */
export function useLiveFacets(
  viewRef: RefObject<EditorView | null>,
  index: DocsIndex | null,
  cross: CrossRefs | null,
): () => Extension[] {
  const indexComp = useRef(new Compartment());
  const crossComp = useRef(new Compartment());
  const indexRef = useRef(index);
  indexRef.current = index;
  const crossRef = useRef(cross);
  crossRef.current = cross;
  // The signature of what the compartments hold; null until the first pass.
  const indexSig = useRef<string | null>(null);
  const crossSig = useRef<string | null>(null);

  useEffect(() => {
    const sig = docsIndexSignature(index);
    if (sig === indexSig.current) return;
    indexSig.current = sig;
    // Ship-gate probe: typing in a note logs nothing (its own saves rebuild
    // the index, but never change what this editor reads from it).
    if (import.meta.env.DEV && viewRef.current) console.debug("[docs] index facet reconfigure");
    viewRef.current?.dispatch({
      effects: indexComp.current.reconfigure(docsIndexFacet.of(index)),
    });
  }, [index, viewRef]);

  useEffect(() => {
    const sig = crossRefsSignature(cross);
    if (sig === crossSig.current) return;
    crossSig.current = sig;
    // Same probe: an idle note logs nothing on the 5s cross-ref poll.
    if (import.meta.env.DEV && viewRef.current) console.debug("[docs] cross-ref facet reconfigure");
    viewRef.current?.dispatch({
      effects: crossComp.current.reconfigure(crossRefsFacet.of(cross)),
    });
  }, [cross, viewRef]);

  return useCallback(() => [
    indexComp.current.of(docsIndexFacet.of(indexRef.current)),
    crossComp.current.of(crossRefsFacet.of(crossRef.current)),
  ], []);
}
