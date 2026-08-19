// Linking a note to a file that sits beside it in the docs tree. Pure string
// work so the awkward parts (climbing out of a subfolder, names markdown can't
// hold raw) are testable without a webview or a disk.
//
// Note-relative, not docs-root-relative: it is what standard markdown means by
// a relative link, so the same text resolves in the editor's live preview, in
// VS Code, and in anything else pointed at the folder. The live preview does
// accept a docs-root path as a fallback (that is what image paste writes), but
// only the note-relative form travels.

import { isImageName } from "./attachments";
import { baseName, parentPath } from "./filePath";

/**
 * Path from the note at `notePath` to the file at `target`, both relative to
 * the docs dir. Shared leading folders drop off, what's left of the note's own
 * folder becomes `..` segments: `guides/setup.md` → `assets/a.png` is
 * `../assets/a.png`, and a sibling is just its name.
 */
export function relativeFrom(notePath: string, target: string): string {
  const from = parentPath(notePath);
  const fromParts = from === "" ? [] : from.split("/");
  const toParts = target.split("/");
  let i = 0;
  while (i < fromParts.length && i < toParts.length - 1 && fromParts[i] === toParts[i]) i++;
  const up = fromParts.length - i;
  return [...Array(up).fill(".."), ...toParts.slice(i)].join("/");
}

/**
 * The markdown a link to `target` is written as, from inside `notePath`:
 * images embed, everything else is a plain link labelled with its filename.
 *
 * A path holding a space or a bracket goes in angle brackets rather than being
 * percent-encoded — CommonMark's own escape hatch, and it leaves the path
 * readable in the raw file, which is the point of keeping notes as markdown.
 */
export function attachmentLink(notePath: string, target: string): string {
  const rel = relativeFrom(notePath, target);
  const href = /[\s()<>]/.test(rel) ? `<${rel}>` : rel;
  const name = baseName(target);
  return isImageName(name) ? `![](${href})` : `[${name}](${href})`;
}
