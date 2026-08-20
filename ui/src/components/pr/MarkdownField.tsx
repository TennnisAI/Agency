import { useState } from "react";
import Markdown from "../Markdown";

// A markdown text box with a preview tab: the same render GitHub will produce,
// seen before the text is saved rather than after. Used wherever the review
// pane writes markdown back to GitHub, which is the PR description and your own
// review comments.
//
// The preview reuses `Markdown`, so what you see here is exactly what the
// thread and description render once the save lands.
export default function MarkdownField({
  value,
  onChange,
  placeholder,
  autoFocus,
  minHeight,
  onSubmit,
  onCancel,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  autoFocus?: boolean;
  minHeight?: number;
  // Cmd/Ctrl+Enter in the text box.
  onSubmit?: () => void;
  // Escape in the text box.
  onCancel?: () => void;
}) {
  const [previewing, setPreviewing] = useState(false);

  return (
    <div className="md-field">
      <div className="md-field-tabs">
        <button
          type="button"
          className={`md-field-tab${previewing ? "" : " on"}`}
          onClick={() => setPreviewing(false)}
        >
          Write
        </button>
        <button
          type="button"
          className={`md-field-tab${previewing ? " on" : ""}`}
          onClick={() => setPreviewing(true)}
        >
          Preview
        </button>
      </div>
      {previewing ? (
        <div className="md-field-preview" style={{ minHeight }}>
          {value.trim() ? (
            <Markdown text={value} />
          ) : (
            <span className="md-field-blank">Nothing to preview yet.</span>
          )}
        </div>
      ) : (
        <textarea
          className="settings-input md-field-input"
          style={{ minHeight }}
          value={value}
          placeholder={placeholder}
          autoFocus={autoFocus}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              onSubmit?.();
            } else if (e.key === "Escape") {
              // Stop here: the same key closes the whole review pane further up.
              e.stopPropagation();
              onCancel?.();
            }
          }}
        />
      )}
    </div>
  );
}
