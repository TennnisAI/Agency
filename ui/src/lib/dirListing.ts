// Comparing two listings of the same directory. The file tree re-lists what it
// is showing on a timer so a change made in Finder turns up on its own, and an
// unchanged directory has to be recognised as unchanged: writing the new array
// into state every tick would re-render the whole tree twice a second and drop
// the row the pointer is on out from under it.

import { DirEntry } from "../api";

/** True when two listings of the same directory hold the same rows, in order. */
export function sameListing(a: DirEntry[], b: DirEntry[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((e, i) => {
    const o = b[i];
    // hasChildren is in the comparison because it draws the twistie: a folder
    // that just gained its first file has to grow one.
    return e.name === o.name && e.isDir === o.isDir && e.hasChildren === o.hasChildren;
  });
}

/**
 * The directories whose rows are actually on screen: the root, plus every
 * expanded folder reachable from it through the listings already loaded.
 *
 * Not simply `open`, which keeps a collapsed folder's expanded children so
 * re-opening it restores the shape it had. Those are invisible, and re-listing
 * them on every tick is work nobody can see the result of.
 */
export function visibleDirs(cache: Map<string, DirEntry[]>, open: Set<string>): string[] {
  const out = [""];
  const walk = (dir: string) => {
    for (const e of cache.get(dir) ?? []) {
      if (!e.isDir) continue;
      const path = dir ? `${dir}/${e.name}` : e.name;
      if (!open.has(path)) continue;
      out.push(path);
      walk(path);
    }
  };
  walk("");
  return out;
}
