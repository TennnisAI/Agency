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

export function FileIcon({
  kind,
  size = 13,
  open = false,
}: {
  kind: FileIconKind;
  size?: number;
  /** For the `folder` kind: render the open (expanded) variant. */
  open?: boolean;
}) {
  const p = base(size);
  switch (kind) {
    case "folder":
      return open ? (
        <svg {...p}>
          {/* Open folder: back flap plus an angled front lid. */}
          <path d="M3 8V6a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v1" />
          <path d="M3 8h17l-2 9a2 2 0 0 1-2 1.6H4.5A1.5 1.5 0 0 1 3 17z" />
        </svg>
      ) : (
        <svg {...p}>
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
        </svg>
      );
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
    case "audio":
      return (
        <svg {...p}>
          {/* Musical note: two stems joined by a beam, with note heads. */}
          <path d="M9 18V6l10-2v10" />
          <circle cx="6.5" cy="18" r="2.5" />
          <circle cx="16.5" cy="16" r="2.5" />
        </svg>
      );
    case "video":
      return (
        <svg {...p}>
          {/* Film frame with a centered play triangle. */}
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="M10 9l5 3-5 3z" fill="currentColor" stroke="none" />
        </svg>
      );
    case "pdf":
      return (
        <svg {...p}>
          {sheet}
          <path d="M8.5 16.5v-3h1.2a1 1 0 0 1 0 2H8.5" />
          <path d="M13 13.5v3h.8a1.3 1.3 0 0 0 0-3z" />
        </svg>
      );
    case "archive":
      return (
        <svg {...p}>
          {sheet}
          {/* Zipper pull-tab down the center. */}
          <line x1="12" y1="3" x2="12" y2="8" />
          <line x1="12" y1="10" x2="12" y2="11.5" />
          <rect x="10.6" y="13" width="2.8" height="3.2" rx="0.6" />
        </svg>
      );
    case "font":
      return (
        <svg {...p}>
          {/* Serif "A" glyph. */}
          <path d="M6 19l6-14 6 14" />
          <line x1="8.5" y1="13" x2="15.5" y2="13" />
        </svg>
      );
    case "binary":
      return (
        <svg {...p}>
          {/* Microchip: body plus pins on all four sides. */}
          <rect x="7" y="7" width="10" height="10" rx="1.5" />
          <path d="M10 4v3M14 4v3M10 17v3M14 17v3M4 10h3M4 14h3M17 10h3M17 14h3" />
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
