import { useCallback, useEffect, useState } from "react";
import {
  CheckItem,
  GhReadiness,
  PrInfo,
  createPr,
  ghReadiness,
  prStatus,
  sendCheckFeedback,
} from "../api";
import GhSetupHint from "./GhSetupHint";
import RunRemoveDialog from "./RunRemoveDialog";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";

const BUCKET_ICON: Record<string, string> = {
  pass: "✓",
  fail: "✕",
  cancel: "✕",
  pending: "○",
  skipping: "–",
};

// The PR half of the finish flow: guided gh setup (install → auth → remote),
// create-PR, and a live check rollup with "send failures to the agent".
// `onLeave` closes the host modal when we navigate away to a setup terminal.
export default function PrSection({
  taskId,
  projectId,
  canCreate,
  onLeave,
  onReviewPr,
}: {
  taskId: string;
  projectId: string;
  canCreate: boolean;
  onLeave: () => void;
  // Deep-links "View PR" to the review panel for the created/existing PR.
  // Required, not optional: a missing handler is how this button silently
  // regressed to opening github.com instead (AGE-59).
  onReviewPr: (number: number) => void;
}) {
  const [readiness, setReadiness] = useState<GhReadiness | null>(null);
  const [pr, setPr] = useState<PrInfo | null>(null);
  const [checks, setChecks] = useState<CheckItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState(false);
  const [queued, setQueued] = useState(false);
  const [error, setError] = useState("");
  // True until the opening probe (gh readiness, then the PR's current state)
  // has settled. Rendering the section's real content before then shows
  // "Create pull request" for a run that turns out to already have one.
  const [probing, setProbing] = useState(true);
  // Offered once the PR has landed: at that point the local branch is a second
  // name for commits that are on the remote, and the worktree is a checkout of
  // work that is finished. See AGE-149 — tidying up should be the next step of
  // the flow, not something you remember to do days later.
  const [tidying, setTidying] = useState(false);
  const { runs } = useRuns();
  const run = runs.find((r) => r.id === taskId);

  const refreshStatus = useCallback(async () => {
    try {
      const s = await prStatus(taskId);
      setPr(s.pr);
      setChecks(s.checks);
    } catch {
      /* transient — next poll retries */
    }
  }, [taskId]);

  useEffect(() => {
    let live = true;
    ghReadiness(projectId)
      .then(async (r) => {
        if (!live) return;
        setReadiness(r);
        if (r === "ready") await refreshStatus();
      })
      .catch(() => {})
      .finally(() => {
        if (live) setProbing(false);
      });
    return () => {
      live = false;
    };
  }, [projectId, refreshStatus]);

  // Live check rollup while a PR exists and the modal is open.
  useEffect(() => {
    if (!pr) return;
    const t = window.setInterval(refreshStatus, 30000);
    return () => window.clearInterval(t);
  }, [pr, refreshStatus]);

  async function doCreate() {
    setBusy(true);
    setError("");
    try {
      setPr(await createPr(taskId));
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function doSendFailures() {
    try {
      // False means it is queued behind the turn the agent is already in.
      setQueued(!(await sendCheckFeedback(taskId)));
      setSent(true);
    } catch (e) {
      toastError(e, "Couldn't send check feedback");
    }
  }

  const failing = checks.filter((c) => c.bucket === "fail" || c.bucket === "cancel");

  // Render the section (with its label and border) from the first frame, and
  // hold it at a spinner until the whole opening probe has settled — the
  // section's real content arrives at its final shape rather than in two hops.
  // The modal is top-anchored, so it grows downward and leaves the Merge button
  // above it alone.
  if (probing || readiness === null) {
    return (
      <div className="pr-section">
        <div className="pr-section-label">Pull request</div>
        <p className="merge-note"><span className="spinner" /> Checking GitHub…</p>
      </div>
    );
  }

  return (
    <div className="pr-section">
      <div className="pr-section-label">Pull request</div>
      {error && <div className="git-error">{error}</div>}

      {readiness !== "ready" && (
        <GhSetupHint readiness={readiness} onLeave={onLeave} />
      )}

      {readiness === "ready" && !pr && canCreate && (
        <div className="pr-setup">
          <button disabled={busy} onClick={doCreate}>
            {busy ? "Creating PR…" : "Create pull request"}
          </button>
          <p className="merge-note">Pushes the branch and opens a PR with a generated description.</p>
        </div>
      )}

      {readiness === "ready" && !pr && !canCreate && (
        <p className="merge-note">Commit some work first. There's nothing to open a PR for.</p>
      )}

      {pr && (
        <div className="pr-info">
          <div className="pr-info-head">
            <span className={`pr-state pr-state-${pr.state.toLowerCase()}`}>
              {pr.isDraft ? "DRAFT" : pr.state}
            </span>
            <span className="pr-title">
              #{pr.number} {pr.title}
            </span>
            <span className="spacer" />
            {/* Viewing a PR keeps you in Agency: the review panel is the richer
                surface, and it carries its own link out to github.com. */}
            <button className="settings-ghost-btn" onClick={() => onReviewPr(pr.number)}>
              View PR
            </button>
            <button className="settings-ghost-btn" onClick={refreshStatus} title="Refresh checks">
              ↻
            </button>
          </div>
          {checks.length > 0 ? (
            <ul className="pr-checks">
              {checks.map((c) => (
                <li key={c.name} className={`pr-check pr-check-${c.bucket || "pending"}`}>
                  <span className="pr-check-icon">{BUCKET_ICON[c.bucket] ?? "○"}</span>
                  <span className="pr-check-name">{c.name}</span>
                  {c.description && <span className="pr-check-desc">{c.description}</span>}
                </li>
              ))}
            </ul>
          ) : (
            <p className="merge-note">No checks reported yet.</p>
          )}
          {failing.length > 0 && (
            <div className="git-actions">
              <button disabled={sent} onClick={doSendFailures}>
                {sent
                  ? queued
                    ? "Queued for the agent"
                    : "Sent to agent ✓"
                  : `Send ${failing.length} failing check${failing.length === 1 ? "" : "s"} to agent`}
              </button>
            </div>
          )}
          {/* Merged upstream: nothing here is the only copy of anything any
              more, and the agent has finished. Only for an agent with a
              worktree — a run in the checkout has nothing to tear down. */}
          {pr.state === "MERGED" && run?.kind === "agent" && run.worktree && (
            <div className="git-actions">
              <button onClick={() => setTidying(true)}>Archive agent</button>
              <span className="merge-note">
                This PR is merged, so the worktree and the local branch have nothing left to hold.
              </span>
            </div>
          )}
        </div>
      )}
      {tidying && run && (
        <RunRemoveDialog run={run} action="archive" onClose={() => setTidying(false)} onRemoved={onLeave} />
      )}
    </div>
  );
}
