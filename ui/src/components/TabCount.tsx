import { RunTab, tabCountLabel, tabsTitle } from "../lib/runTabs";

/**
 * How many agent tabs a workspace holds, beside a run listed by one name
 * (AGE-225). With `onToggle` it is also the disclosure that lists them under
 * the row; without, a plain count for a surface that only surveys (a tile).
 *
 * A span rather than a button because every host is itself a clickable row,
 * and a button inside a button is invalid markup: the tab strip's close ✕ is
 * the same arrangement. Clicks stop here so they do not also open the run.
 */
export default function TabCount({
  tabs,
  open = false,
  onToggle,
}: {
  tabs: RunTab[];
  open?: boolean;
  onToggle?: () => void;
}) {
  const title = tabsTitle(tabs);
  if (!onToggle) {
    return <span className="tab-count" title={title}>{tabCountLabel(tabs.length)}</span>;
  }
  return (
    <span
      role="button"
      aria-expanded={open}
      className={`tab-count toggle${open ? " open" : ""}`}
      title={`${title}. Click to ${open ? "hide" : "list"} them.`}
      onClick={(e) => { e.stopPropagation(); onToggle(); }}
      // The rail row renames on a double-click; toggling twice is not a rename.
      onDoubleClick={(e) => e.stopPropagation()}
    >
      {tabs.length}
      <span className="tab-count-chev" aria-hidden>{open ? "▾" : "▸"}</span>
    </span>
  );
}
