import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Issue } from "../api";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS } from "../lib/issues";
import { BatchPatch, sharedValue } from "../lib/issueBatch";
import { MenuCoords, anchorMenu } from "../lib/menuAnchor";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { useMenuAnchor } from "../hooks/useMenuAnchor";
import { PriorityGlyph, StatusDot } from "./IssueRow";
import DatePicker from "./DatePicker";

type Menu = "status" | "priority";
type DateField = "due" | "scheduled";

// The strip that floats over the bottom of the board while rows are checked:
// how many, the fields a batch can set, and the ways out. Each menu marks the
// value only when every checked issue shares it, so a mixed batch reads as
// mixed rather than as whatever the first row happens to hold.
export default function IssueBatchBar({
  issues,
  visibleCount,
  compact,
  onPatch,
  onDelete,
  onSelectAll,
  onClear,
}: {
  // The checked issues, in board order.
  issues: Issue[];
  // How many rows the board shows, for "Select all".
  visibleCount: number;
  // The list is a sidebar beside the expanded detail pane.
  compact: boolean;
  onPatch: (patch: BatchPatch) => void;
  onDelete: () => void;
  onSelectAll: () => void;
  onClear: () => void;
}) {
  const [menu, setMenu] = useState<Menu | null>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [picker, setPicker] = useState<{ field: DateField; coords: MenuCoords } | null>(null);
  const statusRef = useRef<HTMLButtonElement>(null);
  const prioRef = useRef<HTMLButtonElement>(null);
  const dueRef = useRef<HTMLButtonElement>(null);
  const schedRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const coords = useMenuAnchor(menu ? rect : null, menuRef);
  useDismissOnResize(menu !== null, () => setMenu(null));

  const openMenu = (which: Menu, ref: React.RefObject<HTMLButtonElement>) => {
    setRect(ref.current?.getBoundingClientRect() ?? null);
    setMenu(which);
  };
  const openPicker = (field: DateField, ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setPicker({ field, coords: anchorMenu(r, 240, 260) }); // .date-picker's size, as DateProp
  };
  const apply = (patch: BatchPatch) => {
    setMenu(null);
    setPicker(null);
    onPatch(patch);
  };

  const status = sharedValue(issues, "status");
  const priority = sharedValue(issues, "priority");
  const n = issues.length;

  return (
    <div className={`issue-batch-bar${compact ? " compact" : ""}`} role="toolbar" aria-label="Edit checked issues">
      <span className="issue-batch-count">{n} selected</span>
      <button ref={statusRef} className="issue-batch-btn" title="Set status" onClick={() => openMenu("status", statusRef)}>
        {status ? <StatusDot status={status} /> : <span className="issue-batch-mixed" />}
        Status
      </button>
      <button ref={prioRef} className="issue-batch-btn" title="Set priority (0-4)" onClick={() => openMenu("priority", prioRef)}>
        {priority !== undefined ? <PriorityGlyph priority={priority} /> : <span className="issue-batch-mixed" />}
        Priority
      </button>
      <button ref={dueRef} className="issue-batch-btn" title="Set due date" onClick={() => openPicker("due", dueRef)}>
        <span aria-hidden>◷</span> Due
      </button>
      <button ref={schedRef} className="issue-batch-btn" title="Set scheduled date" onClick={() => openPicker("scheduled", schedRef)}>
        <span aria-hidden>⧖</span> Scheduled
      </button>
      <button className="issue-batch-btn danger" title="Delete the checked issues" onClick={onDelete}>
        Delete…
      </button>
      <span className="issue-batch-spacer" />
      {n < visibleCount && (
        <button className="issue-batch-btn quiet" title="Check every issue on the board" onClick={onSelectAll}>
          Select all {visibleCount}
        </button>
      )}
      <button className="icon-btn" title="Clear selection (Esc)" onClick={onClear}>✕</button>

      {menu && createPortal(
        <>
          <div className="agent-menu-backdrop" onClick={() => setMenu(null)} />
          <div ref={menuRef} className="agent-menu" style={{ position: "fixed", ...coords }}>
            {menu === "status" &&
              ISSUE_STATUSES.map((s) => (
                <button key={s} onClick={() => apply({ status: s })}>
                  <StatusDot status={s} /> {STATUS_LABELS[s]}{s === status ? " ✓" : ""}
                </button>
              ))}
            {menu === "priority" &&
              PRIORITY_LABELS.map((p, v) => (
                <button key={p} onClick={() => apply({ priority: v })}>
                  <PriorityGlyph priority={v} /> {p}{v === priority ? " ✓" : ""}
                </button>
              ))}
          </div>
        </>,
        document.body,
      )}
      {picker && createPortal(
        <DatePicker
          // A date only when the batch already agrees on one; otherwise the
          // calendar opens on today with nothing marked.
          value={sharedValue(issues, picker.field) ?? null}
          coords={picker.coords}
          clearable={issues.some((i) => i[picker.field] != null)}
          onPick={(d) => apply({ [picker.field]: d })}
          onClear={() => apply({ [picker.field]: null })}
          onClose={() => setPicker(null)}
        />,
        document.body,
      )}
    </div>
  );
}
