import { useState } from "react";
import { useModalKeys } from "../hooks/useModalKeys";

/** Single-text-input modal, styled like ConfirmDialog. */
export default function PromptDialog({
  title,
  body,
  placeholder,
  initial = "",
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  body?: string;
  placeholder?: string;
  initial?: string;
  confirmLabel: string;
  onConfirm: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  useModalKeys(onCancel, true);
  const submit = () => { if (value.trim()) onConfirm(value.trim()); };
  return (
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); onCancel(); }}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          {body && <p className="modal-note">{body}</p>}
          <input className="modal-input" autoFocus value={value} placeholder={placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") submit(); }} />
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onCancel}>Cancel</button>
          <button className="btn-primary" disabled={!value.trim()} onClick={submit}>{confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}
