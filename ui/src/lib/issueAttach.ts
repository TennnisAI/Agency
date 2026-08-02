// The IO half of issue attachments: get bytes (or a path) onto disk under
// `.agency/issues/assets/` and hand back the markdown that references them.
// The naming and parsing rules live in ./attachments, which stays pure.

import { FileRoot, createDir, importFile, writeFileBase64 } from "../api";
import {
  ASSETS_DIR,
  ISSUES_DIR,
  attachmentMarkdown,
  attachmentName,
  pastedName,
  stampFor,
} from "./attachments";

/** Retries for the no-clobber write: burst pastes land in the same second. */
const MAX_ATTEMPTS = 5;

const ASSETS_PATH = `${ISSUES_DIR}/${ASSETS_DIR}`;

/**
 * Make sure the assets dir exists. Each level is created separately and its
 * "already exists" failure ignored, so this works whether the tracker has been
 * written before or this is the project's very first attachment.
 */
async function ensureAssetsDir(root: FileRoot): Promise<void> {
  for (const dir of [".agency", ISSUES_DIR, ASSETS_PATH]) {
    await createDir(root, dir).catch(() => { /* already there */ });
  }
}

/** Chunked base64: spreading a multi-MB array into fromCharCode overflows the stack. */
function toBase64(bytes: Uint8Array): string {
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

/**
 * Try `write` under successively suffixed names until one sticks. The backend
 * refuses to overwrite, so a rejection is normally a name collision — but the
 * last attempt's error propagates, and a genuine failure (no disk, too large)
 * surfaces rather than being retried into silence.
 */
async function writeUnique(
  name: (attempt: number) => string,
  write: (relPath: string) => Promise<void>,
): Promise<string> {
  for (let attempt = 1; ; attempt++) {
    const candidate = name(attempt);
    try {
      await write(`${ASSETS_PATH}/${candidate}`);
      return candidate;
    } catch (e) {
      if (attempt >= MAX_ATTEMPTS) throw e;
    }
  }
}

/** One attachment: the file it landed as, and the markdown to reference it. */
export interface Attached {
  name: string;
  markdown: string;
}

/**
 * Attach clipboard/drop bytes (a `File` from a paste, which carries no usable
 * path). `label` is the issue key, `now` the clock — injected so callers in
 * tests aren't at the mercy of the wall clock.
 */
export async function attachBlob(
  root: FileRoot,
  label: string,
  file: File,
  now: Date = new Date(),
): Promise<Attached> {
  await ensureAssetsDir(root);
  const b64 = toBase64(new Uint8Array(await file.arrayBuffer()));
  const stamp = stampFor(now);
  // A pasted screenshot has no name (or a useless generic one); a dragged-in
  // file object does, and keeping it makes the assets dir legible.
  const named = file.name && file.name !== "image.png";
  const name = await writeUnique(
    (attempt) =>
      named
        ? attachmentName(label, file.name, stamp, attempt)
        : pastedName(label, file.type, stamp, attempt),
    (relPath) => writeFileBase64(root, relPath, b64),
  );
  return { name, markdown: attachmentMarkdown(name) };
}

/** Attach a file already on disk, by absolute path (Finder drop, file picker). */
export async function attachPath(
  root: FileRoot,
  label: string,
  srcPath: string,
  now: Date = new Date(),
): Promise<Attached> {
  await ensureAssetsDir(root);
  const stamp = stampFor(now);
  const name = await writeUnique(
    (attempt) => attachmentName(label, srcPath, stamp, attempt),
    (relPath) => importFile(root, srcPath, relPath),
  );
  return { name, markdown: attachmentMarkdown(name) };
}
