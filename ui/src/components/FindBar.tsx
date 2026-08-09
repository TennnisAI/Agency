import { useEffect, useRef } from "react";
import { FindQuery, FindState } from "../lib/find";

/**
 * The one find/replace bar, shared by the notes editor, the file editor and the
 * issue description. It owns no search logic — everything runs through the
 * [`FindEngine`](../lib/find.ts) its owner supplies — so the same keys, the
 * same options and the same "3 / 12" mean the same thing everywhere.
 *
 * `float` hangs it over the top-right of an editor (nothing reflows, the line
 * you were reading stays put); `inline` puts it in normal flow, for panes that
 * scroll their own content and would otherwise scroll away from it.
 */
export default function FindBar({
  query,
  state,
  showReplace,
  canReplace,
  variant = "float",
  focusField,
  onQuery,
  onToggleReplace,
  onStep,
  onReplaceOne,
  onReplaceAll,
  onClose,
}: {
  query: FindQuery;
  state: FindState;
  showReplace: boolean;
  canReplace: boolean;
  variant?: "float" | "inline";
  /** Bumped by the owner to pull focus back into a field (a second ⌘F). */
  focusField: { field: "find" | "replace"; nonce: number };
  onQuery: (q: FindQuery) => void;
  onToggleReplace: () => void;
  onStep: (back: boolean) => void;
  onReplaceOne: () => void;
  onReplaceAll: () => void;
  onClose: () => void;
}) {
  const findRef = useRef<HTMLInputElement>(null);
  const replaceRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const el = focusField.field === "replace" ? replaceRef.current : findRef.current;
    el?.focus();
    el?.select();
  }, [focusField]);

  const set = (patch: Partial<FindQuery>) => onQuery({ ...query, ...patch });

  // Escape always closes, from either field; Enter walks the matches. Handled
  // here rather than on the window so a find bar can never swallow keys meant
  // for another pane.
  const onKey = (e: React.KeyboardEvent, enter: () => void) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      enter();
    }
  };

  // "500+" means counting stopped at the cap (lib/find.ts), not that the
  // number is a guess — navigation still reaches every match.
  const tally = `${state.total}${state.capped ? "+" : ""}`;
  const summary = state.invalid
    ? "bad pattern"
    : !query.search
      ? ""
      : state.total === 0
        ? "no results"
        : state.current === 0
          ? `${tally} matches`
          : `${state.current} of ${tally}`;

  const toggle = (on: boolean, label: string, title: string, onClick: () => void, cls = "") => (
    <button
      type="button"
      className={`find-opt${on ? " on" : ""}${cls ? ` ${cls}` : ""}`}
      title={title}
      aria-label={title}
      aria-pressed={on}
      // Keep the press from blurring the field: the caret has to stay where it
      // is so the option lands on the query being typed.
      onMouseDown={(e) => e.preventDefault()}
      onClick={onClick}
    >
      {label}
    </button>
  );

  return (
    <div className={`find-bar${variant === "inline" ? " inline" : ""}`} role="search">
      <div className="find-row">
        <input
          ref={findRef}
          className={`find-input${state.invalid ? " invalid" : ""}`}
          placeholder="Find"
          aria-label="Find"
          spellCheck={false}
          value={query.search}
          onChange={(e) => set({ search: e.target.value })}
          onKeyDown={(e) => onKey(e, () => onStep(e.shiftKey))}
        />
        <span className="find-count">{summary}</span>
        <span className="find-opts">
          {toggle(query.caseSensitive, "Aa", "Match case", () => set({ caseSensitive: !query.caseSensitive }))}
          {toggle(query.wholeWord, "ab", "Whole word", () => set({ wholeWord: !query.wholeWord }), "find-opt-word")}
          {toggle(query.regexp, ".*", "Regular expression", () => set({ regexp: !query.regexp }))}
        </span>
        <button type="button" className="find-btn" title="Previous match (⇧↵ or ⇧⌘G)" aria-label="Previous match"
          onMouseDown={(e) => e.preventDefault()} onClick={() => onStep(true)}>↑</button>
        <button type="button" className="find-btn" title="Next match (↵ or ⌘G)" aria-label="Next match"
          onMouseDown={(e) => e.preventDefault()} onClick={() => onStep(false)}>↓</button>
        {canReplace && (
          <button
            type="button"
            className={`find-btn${showReplace ? " on" : ""}`}
            title={showReplace ? "Hide replace" : "Show replace (⌥⌘F)"}
            aria-label="Toggle replace"
            aria-expanded={showReplace}
            onMouseDown={(e) => e.preventDefault()}
            onClick={onToggleReplace}
          >⇄</button>
        )}
        <button type="button" className="find-close" title="Close (Esc)" aria-label="Close find"
          onMouseDown={(e) => e.preventDefault()} onClick={onClose}>✕</button>
      </div>
      {canReplace && showReplace && (
        <div className="find-row">
          <input
            ref={replaceRef}
            className="find-input"
            placeholder={query.regexp ? "Replace ($1 for groups)" : "Replace"}
            aria-label="Replace with"
            spellCheck={false}
            value={query.replace}
            onChange={(e) => set({ replace: e.target.value })}
            onKeyDown={(e) => onKey(e, onReplaceOne)}
          />
          <button type="button" className="find-action" title="Replace this match (↵)"
            disabled={!query.search || state.invalid}
            onMouseDown={(e) => e.preventDefault()} onClick={onReplaceOne}>Replace</button>
          <button type="button" className="find-action" title="Replace every match"
            disabled={!query.search || state.invalid || state.total === 0}
            onMouseDown={(e) => e.preventDefault()} onClick={onReplaceAll}>All</button>
        </div>
      )}
    </div>
  );
}
