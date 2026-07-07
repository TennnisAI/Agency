import { useEffect, useState } from "react";
import {
  GhReadiness,
  IssueItem,
  PrInfo,
  createRunFromIssue,
  createRunFromPr,
  ghReadiness,
  listGhIssues,
  listGhPrs,
  listProfiles,
} from "../api";
import { agentLabel } from "../agents";
import GhSetupHint from "./GhSetupHint";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";

// Start a workspace from GitHub: an issue (its body becomes the agent's
// prompt) or an existing PR (its head branch is checked out for review).
export default function GhImportDialog({
  mode,
  onClose,
}: {
  mode: "issue" | "pr";
  onClose: () => void;
}) {
  const { selectedProjectId, refreshRuns, setView, setFocusedRun } = useRuns();
  const [readiness, setReadiness] = useState<GhReadiness | null>(null);
  const [items, setItems] = useState<{ number: number; label: string }[] | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState("claude");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useModalKeys(onClose);

  // Probe gh readiness first: a missing/unauthenticated gh or remote-less
  // repo gets the guided setup, never a raw gh error dump.
  useEffect(() => {
    if (!selectedProjectId) return;
    ghReadiness(selectedProjectId).then(setReadiness).catch(() => setReadiness("notInstalled"));
    listProfiles()
      .then((ps) => {
        const names = ps.map((p) => p.name).filter((n) => n !== "shell");
        setAgents(names);
        if (!names.includes("claude") && names.length > 0) setAgent(names[0]);
      })
      .catch(() => {});
  }, [selectedProjectId]);

  useEffect(() => {
    if (!selectedProjectId || readiness !== "ready") return;
    const load =
      mode === "issue"
        ? listGhIssues(selectedProjectId).then((xs: IssueItem[]) =>
            xs.map((i) => ({ number: i.number, label: `#${i.number} ${i.title}` })),
          )
        : listGhPrs(selectedProjectId).then((xs: PrInfo[]) =>
            xs.map((p) => ({ number: p.number, label: `#${p.number} ${p.title}` })),
          );
    load.then(setItems).catch((e) => {
      setItems([]);
      setError(String(e));
    });
  }, [mode, selectedProjectId, readiness]);

  async function create() {
    if (!selectedProjectId || picked === null) return;
    setBusy(true);
    setError("");
    try {
      const run =
        mode === "issue"
          ? await createRunFromIssue(selectedProjectId, picked, agent)
          : await createRunFromPr(selectedProjectId, picked, agent);
      await refreshRuns();
      setFocusedRun(run.id);
      setView("focus");
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="settings-overlay">
      <div className="merge-modal race-dialog" role="dialog" aria-modal="true" aria-label={mode === "issue" ? "Start from GitHub issue" : "Review GitHub PR"}>
        <div className="settings-head">
          <h2>{mode === "issue" ? "Start from GitHub issue" : "Review GitHub PR"}</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <p className="merge-note">
          {mode === "issue"
            ? "The issue becomes the agent's prompt in a fresh workspace."
            : "The PR's branch is checked out into a workspace so an agent can review or amend it."}
        </p>
        {error && <div className="git-error">{error}</div>}
        {readiness !== null && readiness !== "ready" ? (
          <GhSetupHint readiness={readiness} onLeave={onClose} />
        ) : readiness === null || (readiness === "ready" && items === null) ? (
          <p className="merge-note">Loading…</p>
        ) : items && items.length === 0 && !error ? (
          <p className="merge-note">Nothing open to pick from.</p>
        ) : (
          <div className="gh-pick-list">
            {items?.map((it) => (
              <label key={it.number} className="gh-pick-item">
                <input
                  type="radio"
                  name="gh-pick"
                  checked={picked === it.number}
                  onChange={() => setPicked(it.number)}
                />
                <span>{it.label}</span>
              </label>
            ))}
          </div>
        )}
        {readiness === "ready" && (
          <>
            <div className="gh-pick-agent">
              <span>agent</span>
              <select value={agent} onChange={(e) => setAgent(e.target.value)}>
                {agents.map((n) => (
                  <option key={n} value={n}>{agentLabel(n)}</option>
                ))}
              </select>
            </div>
            <div className="git-actions">
              <button disabled={picked === null || busy} onClick={create}>
                {busy ? "Creating…" : "Create workspace"}
              </button>
              <button className="ghost" onClick={onClose}>Cancel</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
