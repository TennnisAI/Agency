import { useModalKeys } from "../../hooks/useModalKeys";
import { toastInfo } from "../../lib/toast";
import ModalBackdrop from "../ModalBackdrop";

// A dedicated, scrollable output view for git command output — chiefly push
// errors, which are often many lines long (rejected pushes, protected-branch
// rules, permission failures). Kept out of the inline banner so a wall of text
// can't blow out the panel layout; here it scrolls inside a fixed-size box.
export default function GitOutputModal({
  title,
  text,
  onClose,
}: {
  title: string;
  text: string;
  onClose: () => void;
}) {
  useModalKeys(onClose, true);
  const copy = () => {
    navigator.clipboard.writeText(text).then(() => toastInfo("Output copied")).catch(() => {});
  };
  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div className="modal git-output-modal" role="dialog" aria-modal="true" aria-label={title}
        onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <pre className="git-output-text">{text}</pre>
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={copy}>Copy</button>
          <button className="btn-primary" autoFocus onClick={onClose}>Close</button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
