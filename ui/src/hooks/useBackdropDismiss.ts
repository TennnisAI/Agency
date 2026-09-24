import { MouseEvent, useRef } from "react";

/**
 * Whether a click on a dialog's dimmed sheet should dismiss the dialog: only
 * when the press and the release both landed on the sheet itself.
 *
 * A `click` is fired on the nearest common ancestor of the mousedown and the
 * mouseup targets. Select a rename field's text by dragging past the edge of
 * the dialog and the button comes up over the sheet, so the click lands on the
 * sheet, not the dialog, and the dialog's own `stopPropagation` never sees it:
 * the rename dialog closed mid-selection and threw the edit away (AGE-244).
 * Pressing on the sheet and releasing inside the dialog is the same click the
 * other way round, and is not a dismissal either.
 */
export function backdropDismisses(
  pressed: EventTarget | null,
  released: EventTarget | null,
  sheet: EventTarget,
): boolean {
  return pressed === sheet && released === sheet;
}

/**
 * The mouse handlers for a dismiss-on-click-outside sheet. Spread them onto the
 * sheet element; `onDismiss` runs only for a click that `backdropDismisses`
 * accepts. Omit it to make the sheet inert.
 */
export function useBackdropDismiss(onDismiss?: () => void) {
  const pressed = useRef<EventTarget | null>(null);
  const released = useRef<EventTarget | null>(null);
  return {
    onMouseDown: (e: MouseEvent) => { pressed.current = e.target; },
    onMouseUp: (e: MouseEvent) => { released.current = e.target; },
    onClick: (e: MouseEvent) => {
      const dismiss = backdropDismisses(pressed.current, released.current, e.currentTarget);
      pressed.current = released.current = null;
      if (dismiss) onDismiss?.();
    },
  };
}
