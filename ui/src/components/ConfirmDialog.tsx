import { ReactNode } from "react";
import { useModalKeys } from "../hooks/useModalKeys";
import { CloneProgress } from "../api";
import ModalBackdrop from "./ModalBackdrop";
import ProgressReadout from "./ProgressReadout";

export default function ConfirmDialog({
  title,
  body,
  confirmLabel,
  danger,
  busy,
  confirmDisabled,
  progress,
  progressLabel,
  altLabel,
  altDanger,
  onAlt,
  hushLabel,
  hushed,
  onHush,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  /** Disables both buttons (and Escape) while the confirmed action runs. */
  busy?: boolean;
  /**
   * Disables only the confirm. For a dialog still reading what it is about to
   * say: going ahead early would do the right thing under the wrong
   * explanation, but Cancel and Escape must keep working throughout.
   */
  confirmDisabled?: boolean;
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
  /**
   * Offers to stop asking this question. The box only takes effect if the user
   * goes through with the action, so ticking it and then cancelling changes
   * nothing: the caller reads `hushed` in its own onConfirm.
   */
  hushLabel?: string;
  hushed?: boolean;
  onHush?: (next: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useModalKeys(onCancel, !busy);
  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onCancel}>
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
          {hushLabel && onHush && (
            <label className="modal-hush">
              <input
                type="checkbox"
                checked={!!hushed}
                disabled={busy}
                onChange={(e) => onHush(e.target.checked)}
              />
              <span>{hushLabel}</span>
            </label>
          )}
          {/* Autofocused so Enter/Escape act immediately — and Enter lands on
              the safe option, not the (possibly destructive) confirm. */}
          <button className="btn-secondary" autoFocus disabled={busy} onClick={onCancel}>Cancel</button>
          {altLabel && onAlt && (
            <button className={altDanger ? "btn-danger" : "btn-secondary"} disabled={busy} onClick={onAlt}>
              {altLabel}
            </button>
          )}
          <button
            className={danger ? "btn-danger" : "btn-primary"}
            disabled={busy || confirmDisabled}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
