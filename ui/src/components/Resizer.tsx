import { useCallback } from "react";

export default function Resizer({
  width,
  min,
  max,
  onChange,
  side = "left",
}: {
  width: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  side?: "left" | "right";
}) {
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startW = width;
      const move = (ev: PointerEvent) => {
        const delta = ev.clientX - startX;
        onChange(side === "left" ? startW + delta : startW - delta);
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        document.body.style.cursor = "";
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      document.body.style.cursor = "col-resize";
    },
    [width, min, max, onChange, side],
  );

  return (
    <div
      className="resizer"
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation="vertical"
    />
  );
}
