import { ReactNode, RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import FindBar from "../components/FindBar";
import { EMPTY_FIND_QUERY, FindEngine, FindMode, FindQuery, FindState, NO_FIND_STATE } from "../lib/find";
import { registerFindTarget } from "../lib/findBus";

/**
 * Give one surface a find bar and hand it to ⌘F.
 *
 * The caller supplies a [`FindEngine`] and the element that owns the search
 * (content *and* bar, so focus resolution in `findBus` stays inside it). What
 * comes back is the bar to render and a `onContentChange` to call whenever the
 * content moves underneath — the counts are recomputed from it, which is the
 * one thing an engine can't notice on its own.
 */
export function useFind(opts: {
  host: RefObject<HTMLElement | null>;
  engine: FindEngine;
  canReplace: boolean;
  rank: number;
  requiresFocus?: boolean;
  variant?: "float" | "inline";
  /**
   * False while there is nothing to search — a file showing its rendered
   * preview, an image, a PDF. The target stops resolving, so ⌘F falls through
   * to whatever else is on screen instead of opening a bar over content the
   * user can't see.
   */
  enabled?: boolean;
  /** Close the bar when this changes — a different issue, note or file. */
  resetKey?: string;
}): { bar: ReactNode; onContentChange: () => void } {
  const { host, engine, canReplace, rank, requiresFocus, variant, resetKey } = opts;
  const enabled = opts.enabled ?? true;
  const [open, setOpen] = useState(false);
  const [showReplace, setShowReplace] = useState(false);
  const [query, setQuery] = useState<FindQuery>(EMPTY_FIND_QUERY);
  const [state, setState] = useState<FindState>(NO_FIND_STATE);
  // Bumped to pull focus into a field; a second ⌘F re-selects what's there.
  const [focusField, setFocusField] = useState({ field: "find" as "find" | "replace", nonce: 0 });

  // The registration is bound once, so everything it reaches for lives in refs.
  const live = useRef({ engine, open, query, canReplace, enabled, host });
  live.current = { engine, open, query, canReplace, enabled, host };

  const openBar = useCallback((mode: FindMode) => {
    const { engine: e, open: isOpen, query: q, canReplace: replaceable } = live.current;
    const wantReplace = mode === "replace" && replaceable;
    let next = q;
    if (!isOpen) {
      // Seed from the selection, the way every editor's ⌘F does — but never
      // wipe a query that's already there when there's nothing selected.
      const selected = e.selectedText();
      next = { ...q, search: selected || q.search };
      setQuery(next);
      setOpen(true);
    }
    if (wantReplace) setShowReplace(true);
    setState(e.sync(next));
    setFocusField((f) => ({ field: wantReplace ? "replace" : "find", nonce: f.nonce + 1 }));
  }, []);

  const close = useCallback(() => {
    setOpen(false);
    live.current.engine.dismiss();
    live.current.engine.refocus();
  }, []);

  const step = useCallback((back: boolean) => {
    const { engine: e, open: isOpen, query: q } = live.current;
    // Nothing to step through yet, so ⌘G means "start a search" — the same
    // thing every editor does when Find Next has no query behind it.
    if (!q.search) {
      openBar("find");
      return;
    }
    setState(e.step(q, back));
    // With the bar closed the jump is all the user asked for: the selection is
    // the feedback, and lighting every match up would leave highlighting on
    // with no bar to press Escape in.
    if (!isOpen) e.dismiss();
  }, [openBar]);

  useEffect(() => {
    return registerFindTarget({
      host: () => (live.current.enabled ? live.current.host.current : null),
      open: openBar,
      step,
      canReplace,
      rank,
      requiresFocus,
    });
  }, [openBar, step, canReplace, rank, requiresFocus]);

  // A different document behind the same bar is a different search.
  useEffect(() => {
    if (resetKey === undefined) return;
    setOpen(false);
    setState(NO_FIND_STATE);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetKey]);

  // Counts go stale when the content changes under an open bar (an edit in the
  // pane behind it, a note rewritten on disk). Coalesced so a burst of
  // keystrokes costs one scan.
  const recountTimer = useRef<number | null>(null);
  const onContentChange = useCallback(() => {
    if (!live.current.open || recountTimer.current !== null) return;
    recountTimer.current = window.setTimeout(() => {
      recountTimer.current = null;
      const { engine: e, open: isOpen, query: q } = live.current;
      if (isOpen && q.search) setState(e.recount(q));
    }, 120);
  }, []);
  useEffect(() => () => {
    if (recountTimer.current !== null) window.clearTimeout(recountTimer.current);
  }, []);

  const bar = useMemo(() => {
    if (!open || !enabled) return null;
    return (
      <FindBar
        query={query}
        state={state}
        showReplace={showReplace}
        canReplace={canReplace}
        variant={variant}
        focusField={focusField}
        onQuery={(q) => { setQuery(q); setState(engine.sync(q)); }}
        onToggleReplace={() => setShowReplace((s) => !s)}
        onStep={step}
        onReplaceOne={() => setState(engine.replaceOne(live.current.query))}
        onReplaceAll={() => setState(engine.replaceAll(live.current.query))}
        onClose={close}
      />
    );
  }, [open, enabled, query, state, showReplace, canReplace, variant, focusField, engine, step, close]);

  return { bar, onContentChange };
}
