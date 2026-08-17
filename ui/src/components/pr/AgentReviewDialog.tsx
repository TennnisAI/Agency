import { useEffect, useState } from "react";
import { AgentProfile, createPrReviewRun, listProfiles } from "../../api";
import { agentLabel } from "../../agents";
import { useModalKeys } from "../../hooks/useModalKeys";
import { useRuns } from "../../store/runs";
import ModalBackdrop from "../ModalBackdrop";

// Start an agent that reviews this PR and then stays available to fix what it
// found. The agent works in the PR's head branch, so its fixes push straight
// onto the PR. Posting the findings to GitHub is opt-in: it writes to a page
// other people are watching, so it never happens by default.
export default function AgentReviewDialog({
  projectId,
  number,
  title,
  onStarted,
  onCancel,
}: {
  projectId: string;
  number: number;
  title: string;
  onStarted: () => void;
  onCancel: () => void;
}) {
  const { refreshRuns, setFocusedRun, setView, setTab, setPendingSession } = useRuns();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState("claude");
  const [postComments, setPostComments] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useModalKeys(onCancel, !busy);

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        setProfiles(ps);
        const names = ps.map((p) => p.name);
        if (!names.includes("claude") && names.length > 0) setAgent(names[0]);
      })
      .catch((e) => setError(String(e)));
  }, []);

  async function start() {
    setBusy(true);
    setError("");
    try {
      const { run, sessionId } = await createPrReviewRun(projectId, number, agent, postComments);
      await refreshRuns();
      // When the review had to share an existing run's worktree, open its tab
      // rather than the agent that wrote the code.
      if (sessionId) setPendingSession(sessionId);
      setFocusedRun(run.id);
      setTab("agents");
      setView("focus");
      onStarted();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onCancel}>
      <div
        className="modal confirm"
        role="dialog"
        aria-modal="true"
        aria-label="Review with an agent"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>Review with an agent</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">
            An agent reads #{number} “{title}” in a workspace on the PR's branch and reports what it
            finds. It stays open afterwards, so you can ask it to fix anything it raised and push.
          </p>
          {error && <div className="git-error">{error}</div>}
          <div className="merge-methods">
            <label className="gh-pick-agent">
              <span>agent</span>
              <select value={agent} disabled={busy} onChange={(e) => setAgent(e.target.value)}>
                {profiles.map((p) => (
                  <option key={p.name} value={p.name}>{agentLabel(p.name)}</option>
                ))}
              </select>
            </label>
            <label
              className="merge-method"
              title="Publishes the review to the pull request on GitHub, where everyone watching it can see. Leave off to keep the findings in the agent's terminal."
            >
              <input
                type="checkbox"
                checked={postComments}
                disabled={busy}
                onChange={(e) => setPostComments(e.target.checked)}
              />
              <span>Post the findings as comments on GitHub</span>
            </label>
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          <button className="btn-primary" disabled={busy || profiles.length === 0} onClick={start}>
            {busy ? "Starting…" : "Start review"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
