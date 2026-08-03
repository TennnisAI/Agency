// Inline SVG icons. Stroke uses currentColor so they inherit button text color
// (e.g. red on `.danger` buttons). Sized to sit inline next to button labels.

type IconProps = { size?: number };

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  className: "btn-ico",
  "aria-hidden": true,
});

export function TrashIcon({ size = 13 }: IconProps) {
  return (
    <svg {...base(size)}>
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
      <line x1="10" y1="11" x2="10" y2="17" />
      <line x1="14" y1="11" x2="14" y2="17" />
      <path d="M9 6V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
    </svg>
  );
}

export function TerminalIcon({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <rect x="2.5" y="4" width="19" height="16" rx="2" />
      <polyline points="6.5 9 9.5 12 6.5 15" />
      <line x1="12" y1="15" x2="16" y2="15" />
    </svg>
  );
}

export function PencilIcon({ size = 13 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  );
}

export function InboxIcon({ size = 13 }: IconProps) {
  return (
    <svg {...base(size)}>
      <polyline points="22 12 16 12 14 15 10 15 8 12 2 12" />
      <path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
    </svg>
  );
}

export function CheckIcon({ size = 13 }: IconProps) {
  return (
    <svg {...base(size)} strokeWidth={2.6}>
      <polyline points="4 12.5 9.5 18 20 6.5" />
    </svg>
  );
}

// Horizontal "…" affordance for an overflow menu. Dots are filled, not stroked,
// so they stay round at 14px instead of collapsing into dashes.
export function MoreIcon({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)} fill="currentColor" stroke="none">
      <circle cx="5" cy="12" r="1.85" />
      <circle cx="12" cy="12" r="1.85" />
      <circle cx="19" cy="12" r="1.85" />
    </svg>
  );
}

export function BranchIcon({ size = 12 }: IconProps) {
  return (
    <svg {...base(size)}>
      <circle cx="6" cy="5" r="2.5" />
      <circle cx="6" cy="19" r="2.5" />
      <circle cx="18" cy="8" r="2.5" />
      <path d="M18 10.5v1A3.5 3.5 0 0 1 14.5 15H6" />
      <path d="M6 7.5v9" />
    </svg>
  );
}

// Expand / contract for a pane that grows to take over its view. Diagonal
// arrows out of the corners when there is room to grow, back into them when
// the pane is already wide.
export function ExpandIcon({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M14 4h6v6" />
      <path d="M20 4l-7 7" />
      <path d="M10 20H4v-6" />
      <path d="M4 20l7-7" />
    </svg>
  );
}

export function ContractIcon({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M20 10h-6V4" />
      <path d="M14 10l7-7" />
      <path d="M4 14h6v6" />
      <path d="M10 14l-7 7" />
    </svg>
  );
}
