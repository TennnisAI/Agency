import { PrConflicts } from "../../api";

// How many conflicting files the banner names before it stops listing. Past a
// handful the list stops being a summary and starts being the diff.
export const CONFLICT_LIST_CAP = 6;

// The line under the banner's headline. The probe has four outcomes and they
// read as sentences rather than as a nest of ternaries in the markup.
export function conflictNote(c: PrConflicts | null): string {
  if (!c) return "Checking which files collide…";
  const shown = c.files.slice(0, CONFLICT_LIST_CAP).join(", ");
  const rest = c.files.length - CONFLICT_LIST_CAP;
  if (c.files.length === 1) return `One file collides: ${shown}`;
  if (c.files.length > 1) {
    return rest > 0
      ? `${c.files.length} files collide: ${shown}, and ${rest} more`
      : `${c.files.length} files collide: ${shown}`;
  }
  // Probed and clean, yet GitHub says otherwise: usually a resolution that has
  // been pushed but not yet re-checked upstream.
  return c.probed
    ? "GitHub reports a conflict that no longer shows up locally. Try again in a moment."
    : "Agency couldn't work out which files collide from here.";
}

