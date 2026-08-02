import { useEffect, useState } from "react";
import { FileRoot, readFileBase64 } from "../api";
import { Attachment } from "../lib/attachments";
import { reveal, revealLabel } from "../lib/fileActions";
import { useModalKeys } from "../hooks/useModalKeys";

// Bytes come over IPC as base64 and render as data URLs: the CSP is
// `img-src 'self' data:`, so there is no file:// or custom-scheme route to an
// attachment on disk. Same approach the docs live preview takes.
//
// Attachments are written once and never overwritten (the backend refuses to
// clobber), so the path alone is a sound cache key.
const urlCache = new Map<string, Promise<string>>();

const cacheKey = (root: FileRoot, repoPath: string) => `${root.kind}:${root.id}:${repoPath}`;

/**
 * Drop a cached preview. Removing an attachment trashes its file and frees the
 * name, so a later attachment can legitimately land on the same path — without
 * this, it would render the bytes of the one that was deleted.
 */
export function forgetAttachment(root: FileRoot, repoPath: string): void {
  urlCache.delete(cacheKey(root, repoPath));
}

function loadAttachment(root: FileRoot, repoPath: string): Promise<string> {
  const key = cacheKey(root, repoPath);
  let hit = urlCache.get(key);
  if (!hit) {
    // A failed load caches as "" so a deleted file doesn't re-fetch on every
    // render of the pane.
    hit = readFileBase64(root, repoPath)
      .then((b) => (b.tooLarge || !b.mime.startsWith("image/") ? "" : `data:${b.mime};base64,${b.b64}`))
      .catch(() => "");
    urlCache.set(key, hit);
  }
  return hit;
}

function Thumb({ root, a }: { root: FileRoot; a: Attachment }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void loadAttachment(root, a.repoPath).then((u) => { if (live) setUrl(u); });
    return () => { live = false; };
  }, [root.kind, root.id, a.repoPath]);

  if (url === null) return <span className="issue-attach-thumb loading" aria-hidden />;
  // A reference whose file is gone (hand-edited body, deleted asset) says so
  // rather than rendering a broken tile.
  if (!url) return <span className="issue-attach-thumb missing" title="File not found">?</span>;
  return <img className="issue-attach-thumb" src={url} alt={a.alt || a.name} />;
}

// Full-size view of one image attachment. Click anywhere (or Escape) to close.
function Lightbox({ root, a, onClose }: { root: FileRoot; a: Attachment; onClose: () => void }) {
  const [url, setUrl] = useState<string | null>(null);
  useModalKeys(onClose);
  useEffect(() => {
    let live = true;
    void loadAttachment(root, a.repoPath).then((u) => { if (live) setUrl(u); });
    return () => { live = false; };
  }, [root.kind, root.id, a.repoPath]);

  return (
    <div className="lightbox-backdrop" onClick={onClose}>
      <div className="lightbox" onClick={(e) => e.stopPropagation()}>
        <div className="lightbox-head">
          <span className="lightbox-name" title={a.repoPath}>{a.name}</span>
          <div className="spacer" />
          <button className="ghost" onClick={() => void reveal(root, a.repoPath)}>{revealLabel}</button>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        {url ? (
          <img className="lightbox-img" src={url} alt={a.alt || a.name} />
        ) : (
          <div className="diff-empty">{url === null ? "loading…" : "File not found."}</div>
        )}
      </div>
    </div>
  );
}

/**
 * The strip under an issue's body: every file the body links out of
 * `assets/`. Images preview inline and open full size; anything else is a chip
 * that reveals in the file manager. Removing a tile takes the link out of the
 * body — the caller decides what happens to the bytes.
 */
export default function IssueAttachments({
  root,
  attachments,
  onRemove,
}: {
  root: FileRoot;
  attachments: Attachment[];
  onRemove: (a: Attachment) => void;
}) {
  const [zoom, setZoom] = useState<Attachment | null>(null);
  // A body edit can drop the attachment being viewed out from under the
  // lightbox; close rather than showing a stale one.
  useEffect(() => {
    if (zoom && !attachments.some((a) => a.ref === zoom.ref)) setZoom(null);
  }, [attachments, zoom]);

  if (attachments.length === 0) return null;

  return (
    <div className="issue-attachments">
      <h3>{attachments.length === 1 ? "1 attachment" : `${attachments.length} attachments`}</h3>
      <div className="issue-attach-grid">
        {attachments.map((a) => (
          <div key={a.ref} className={`issue-attach${a.isImage ? " image" : " file"}`}>
            <button
              className="issue-attach-open"
              title={a.isImage ? `Open ${a.name}` : `${revealLabel}: ${a.name}`}
              onClick={() => (a.isImage ? setZoom(a) : void reveal(root, a.repoPath))}
            >
              {a.isImage ? <Thumb root={root} a={a} /> : <span className="issue-attach-glyph" aria-hidden>▤</span>}
              <span className="issue-attach-name">{a.name}</span>
            </button>
            <button
              className="issue-attach-remove"
              title="Remove attachment"
              onClick={() => onRemove(a)}
            >
              ✕
            </button>
          </div>
        ))}
      </div>
      {zoom && <Lightbox root={root} a={zoom} onClose={() => setZoom(null)} />}
    </div>
  );
}
