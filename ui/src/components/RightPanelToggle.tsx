// Right-side source-control panel show/hide control. Lives in the content
// header, mirroring the left SidebarToggle. The glyph is a source-control
// branch (two nodes joined to a third) so it reads as "source control panel",
// filled when the panel is open.
export default function RightPanelToggle({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      className={`icon-btn${open ? " on" : ""}`}
      title={open ? "Hide source control" : "Show source control"}
      onClick={onToggle}
    >
      <BranchIcon />
    </button>
  );
}

// Standard source-control branch glyph: a straight rail with a branch curving
// off it, three nodes. Monochrome stroke — no emoji (project rule).
function BranchIcon() {
  return (
    <svg
      width="17"
      height="17"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="6" cy="4" r="2" />
      <circle cx="6" cy="20" r="2" />
      <circle cx="18" cy="8" r="2" />
      <path d="M6 6 v12" />
      <path d="M18 10 a6 6 0 0 1 -6 6 H6" />
    </svg>
  );
}
