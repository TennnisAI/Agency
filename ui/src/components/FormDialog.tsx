import { ReactNode } from "react";
import { useModalKeys } from "../hooks/useModalKeys";

// A labelled form field: name, an optional plain-language explanation of what
// the setting does, then the control. The whole thing is a <label>, so clicking
// the name or the explanation focuses the input.
export function Field({
  label,
  hint,
  optional,
  children,
}: {
  label: string;
  hint?: ReactNode;
  optional?: boolean;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field-head">
        <span className="field-label">{label}</span>
        {optional && <span className="field-optional">optional</span>}
      </span>
      {hint && <span className="field-hint">{hint}</span>}
      {children}
    </label>
  );
}

// Modal shell for settings forms. Configuration this dense belongs in a focused
// window rather than expanded inline under the card it edits: the surrounding
// page stops shifting, and the fields get room for real labels.
export default function FormDialog({
  title,
  subtitle,
  submitLabel,
  submitDisabled,
  onSubmit,
  onCancel,
  children,
}: {
  title: string;
  subtitle?: string;
  submitLabel: string;
  submitDisabled?: boolean;
  onSubmit: () => void;
  onCancel: () => void;
  children: ReactNode;
}) {
  useModalKeys(onCancel);
  return (
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); onCancel(); }}>
      <form
        className="modal modal-form"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => { e.preventDefault(); if (!submitDisabled) onSubmit(); }}
      >
        <div className="modal-head">
          <div className="modal-head-text">
            <h3>{title}</h3>
            {subtitle && <p className="modal-sub">{subtitle}</p>}
          </div>
          <button type="button" className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body modal-form-body">{children}</div>
        <div className="modal-foot">
          <button type="button" className="btn-secondary" onClick={onCancel}>Cancel</button>
          <button type="submit" className="btn-primary" disabled={submitDisabled}>{submitLabel}</button>
        </div>
      </form>
    </div>
  );
}
