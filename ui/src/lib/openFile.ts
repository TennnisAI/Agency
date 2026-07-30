// Cross-component "open this file in the Files tab" routing (quick-open,
// find-in-files jumps from elsewhere). Same idea as the `agency:open-note`
// event, plus a pending slot for the mount race: the requester switches to the
// Files tab first, so FilesView may not be listening yet — it consumes the
// pending request on mount instead.

export interface OpenFileRequest {
  /** Relative to the root FilesView is currently showing. */
  path: string;
  /** Optional 1-based line to land on. */
  line?: number;
}

const EVENT = "agency:open-file";

let pending: OpenFileRequest | null = null;

export function requestOpenFile(req: OpenFileRequest): void {
  pending = req;
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent<OpenFileRequest>(EVENT, { detail: req }));
  }
}

/** Return-and-clear the undelivered request, if any. */
export function consumePendingOpen(): OpenFileRequest | null {
  const req = pending;
  pending = null;
  return req;
}

/** Live delivery while FilesView is mounted. Returns the unsubscribe. */
export function onOpenFile(handler: (req: OpenFileRequest) => void): () => void {
  const fn = (e: Event) => {
    pending = null; // delivered — nothing left for a later mount to consume
    handler((e as CustomEvent<OpenFileRequest>).detail);
  };
  window.addEventListener(EVENT, fn);
  return () => window.removeEventListener(EVENT, fn);
}
