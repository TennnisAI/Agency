import { useModalKeys } from "../hooks/useModalKeys";
import { CloneProgress } from "../api";
import ProgressReadout from "./ProgressReadout";

export default function ConfirmDialog({
  title,
  body,
  confirmLabel,
  danger,
  busy,
  progress,
  progressLabel,
  altLabel,
  altDanger,
  onAlt,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  /** Disables both buttons (and Escape) while the confirmed action runs. */
  busy?: boolean;
  /**
   * Latest step of the confirmed action, when it streams progress. Rendered
   * with `progressLabel` while `busy`, so a slow one (an agent teardown, say)
   * says what it is doing instead of sitting on a disabled dialog.
   */
  progress?: CloneProgress | null;
  /** Phase to show until the first `progress` update lands. */
  progressLabel?: string;
  /** Optional second action rendered between Cancel and the confirm button. */
  altLabel?: string;
  altDanger?: boolean;
  onAlt?: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useModalKeys(onCancel, !busy);
  return (
    // stopPropagation: dialogs are often rendered inside clickable hosts (e.g.
    // an agent tile) — a backdrop-cancel click must not bubble into the host's
    // own onClick and navigate away.
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); if (!busy) onCancel(); }}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          {body}
          {busy && progressLabel && (
            <ProgressReadout progress={progress ?? null} fallback={progressLabel} />
          )}
        </div>
        <div className="modal-foot">
          {/* Autofocused so Enter/Escape act immediately — and Enter lands on
              the safe option, not the (possibly destructive) confirm. */}
          <button className="btn-secondary" autoFocus disabled={busy} onClick={onCancel}>Cancel</button>
          {altLabel && onAlt && (
            <button className={altDanger ? "btn-danger" : "btn-secondary"} disabled={busy} onClick={onAlt}>
              {altLabel}
            </button>
          )}
          <button className={danger ? "btn-danger" : "btn-primary"} disabled={busy} onClick={onConfirm}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
