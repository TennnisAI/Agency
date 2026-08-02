// Issue attachments. Files dropped, pasted, or picked into an issue are copied
// into `.agency/issues/assets/` — beside the issue files — and referenced from
// the issue body as a *relative* markdown link:
// `![](assets/AGE-13-20260802-134501.png)`.
//
// Relative to the issue file's own directory is the whole trick: the same link
// resolves in the app, in VS Code, and in any markdown editor, without knowing
// where the repo sits on disk.
//
// Issue files and their assets are local to a checkout, not tracked by git
// (`issuefs::ISSUES_DIR`, `worktree::untrack_issue_files`), so attachments do
// not travel with an agent branch and these links do not resolve on GitHub.
// Both would change if the backlog ever becomes shareable — see
// docs/tracked-issues.md.
//
// Everything here is pure string work so it can be tested without a webview;
// the IO lives in IssueDetail.

/** Attachments dir, relative to an issue file (which sits in `.agency/issues/`). */
export const ASSETS_DIR = "assets";

/** Repo-relative issues dir — mirrors `issuefs::ISSUES_DIR` on the Rust side. */
export const ISSUES_DIR = ".agency/issues";

/** Extensions the UI renders inline; the rest become file chips. */
const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif", "svg", "ico"]);

/** Clipboard images arrive as a MIME type with no filename. */
const MIME_EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/bmp": "bmp",
  "image/avif": "avif",
  "image/svg+xml": "svg",
};

export interface Attachment {
  /** As written in the body: `assets/AGE-13-20260802-134501.png`. */
  ref: string;
  /** Filename alone, for display. */
  name: string;
  /** Repo-relative path the file API wants: `.agency/issues/assets/…`. */
  repoPath: string;
  /** The link's text — alt text for images, label for files. */
  alt: string;
  /** Written as an embed (`![…]`) rather than a plain link. */
  embedded: boolean;
  /** Renderable as an image (by extension). */
  isImage: boolean;
  /** Bounds of the whole markdown link in the body, for removal. */
  from: number;
  to: number;
}

export function isImageName(name: string): boolean {
  return IMAGE_EXTS.has(splitExt(name).ext);
}

export function extForMime(mime: string): string {
  return MIME_EXT[mime] ?? "png";
}

/** `"shot.tar.gz"` → `{ base: "shot.tar", ext: "gz" }`; no dot → empty ext. */
export function splitExt(name: string): { base: string; ext: string } {
  const i = name.lastIndexOf(".");
  // A leading dot is part of the name (".gitignore"), not an extension.
  if (i <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, i), ext: name.slice(i + 1).toLowerCase() };
}

/** Last path segment, for either separator — drop payloads carry full paths. */
export function baseName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i < 0 ? path : path.slice(i + 1);
}

/**
 * Filesystem- and URL-safe slug: lowercase, runs of anything else collapsed to
 * a single dash. Keeps attachment links free of percent-encoding, so the raw
 * markdown stays readable and every viewer resolves it the same way.
 *
 * May return "" (a name with nothing ASCII in it) — callers pick the fallback,
 * so a nameless file gets something traceable rather than a shared "file".
 */
export function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/g, "");
}

/** `20260802-134501` — local time, matching the docs paste convention. */
export function stampFor(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}` +
    `-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`
  );
}

/**
 * Name an attachment file. Issue-keyed so a flat assets dir stays legible and
 * an orphan is traceable to the issue that made it.
 *
 * A dropped file keeps its (slugified) name — `AGE-13-login-crash.png` beats a
 * timestamp nobody can read. A pasted image has no name, so it gets the stamp.
 * `attempt` > 1 appends a counter for the no-clobber write's retry.
 */
export function attachmentName(
  label: string,
  original: string | null,
  stamp: string,
  attempt = 1,
): string {
  const suffix = attempt > 1 ? `-${attempt}` : "";
  const key = slugify(label) || "issue";
  if (!original) return `${key}-${stamp}${suffix}.png`;
  const { base, ext } = splitExt(baseName(original));
  const name = `${key}-${slugify(base) || stamp}${suffix}`;
  return ext ? `${name}.${ext}` : name;
}

/** Same, but for a pasted blob that only knows its MIME type. */
export function pastedName(label: string, mime: string, stamp: string, attempt = 1): string {
  const suffix = attempt > 1 ? `-${attempt}` : "";
  return `${slugify(label) || "issue"}-${stamp}${suffix}.${extForMime(mime)}`;
}

/** The markdown one attachment is written as: images embed, everything links. */
export function attachmentMarkdown(name: string, alt = ""): string {
  const ref = `${ASSETS_DIR}/${name}`;
  return isImageName(name) ? `![${alt}](${ref})` : `[${alt || name}](${ref})`;
}

// Markdown links whose target sits under `assets/`. Angle-bracket targets
// (`[x](<assets/a b.png>)`) are accepted on read even though we never write
// them — hand-edited files and other editors do.
const LINK_RE = /(!?)\[([^\]]*)\]\(\s*(?:<([^>]+)>|([^)\s]+))\s*\)/g;

/** Every attachment the body references, in document order. */
export function parseAttachments(body: string): Attachment[] {
  const out: Attachment[] = [];
  const seen = new Set<string>();
  for (const m of body.matchAll(LINK_RE)) {
    const raw = m[3] ?? m[4] ?? "";
    // Percent-encoding is what other editors write for spaces; decode so the
    // path we hand the file API is the real one on disk.
    let ref: string;
    try {
      ref = decodeURI(raw);
    } catch {
      ref = raw;
    }
    if (!ref.startsWith(`${ASSETS_DIR}/`) || ref.includes("..")) continue;
    const name = baseName(ref);
    if (!name) continue;
    // The same file linked twice is one attachment; the first mention wins so
    // removal targets a stable span.
    if (seen.has(ref)) continue;
    seen.add(ref);
    out.push({
      ref,
      name,
      repoPath: `${ISSUES_DIR}/${ref}`,
      alt: m[2],
      embedded: m[1] === "!",
      isImage: isImageName(name),
      from: m.index,
      to: m.index + m[0].length,
    });
  }
  return out;
}

/**
 * Splice `insert` into `body` at `[from, to)`, padding so the result reads as
 * markdown rather than running into the previous word. Returns the new body
 * and where the caret should land.
 */
export function insertAttachment(
  body: string,
  from: number,
  to: number,
  insert: string,
): { body: string; cursor: number } {
  const before = body.slice(0, from);
  const after = body.slice(to);
  // An embed wants its own line; a space is enough for anything inline.
  const pad = before === "" || before.endsWith("\n") ? "" : before.endsWith(" ") ? "" : " ";
  const text = `${pad}${insert}`;
  return { body: before + text + after, cursor: from + text.length };
}

/**
 * Drop an attachment's link out of the body. Also takes the line with it when
 * the link was all that was on it, so removing an image doesn't leave a blank
 * line behind every time.
 */
export function removeAttachment(body: string, a: Attachment): string {
  let from = a.from;
  let to = a.to;
  const lineStart = body.lastIndexOf("\n", from - 1) + 1;
  const nlEnd = body.indexOf("\n", to);
  const lineEnd = nlEnd === -1 ? body.length : nlEnd;
  const wholeLine =
    body.slice(lineStart, from).trim() === "" && body.slice(to, lineEnd).trim() === "";
  if (wholeLine) {
    from = lineStart;
    // Swallow the newline too, unless this was the last line.
    to = nlEnd === -1 ? lineEnd : nlEnd + 1;
  }
  const before = body.slice(0, from);
  let after = body.slice(to);
  if (wholeLine) {
    // The blank lines that flanked the removed line are now adjacent; collapse
    // the seam back to a single paragraph break instead of stacking them.
    const nlBefore = /\n*$/.exec(before)![0].length;
    const nlAfter = /^\n*/.exec(after)![0].length;
    if (nlBefore + nlAfter > 2) {
      after = after.slice(Math.min(nlBefore + nlAfter - 2, nlAfter));
    }
  }
  return before + after;
}
