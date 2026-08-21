// Cross-component "open this file in the Files tab" routing (quick-open,
// find-in-files jumps from elsewhere). Same idea as the `agency:open-note`
// event, plus a pending slot for the mount race: the requester switches to the
// Files tab first, so FilesView may not be listening yet — it consumes the
// pending request on mount instead.
//
// Requests are scoped to a Files root ("kind:id"): FilesView only delivers a
// request whose root matches what it is showing, and a mismatched pending
// request is dropped on the next consume attempt. Without the scope, a request
// that never found its view (e.g. a tab redirect) would sit in the slot and
// fire later against a completely different project's root.

export interface OpenFileRequest {
  /** Key of the Files root (`fileRootKey`) the path is relative to. */
  rootKey: string;
  /** Relative to that root. */
  path: string;
  /** Optional 1-based line to land on. */
  line?: number;
  /**
   * Also show the path in the file tree: open the folders on the way to it and
   * scroll its row into view. Set by "Reveal in Files" and the diff header;
   * a quick-open jump leaves it off, because the tree is not what you were
   * looking at.
   *
   * A path ending in "/" is a folder (an untracked directory git reports as
   * one entry): it is revealed in the tree and nothing is opened in the editor.
   */
  reveal?: boolean;
}

/** The canonical "kind:id" key for a Files root. */
export function fileRootKey(root: { kind: string; id: string }): string {
  return `${root.kind}:${root.id}`;
}

const EVENT = "agency:open-file";

let pending: OpenFileRequest | null = null;

export function requestOpenFile(req: OpenFileRequest): void {
  pending = req;
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent<OpenFileRequest>(EVENT, { detail: req }));
  }
}

/**
 * Return-and-clear the undelivered request for `rootKey`, if any. A pending
 * request for a different root is stale (its view never mounted) and is
 * dropped rather than delivered to the wrong root.
 */
export function consumePendingOpen(rootKey: string): OpenFileRequest | null {
  const req = pending;
  pending = null;
  return req && req.rootKey === rootKey ? req : null;
}

/**
 * Live delivery while FilesView is mounted, filtered to its root. A request
 * for another root is left pending: the requester may have just switched
 * projects, so the right mount consumes it a render later.
 */
export function onOpenFile(rootKey: string, handler: (req: OpenFileRequest) => void): () => void {
  const fn = (e: Event) => {
    const req = (e as CustomEvent<OpenFileRequest>).detail;
    if (req.rootKey !== rootKey) return;
    pending = null; // delivered — nothing left for a later mount to consume
    handler(req);
  };
  window.addEventListener(EVENT, fn);
  return () => window.removeEventListener(EVENT, fn);
}
