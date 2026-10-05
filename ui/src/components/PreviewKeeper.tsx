import { useEffect, useState } from "react";
import { PreviewTarget, previewTargets } from "../api";
import { isPreviewVisible, subscribePreviewHosts } from "../lib/previewHost";
import { usePopouts } from "../hooks/usePopouts";
import { popoutForSession } from "../lib/popout";

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
//
// Each window keeps its own: the visible set is per page, so the main window
// cannot see a Run tab open in a popped-out agent's window (AGE-252). It used
// to keep a hidden copy anyway, alongside the popout's visible one, and after
// any full reload of the app under preview the agent's tools drove whichever
// copy said hello last, half the time the one nobody could see. So the main
// window leaves popped-out runs alone, and a popout keeps its own run's page
// (`only`) by the same rule the main window keeps everyone else's.
export default function PreviewKeeper({ only }: { only?: string } = {}) {
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

  const popouts = usePopouts();
  const mine = (runId: string) =>
    only !== undefined ? runId === only : !popoutForSession(popouts, runId);
  const hosted = targets.filter((t) => t.active && mine(t.runId) && !isPreviewVisible(t.runId));
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
