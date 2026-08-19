import { DocsIndex } from "./docsIndex";
import { joinPath, parentPath } from "./filePath";

/** A folder level in the Docs tree. `path` is rel to the docs dir ("" = root). */
export interface TreeDir {
  path: string;
  dirs: Map<string, TreeDir>;
  notes: { path: string; title: string }[];
}

/**
 * The Docs tree's shape: a folder per path segment of every note, plus the
 * index's empty folders.
 *
 * The empty ones have to be handed in because nothing else implies them — a
 * folder with no markdown under it is invisible to a markdown walk, so a
 * folder the user just made disappeared from the tree the moment the view
 * rebuilt (AGE-119). Folders holding only non-markdown files stay hidden, as
 * they always were: the Files tab is where those live.
 */
export function buildDocsTree(index: DocsIndex | null): TreeDir {
  const root: TreeDir = { path: "", dirs: new Map(), notes: [] };
  const dirAt = (dir: string): TreeDir => {
    if (dir === "") return root;
    let cur = root;
    let acc = "";
    for (const part of dir.split("/")) {
      acc = joinPath(acc, part);
      let next = cur.dirs.get(part);
      if (!next) {
        next = { path: acc, dirs: new Map(), notes: [] };
        cur.dirs.set(part, next);
      }
      cur = next;
    }
    return cur;
  };
  if (index) {
    for (const d of index.docs.values()) {
      dirAt(parentPath(d.path)).notes.push({ path: d.path, title: d.title });
    }
    for (const dir of index.emptyDirs) dirAt(dir);
  }
  const sortDir = (d: TreeDir) => {
    // The journal reads newest-first (date-stamped names, so name order is
    // date order); everything else alphabetical.
    if (d.path === "journal") {
      d.notes.sort((a, b) => b.path.localeCompare(a.path));
    } else {
      d.notes.sort((a, b) => a.title.localeCompare(b.title));
    }
    for (const sub of d.dirs.values()) sortDir(sub);
  };
  sortDir(root);
  return root;
}

/** Every folder path in the tree, so the toolbar can open them all at once. */
export function allDirPaths(tree: TreeDir): string[] {
  const out: string[] = [];
  const walk = (d: TreeDir) => {
    for (const sub of d.dirs.values()) {
      out.push(sub.path);
      walk(sub);
    }
  };
  walk(tree);
  return out;
}
