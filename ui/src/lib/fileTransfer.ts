// Rules for moving and copying a path around inside one file tree — the
// cut/copy/paste menu and the drag. Pure, because the interesting cases are the
// refusals (a folder dropped into itself, a move that goes nowhere) and those
// are worth testing without a disk under them.
//
// The backend refuses the same things (`files.rs::is_within`); this is what
// lets the UI grey the menu item out and say why, instead of letting the user
// fire an operation that comes back as an error toast.

import { parentPath } from "./filePath";

/** What the tree is holding for a paste. */
export type Transfer = { mode: "copy" | "move"; path: string };

/**
 * True when the relative path `inner` is `outer` itself or sits underneath it.
 * Compared segment-wise, so "src2/x" is not inside "src".
 */
export function isWithin(outer: string, inner: string): boolean {
  const o = outer.replace(/^\/+|\/+$/g, "");
  const i = inner.replace(/^\/+|\/+$/g, "");
  // The tree root holds everything, itself included.
  if (o === "") return true;
  return i === o || i.startsWith(`${o}/`);
}

/**
 * Why `src` can't be transferred into the folder `dir`, or null when it can.
 * The message is shown to the user, so it names the thing they just tried.
 */
export function transferProblem(
  src: string,
  dir: string,
  mode: Transfer["mode"],
): string | null {
  if (isWithin(src, dir)) return "A folder can't go inside itself";
  // A copy into the same folder is a duplicate, which is a real thing to want;
  // a move into it is a no-op dressed up as an action.
  if (mode === "move" && parentPath(src) === dir) return "It's already there";
  return null;
}
