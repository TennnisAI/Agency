// In-memory stash for unsaved editor buffers. FileEditor stashes its doc when
// it unmounts dirty (tab kept but root switched, etc.) and prefers the stash
// over the disk read on the next mount — this is what lets edits survive
// "switch roots and back" without keeping foreign-root editors alive. Session
// only, by design: unsaved edits never touch disk behind the user's back.

interface FileRootLike {
  kind: string;
  id: string;
}

const MAX_BUFFERS = 50;

const buffers = new Map<string, string>();

export const bufferKey = (root: FileRootLike, path: string): string =>
  `${root.kind}:${root.id}:${path}`;

/** Save an unsaved doc. Re-stashing an existing key makes it newest again. */
export function stashBuffer(key: string, text: string): void {
  buffers.delete(key); // Map iteration order = insertion order; refresh age
  buffers.set(key, text);
  if (buffers.size > MAX_BUFFERS) {
    const oldest = buffers.keys().next().value;
    if (oldest !== undefined) buffers.delete(oldest);
  }
}

/** Return-and-clear: a taken buffer belongs to the mounting editor. */
export function takeBuffer(key: string): string | null {
  const text = buffers.get(key);
  if (text === undefined) return null;
  buffers.delete(key);
  return text;
}

/** Peek without taking — restores dirty dots for tabs with no live editor. */
export function hasBuffer(key: string): boolean {
  return buffers.has(key);
}

/** Discard a stash (file saved, reverted, or its tab closed). */
export function dropBuffer(key: string): void {
  buffers.delete(key);
}

/** Test hook. */
export function clearBuffers(): void {
  buffers.clear();
}
