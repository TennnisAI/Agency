import { useEffect, useMemo, useRef, useState } from "react";
import { IssueRef } from "../lib/links";
import { issueLabel } from "../lib/issues";
import { fuzzyFilter } from "../lib/fuzzy";
import { MenuCoords, anchorMenu } from "../lib/menuAnchor";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { StatusDot } from "./IssueRow";

// Matches `.issue-link-menu` in styles.css — the anchor is computed, not
// measured, like every other popover here.
const MENU_W = 320;
const MENU_H = 300;

/**
 * "+" beside the Links heading: pick an issue to link this one to. Every
 * loaded issue is on offer (a link may cross projects), searched by key and
 * title; already-linked issues and this one are filtered out by the caller.
 */
export default function IssueLinkMenu({
  candidates,
  onPick,
}: {
  candidates: IssueRef[];
  onPick: (ref: IssueRef) => void;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<MenuCoords>({});
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const btnRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const shown = useMemo(
    () => fuzzyFilter(query, candidates, (ref) => `${issueLabel(ref.project, ref.issue)} ${ref.issue.title}`, 60),
    [query, candidates],
  );
  // A narrowed list can be shorter than where the cursor was.
  const cursor = Math.min(active, Math.max(shown.length - 1, 0));

  useEffect(() => { if (open) inputRef.current?.focus(); }, [open]);
  useDismissOnResize(open, () => setOpen(false));

  const toggle = () => {
    const r = btnRef.current?.getBoundingClientRect();
    if (r) setCoords(anchorMenu(r, MENU_W, MENU_H));
    setQuery("");
    setActive(0);
    setOpen((o) => !o);
  };

  const pick = (ref: IssueRef) => {
    setOpen(false);
    onPick(ref);
  };

  return (
    <>
      <button
        ref={btnRef}
        className="icon-btn issue-link-add"
        title={candidates.length > 0 ? "Link another issue" : "No other issue to link"}
        disabled={candidates.length === 0}
        onClick={toggle}
      >
        +
      </button>
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu issue-link-menu" style={{ position: "fixed", ...coords }}>
            <input
              ref={inputRef}
              className="issue-link-search"
              placeholder="Search issues…"
              value={query}
              onChange={(e) => { setQuery(e.target.value); setActive(0); }}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.stopPropagation();
                  setOpen(false);
                } else if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setActive(Math.min(cursor + 1, shown.length - 1));
                } else if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setActive(Math.max(cursor - 1, 0));
                } else if (e.key === "Enter" && shown[cursor]) {
                  e.preventDefault();
                  pick(shown[cursor]);
                }
              }}
            />
            <div className="issue-link-options">
              {shown.length === 0 && <div className="issue-link-empty">No match</div>}
              {shown.map((ref, i) => (
                <button
                  key={ref.issue.id}
                  className={i === cursor ? "on" : ""}
                  title={`${ref.project.name} · ${ref.issue.title}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(ref)}
                >
                  <StatusDot status={ref.issue.status} />
                  <code className="issue-link-key">{issueLabel(ref.project, ref.issue)}</code>
                  <span className="issue-link-title">{ref.issue.title}</span>
                </button>
              ))}
            </div>
          </div>
        </>
      )}
    </>
  );
}
