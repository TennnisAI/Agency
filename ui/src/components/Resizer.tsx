import { useCallback } from "react";

export function resizerClass(orientation: "vertical" | "horizontal"): string {
  return orientation === "horizontal" ? "resizer horizontal" : "resizer";
}

export function axisCoord(
  orientation: "vertical" | "horizontal",
  clientX: number,
  clientY: number,
): number {
  return orientation === "horizontal" ? clientY : clientX;
}

export function resizerCursor(orientation: "vertical" | "horizontal"): string {
  return orientation === "horizontal" ? "row-resize" : "col-resize";
}

export default function Resizer({
  size,
  min,
  max,
  onChange,
  side = "left",
  orientation = "vertical",
}: {
  // Current pane extent along the drag axis: width for vertical, height for horizontal.
  size: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  side?: "left" | "right";
  orientation?: "vertical" | "horizontal";
}) {
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const start = axisCoord(orientation, e.clientX, e.clientY);
      const startSize = size;
      const move = (ev: PointerEvent) => {
        const pos = axisCoord(orientation, ev.clientX, ev.clientY);
        const delta = pos - start;
        onChange(side === "left" ? startSize + delta : startSize - delta);
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        document.body.style.cursor = "";
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      document.body.style.cursor = resizerCursor(orientation);
    },
    [size, min, max, onChange, side, orientation],
  );

  return (
    <div
      className={resizerClass(orientation)}
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation={orientation}
    />
  );
}
