import { useState } from "react";
import { monthGrid, monthLabel, shiftMonth } from "../lib/calendar";
import { dateStamp } from "../lib/dailyNote";
import { useModalKeys } from "../hooks/useModalKeys";

// In-app calendar popover — the native date picker can't be dismissed without
// choosing a date in this webview, so dates use the same backdrop + Escape
// pattern as every other menu. Anchored via fixed coords from the trigger.
export default function DatePicker({
  value,
  coords,
  onPick,
  onClear,
  onClose,
}: {
  value: string | null;
  coords: { top: number; left?: number; right?: number };
  // onPick/onClear also close — the caller owns the open state.
  onPick: (date: string) => void;
  onClear: () => void;
  onClose: () => void;
}) {
  const today = dateStamp(new Date());
  const anchor = value ?? today;
  const [[year, month], setYm] = useState<[number, number]>([
    Number(anchor.slice(0, 4)),
    Number(anchor.slice(5, 7)),
  ]);
  useModalKeys(onClose);

  return (
    <>
      <div className="agent-menu-backdrop" onClick={onClose} />
      <div className="agent-menu date-picker" style={{ position: "fixed", ...coords }}>
        <div className="date-picker-head">
          <button className="icon-btn" title="Previous month" onClick={() => setYm(shiftMonth(year, month, -1))}>‹</button>
          <span className="date-picker-label">{monthLabel(year, month)}</span>
          <button className="icon-btn" title="Next month" onClick={() => setYm(shiftMonth(year, month, 1))}>›</button>
        </div>
        <div className="date-picker-grid">
          {["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map((d) => (
            <span key={d} className="date-picker-dow">{d}</span>
          ))}
          {monthGrid(year, month).flat().map((c) => (
            <button
              key={c.date}
              className={
                "date-cell" +
                (c.inMonth ? "" : " out") +
                (c.date === value ? " selected" : "") +
                (c.date === today ? " today" : "")
              }
              title={c.date}
              onClick={() => onPick(c.date)}
            >
              {c.day}
            </button>
          ))}
        </div>
        <div className="date-picker-foot">
          <button className="ghost" onClick={() => onPick(today)}>Today</button>
          <div className="spacer" />
          {value && <button className="ghost" onClick={onClear}>Clear</button>}
        </div>
      </div>
    </>
  );
}
