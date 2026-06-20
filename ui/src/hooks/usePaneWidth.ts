import { useCallback, useState } from "react";

export const clampWidth = (n: number, min: number, max: number): number =>
  Math.min(max, Math.max(min, n));

export function loadWidth(
  storage: Pick<Storage, "getItem">,
  key: string,
  def: number,
  min: number,
  max: number,
): number {
  const raw = storage.getItem("pane:" + key);
  const parsed = raw !== null ? Number(raw) : NaN;
  return Number.isFinite(parsed) ? clampWidth(parsed, min, max) : def;
}

export function saveWidth(
  storage: Pick<Storage, "setItem">,
  key: string,
  w: number,
  min: number,
  max: number,
): number {
  const clamped = clampWidth(w, min, max);
  storage.setItem("pane:" + key, String(clamped));
  return clamped;
}

export function usePaneWidth(key: string, def: number, min: number, max: number) {
  const [width, setWidthState] = useState<number>(() => {
    if (typeof localStorage === "undefined") return def;
    return loadWidth(localStorage, key, def, min, max);
  });

  const setWidth = useCallback(
    (n: number) => {
      const w = clampWidth(n, min, max);
      setWidthState(w);
      try {
        if (typeof localStorage !== "undefined") {
          saveWidth(localStorage, key, w, min, max);
        }
      } catch {
        // ignore quota / security errors
      }
    },
    [key, min, max],
  );

  return { width, setWidth };
}
