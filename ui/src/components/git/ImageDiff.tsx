import { useEffect, useState } from "react";
import { BlobSide, BlobSides, gitBlobSides } from "../../api";
import { formatSize } from "./binary";

/** Pixel size of a loaded image, filled in from the <img> itself — no decoder
 *  needed on the Rust side just to print "64 × 64". */
type Dims = { w: number; h: number };

/**
 * The before/after of an image change. git has no textual diff to offer for one
 * ("Binary files a/x and b/x differ"), so the two versions are fetched as bytes
 * and shown as pictures: two-up by default, or under a swipe divider when both
 * sides exist and the difference is too fine to spot side by side.
 */
export default function ImageDiff({
  taskId, path, staged, hash,
}: {
  taskId: string;
  path: string;
  staged: boolean;
  hash?: string;
}) {
  const [sides, setSides] = useState<BlobSides | null>(null);
  const [error, setError] = useState("");
  const [swipe, setSwipe] = useState(false);
  const [split, setSplit] = useState(50);
  const [oldDims, setOldDims] = useState<Dims | null>(null);
  const [newDims, setNewDims] = useState<Dims | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSides(null); setError(""); setOldDims(null); setNewDims(null);
    gitBlobSides(taskId, path, staged, hash ?? null)
      .then((s) => { if (!cancelled) setSides(s); })
      .catch((e) => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [taskId, path, staged, hash]);

  if (error) return <div className="git-error">{error}</div>;
  if (!sides) return <div className="diff-empty">loading…</div>;

  const { old: before, new: after } = sides;
  if (!before && !after) return <div className="diff-empty">This image is not in either version.</div>;

  const url = (b64: string) => `data:${sides.mime};base64,${b64}`;
  // A mode-only change (chmod) leaves the bytes alone, so say so rather than
  // showing the same picture twice and letting the eye hunt for a difference.
  const identical = !!before && !!after && before.b64 === after.b64;
  const canSwipe = !!before && !!after && !before.tooLarge && !after.tooLarge;
  const meta = (side: { size: number }, dims: Dims | null) =>
    [dims && `${dims.w} × ${dims.h}`, formatSize(side.size)].filter(Boolean).join(" · ");

  const pane = (
    side: BlobSide | null,
    label: string,
    kind: "old" | "new",
    onDims: (d: Dims) => void,
    dims: Dims | null,
  ) => (
    <div className={`imgdiff-pane ${kind}`}>
      <div className="imgdiff-label">
        <span className="imgdiff-side">{label}</span>
        {side && <span className="imgdiff-meta">{meta(side, dims)}</span>}
      </div>
      <div className="imgdiff-stage">
        {!side && (
          <span className="imgdiff-none">
            {kind === "old" ? "Added in this change" : "Deleted in this change"}
          </span>
        )}
        {side?.tooLarge && <span className="imgdiff-none">Too large to preview</span>}
        {side && !side.tooLarge && (
          <img src={url(side.b64)} alt={`${label} ${path}`}
            onLoad={(e) => onDims({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })} />
        )}
      </div>
    </div>
  );

  return (
    <div className="diffviewer imgdiff">
      <div className="diff-toolbar">
        <span className="diff-path">{path}</span>
        {identical && <span className="imgdiff-note">bytes unchanged</span>}
        <span className="spacer" style={{ flex: 1 }} />
        {canSwipe && swipe && (
          <input className="imgdiff-slider" type="range" min={0} max={100} value={split}
            aria-label="Reveal the new image" onChange={(e) => setSplit(+e.target.value)} />
        )}
        {canSwipe && (
          <button className="git-iconbtn" onClick={() => setSwipe((s) => !s)}>
            {swipe ? "Side by side" : "Swipe"}
          </button>
        )}
      </div>
      {canSwipe && swipe ? (
        <div className="imgdiff-body swipe">
          <div className="imgdiff-stage">
            {/* The box is sized by the old image; the new one is stretched over
                it and clipped from the right. Clip and divider are percentages
                of that one box, so the line sits exactly where the reveal ends
                and both versions are compared over the very same pixels. */}
            <div className="imgdiff-swipebox">
              <img src={url(before!.b64)} alt={`Before ${path}`}
                onLoad={(e) => setOldDims({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })} />
              <img className="imgdiff-over" src={url(after!.b64)} alt={`After ${path}`}
                style={{ clipPath: `inset(0 ${100 - split}% 0 0)` }}
                onLoad={(e) => setNewDims({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })} />
              <div className="imgdiff-divider" style={{ left: `${split}%` }} />
            </div>
          </div>
          <div className="imgdiff-swipe-meta">
            <span className="imgdiff-side old">Before</span>
            <span className="imgdiff-meta">{meta(before!, oldDims)}</span>
            <span className="spacer" style={{ flex: 1 }} />
            <span className="imgdiff-meta">{meta(after!, newDims)}</span>
            <span className="imgdiff-side new">After</span>
          </div>
        </div>
      ) : (
        <div className="imgdiff-body twoup">
          {pane(before, "Before", "old", setOldDims, oldDims)}
          {pane(after, "After", "new", setNewDims, newDims)}
        </div>
      )}
    </div>
  );
}
