import { useCallback } from "react";

export default function Resizer({
  width,
  min,
  max,
  onChange,
  side = "left",
  orientation = "vertical",
}: {
  width: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  side?: "left" | "right";
  orientation?: "vertical" | "horizontal";
}) {
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const horizontal = orientation === "horizontal";
      const start = horizontal ? e.clientY : e.clientX;
      const startW = width;
      const move = (ev: PointerEvent) => {
        const pos = horizontal ? ev.clientY : ev.clientX;
        const delta = pos - start;
        onChange(side === "left" ? startW + delta : startW - delta);
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        document.body.style.cursor = "";
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      document.body.style.cursor = horizontal ? "row-resize" : "col-resize";
    },
    [width, min, max, onChange, side, orientation],
  );

  return (
    <div
      className={`resizer${orientation === "horizontal" ? " horizontal" : ""}`}
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation={orientation}
    />
  );
}
