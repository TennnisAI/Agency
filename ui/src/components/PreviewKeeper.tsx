import { useEffect, useState } from "react";
import { PreviewTarget, previewTargets } from "../api";
import { isPreviewVisible, subscribePreviewHosts } from "../lib/previewHost";

// Keeps a dispatched agent's preview alive when nobody is looking at it.
//
// The agent's preview tools (AGE-143) act through a bridge script running
// inside the preview page, so the page has to exist somewhere for the tools to
// work at all. When the run's Run tab is open, the visible pane is that page.
// This component covers every other moment: for each run whose preview server
// is up and whose app port is actually serving, it mounts an invisible iframe
// on the same instrumented URL. `visibility: hidden` rather than
// `display: none` on purpose — the page must keep real layout for snapshots
// and clicks to mean anything, it just must not paint.
//
// When a RunPanel starts showing a run's preview it registers in previewHost
// and the hidden copy unmounts; the two coexist for at most a beat, and the
// server treats whichever page said hello last as the live one.
export default function PreviewKeeper() {
  const [targets, setTargets] = useState<PreviewTarget[]>([]);
  // Bumped by the previewHost registry so visibility changes re-render.
  const [, setEpoch] = useState(0);

  useEffect(() => subscribePreviewHosts(() => setEpoch((e) => e + 1)), []);

  useEffect(() => {
    let live = true;
    const load = () =>
      previewTargets()
        .then((t) => {
          if (!live) return;
          // Only replace state when something changed, so React keeps the
          // iframe elements (and their pages) instead of reloading them.
          setTargets((cur) =>
            JSON.stringify(cur) === JSON.stringify(t) ? cur : t,
          );
        })
        .catch(() => {});
    load();
    const iv = setInterval(load, 3000);
    return () => { live = false; clearInterval(iv); };
  }, []);

  const hosted = targets.filter((t) => t.active && !isPreviewVisible(t.runId));
  if (!hosted.length) return null;
  return (
    <>
      {hosted.map((t) => (
        <iframe
          key={t.runId}
          className="preview-keeper"
          src={t.url}
          title=""
          aria-hidden
          tabIndex={-1}
        />
      ))}
    </>
  );
}
