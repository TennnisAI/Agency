// File-type icons for the file tree. Stroked SVG in currentColor (the caller
// sets color from the theme palette — see lib/fileIcon.ts), matching the icon
// convention in icons.tsx. One <FileIcon> dispatches on the mapped kind.

import type { FileIconKind } from "../lib/fileIcon";

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.9,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
});

// A sheet-of-paper outline shared by the document-ish kinds.
const sheet = (
  <>
    <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
    <polyline points="14 3 14 8 19 8" />
  </>
);

export function FileIcon({ kind, size = 13 }: { kind: FileIconKind; size?: number }) {
  const p = base(size);
  switch (kind) {
    case "code":
      return (
        <svg {...p}>
          <polyline points="8 8 4 12 8 16" />
          <polyline points="16 8 20 12 16 16" />
        </svg>
      );
    case "markup":
      return (
        <svg {...p}>
          <polyline points="7 8 3 12 7 16" />
          <polyline points="17 8 21 12 17 16" />
          <line x1="14" y1="5" x2="10" y2="19" />
        </svg>
      );
    case "braces":
      return (
        <svg {...p}>
          <path d="M8 4a3 3 0 0 0-3 3v2a2 2 0 0 1-2 2 2 2 0 0 1 2 2v2a3 3 0 0 0 3 3" />
          <path d="M16 4a3 3 0 0 1 3 3v2a2 2 0 0 0 2 2 2 2 0 0 0-2 2v2a3 3 0 0 1-3 3" />
        </svg>
      );
    case "style":
      return (
        <svg {...p}>
          <line x1="10" y1="4" x2="8" y2="20" />
          <line x1="16" y1="4" x2="14" y2="20" />
          <line x1="4" y1="9" x2="20" y2="9" />
          <line x1="4" y1="15" x2="20" y2="15" />
        </svg>
      );
    case "doc":
      return (
        <svg {...p}>
          {sheet}
          <line x1="9" y1="13" x2="15" y2="13" />
          <line x1="9" y1="17" x2="15" y2="17" />
        </svg>
      );
    case "image":
      return (
        <svg {...p}>
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <circle cx="8.5" cy="9.5" r="1.5" />
          <path d="M21 16l-5-5-8 8" />
        </svg>
      );
    case "lock":
      return (
        <svg {...p}>
          <rect x="5" y="11" width="14" height="9" rx="2" />
          <path d="M8 11V7a4 4 0 0 1 8 0v4" />
        </svg>
      );
    case "config":
      return (
        <svg {...p}>
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v3M12 19v3M4.5 4.5l2 2M17.5 17.5l2 2M2 12h3M19 12h3M4.5 19.5l2-2M17.5 6.5l2-2" />
        </svg>
      );
    case "file":
    default:
      return <svg {...p}>{sheet}</svg>;
  }
}
