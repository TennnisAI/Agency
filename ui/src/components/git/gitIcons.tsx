// Small stroked SVG glyphs for the git panel (branch/remote/tag pills, menus).
// currentColor throughout, matching the icons.tsx / fileIcons.tsx convention.

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.4,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
});

/** Fork glyph: a lane splitting off a trunk (VS Code's source-control shape). */
export function BranchIcon({ size = 11 }: { size?: number }) {
  const p = base(size);
  return (
    <svg {...p}>
      <circle cx="4.5" cy="3.5" r="1.8" />
      <circle cx="4.5" cy="12.5" r="1.8" />
      <circle cx="11.5" cy="5" r="1.8" />
      <path d="M4.5 5.3v5.4" />
      <path d="M11.5 6.8c0 2.2-2.6 3-4.8 3.4" />
    </svg>
  );
}

/** Cloud glyph for remote-tracking refs. */
export function CloudIcon({ size = 11 }: { size?: number }) {
  const p = base(size);
  return (
    <svg {...p}>
      <path d="M4.5 12.5h7a3 3 0 0 0 .5-5.96 4 4 0 0 0-7.83-.86A3.2 3.2 0 0 0 4.5 12.5z" />
    </svg>
  );
}

/** Tag glyph. */
export function TagIcon({ size = 11 }: { size?: number }) {
  const p = base(size);
  return (
    <svg {...p}>
      <path d="M8.2 2.5H13.5v5.3l-6 6-5.3-5.3z" />
      <circle cx="10.7" cy="5.3" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  );
}

/** Layered-boxes glyph for stashes. */
export function StashIcon({ size = 12 }: { size?: number }) {
  const p = base(size);
  return (
    <svg {...p}>
      <path d="M2.5 8 8 10.8 13.5 8" />
      <path d="M2.5 11 8 13.8 13.5 11" />
      <path d="M2.5 5.2 8 2.5l5.5 2.7L8 8z" />
    </svg>
  );
}
