/** Join a child name onto a worktree-relative parent path. The root is "". */
export function joinPath(parent: string, name: string): string {
  return parent ? `${parent}/${name}` : name;
}

/** Parent directory of a relative path ("" for a top-level entry). */
export function parentPath(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? "" : path.slice(0, i);
}

/** Final path segment (the file/dir name). */
export function baseName(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? path : path.slice(i + 1);
}

/**
 * Every directory between the root and `path`, outermost first:
 * "a/b/c.ts" → ["a", "a/b"]. The root ("") is never included — it is always
 * open — and the leaf itself is left off, so a caller revealing a folder has
 * to append it.
 */
export function ancestorDirs(path: string): string[] {
  const parts = path.split("/").filter(Boolean);
  parts.pop(); // the leaf; only what contains it is a folder to open
  const dirs: string[] = [];
  let cur = "";
  for (const p of parts) {
    cur = cur ? `${cur}/${p}` : p;
    dirs.push(cur);
  }
  return dirs;
}
