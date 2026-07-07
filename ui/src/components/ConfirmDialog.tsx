import { useModalKeys } from "../hooks/useModalKeys";

export default function ConfirmDialog({
  title,
  body,
  confirmLabel,
  danger,
  busy,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  /** Disables both buttons (and Escape) while the confirmed action runs. */
  busy?: boolean;
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
        <div className="modal-body">{body}</div>
        <div className="modal-foot">
          {/* Autofocused so Enter/Escape act immediately — and Enter lands on
              the safe option, not the (possibly destructive) confirm. */}
          <button className="btn-secondary" autoFocus disabled={busy} onClick={onCancel}>Cancel</button>
          <button className={danger ? "btn-danger" : "btn-primary"} disabled={busy} onClick={onConfirm}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
