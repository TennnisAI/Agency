import { useCallback, useEffect, useState } from "react";
import { GhReadiness, PrInfo, ghReadiness, listGhPrs } from "../../api";
import GhSetupHint from "../GhSetupHint";
import PrReview from "./PrReview";

// The Source Control → Pull Requests surface: a list of the repo's open PRs on
// the left, the full review of the selected one on the right. `initialPr` opens
// a specific PR (the "Review in Agency" deep-link from the Approve window);
// `onConsumeInitial` clears it once applied so it doesn't re-fire.
export default function PrReviewPanel({
  projectId,
  initialPr,
  onConsumeInitial,
}: {
  projectId: string;
  initialPr: number | null;
  onConsumeInitial: () => void;
}) {
  const [readiness, setReadiness] = useState<GhReadiness | null>(null);
  const [prs, setPrs] = useState<PrInfo[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    setError("");
    try {
      const r = await ghReadiness(projectId);
      setReadiness(r);
      if (r === "ready") setPrs(await listGhPrs(projectId));
    } catch (e) {
      setError(String(e));
    }
  }, [projectId]);

  useEffect(() => {
    setPrs(null);
    setSelected(null);
    load();
  }, [load]);

  // Apply a deep-linked PR once we know about it, then clear the request.
  useEffect(() => {
    if (initialPr != null) {
      setSelected(initialPr);
      onConsumeInitial();
    }
  }, [initialPr, onConsumeInitial]);

  if (readiness === null && !error) {
    return <div className="pr-review-empty"><span className="spinner" /> Checking GitHub…</div>;
  }
  if (readiness !== null && readiness !== "ready") {
    return (
      <div className="pr-panel-setup">
        <GhSetupHint readiness={readiness} onLeave={() => {}} />
      </div>
    );
  }

  return (
    <div className="pr-panel">
      <aside className="pr-list">
        <div className="pr-list-head">
          <span>Pull requests</span>
          <span className="spacer" style={{ flex: 1 }} />
          <button className="git-iconbtn" title="Refresh" onClick={load}>⟲</button>
        </div>
        {error && <div className="git-error">{error}</div>}
        {prs === null && <div className="pr-list-empty"><span className="spinner" /></div>}
        {prs !== null && prs.length === 0 && <div className="pr-list-empty">No open pull requests.</div>}
        {prs?.map((p) => (
          <button
            key={p.number}
            className={`pr-list-row${selected === p.number ? " on" : ""}`}
            onClick={() => setSelected(p.number)}
          >
            <span className="pr-list-num">#{p.number}</span>
            <span className="pr-list-title">{p.title}</span>
            {p.isDraft && <span className="pr-list-draft">draft</span>}
          </button>
        ))}
      </aside>
      <div className="pr-panel-main">
        {selected != null ? (
          <PrReview key={selected} projectId={projectId} number={selected} />
        ) : (
          <div className="pr-review-empty">Select a pull request to review.</div>
        )}
      </div>
    </div>
  );
}
