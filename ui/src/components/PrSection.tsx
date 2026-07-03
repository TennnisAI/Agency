import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
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
}: {
  taskId: string;
  projectId: string;
  canCreate: boolean;
  onLeave: () => void;
}) {
  const [readiness, setReadiness] = useState<GhReadiness | null>(null);
  const [pr, setPr] = useState<PrInfo | null>(null);
  const [checks, setChecks] = useState<CheckItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState("");

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
      .then((r) => {
        if (!live) return;
        setReadiness(r);
        if (r === "ready") refreshStatus();
      })
      .catch(() => {});
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
      await sendCheckFeedback(taskId);
      setSent(true);
    } catch (e) {
      toastError(e, "Couldn't send check feedback");
    }
  }

  if (readiness === null) return null;

  const failing = checks.filter((c) => c.bucket === "fail" || c.bucket === "cancel");

  return (
    <div className="pr-section">
      <div className="pr-section-label">Pull request</div>
      {error && <div className="git-error">{error}</div>}

      {readiness !== null && readiness !== "ready" && (
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
        <p className="merge-note">Commit some work first — there's nothing to open a PR for.</p>
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
            <button className="settings-ghost-btn" onClick={() => openUrl(pr.url).catch(() => {})}>
              Open ↗
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
                {sent ? "Sent to agent ✓" : `Send ${failing.length} failing check${failing.length === 1 ? "" : "s"} to agent`}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
