import { ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * The dimmed sheet every dialog sits on.
 *
 * Portalled into the body rather than left where it is written. A dialog is
 * rendered from wherever the thing it acts on is listed, and the sheet is
 * `position: fixed`, so any positioned or transformed ancestor between here and
 * the viewport becomes its containing block. An agent's archive/discard confirm
 * is rendered inside the rail row whose ✕ menu opened it, and `.rail-row-wrap`
 * is `position: relative`: the sheet took that ~30px row as its box, so the
 * dialog came out 92% of a rail row wide and centred on a sliver, clipped by
 * the rail's scroller. The "Archive agent?" title was cut off the top entirely
 * and the warning read as a wall of two-word lines (AGE-113).
 *
 * React events still bubble along the component tree, so a host that stops a
 * backdrop click from reaching its own onClick (an agent tile, a rail row) goes
 * on seeing it.
 */
export default function ModalBackdrop({
  onBackdropClick,
  children,
}: {
  /** Dismiss on a click outside the dialog. Omit to make the sheet inert, as a busy dialog does. */
  onBackdropClick?: () => void;
  children: ReactNode;
}) {
  return createPortal(
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); onBackdropClick?.(); }}>
      {children}
    </div>,
    document.body,
  );
}
