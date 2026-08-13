// Naming rules for files dragged in from the OS file manager. Pure so the
// awkward parts (collisions, extensions, path separators) are testable without
// a webview or a disk.

/** Last segment of an OS path, whichever separator the platform uses. */
export function dropName(srcPath: string): string {
  const i = Math.max(srcPath.lastIndexOf("/"), srcPath.lastIndexOf("\\"));
  return i === -1 ? srcPath : srcPath.slice(i + 1);
}

/**
 * Split a file name into its stem and extension. A leading dot belongs to the
 * stem, so ".gitignore" stays whole rather than becoming an extension.
 */
export function splitExt(name: string): [stem: string, ext: string] {
  const i = name.lastIndexOf(".");
  return i <= 0 ? [name, ""] : [name.slice(0, i), name.slice(i)];
}

export const isMarkdown = (name: string) => /\.(md|markdown)$/i.test(name);

/**
 * A name that doesn't collide with anything in `taken`, by appending " 2",
 * " 3"… before the extension (the same shape Finder uses for a copy). The
 * comparison is case-insensitive because macOS filesystems are: importing
 * "Notes.md" next to "notes.md" would be refused, not accepted as a second file.
 */
export function uniqueName(taken: Set<string>, name: string): string {
  const has = (n: string) => taken.has(n.toLowerCase());
  if (!has(name)) return name;
  const [stem, ext] = splitExt(name);
  for (let n = 2; ; n++) {
    const candidate = `${stem} ${n}${ext}`;
    if (!has(candidate)) return candidate;
  }
}

/** Fold a list of names into one clause, naming a few and counting the rest. */
export function nameList(names: string[], max = 3): string {
  if (names.length <= max) return names.join(", ");
  return `${names.slice(0, max).join(", ")} and ${names.length - max} more`;
}
