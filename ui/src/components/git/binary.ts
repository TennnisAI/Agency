// Changes git won't show as text: images, other binary files, mode-only edits.
// The pure half of that story — which files get the picture view, and what to
// say when a diff has no lines — lives here so it can be tested on its own.

const RASTER_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif"];

/** Images the webview draws as a picture, so a change to one can be *seen*.
 *  SVG is deliberately absent: it is text, git diffs it line by line, and those
 *  lines are the more useful read. Keep in step with mime_for() in
 *  crates/agency-core/src/files.rs — a format with no MIME there won't render. */
export function isRasterImage(path: string): boolean {
  return RASTER_EXTS.includes((path.split(".").pop() ?? "").toLowerCase());
}

/** A diff with no hunks isn't always an unchanged file: git reports a binary
 *  file's change as a single "Binary files … differ" line, and a mode-only
 *  change as header lines alone. Say which it is rather than the flat "no
 *  textual changes", which reads as "nothing happened". */
export function emptyReason(header: string): string {
  if (/^Binary files .* differ$/m.test(header) || header.includes("GIT binary patch")) {
    return "Binary file changed; git has no line-by-line diff for it.";
  }
  if (/^(old|new) mode /m.test(header)) return "File permissions changed; the contents are the same.";
  return "no textual changes";
}

/** Human-readable byte size. Runs to GB because it also labels the files the
 *  repo setup dialog warns about, where "51200.0 MB" is the whole point. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
