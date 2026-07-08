import { useEffect, useRef, useState } from "react";
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

// Start an agent from GitHub: an issue (its body becomes the agent's
// prompt) or an existing PR (its head branch is checked out for review).
export default function GhImportDialog({
  mode,
  onClose,
}: {
  mode: "issue" | "pr";
  onClose: () => void;
}) {
  const { selectedProjectId, refreshRuns, setView, setFocusedRun } = useRuns();
  // "error" is local-only (not a GhReadiness): the probe call itself failed,
  // which is distinct from gh being genuinely absent ("notInstalled").
  const [readiness, setReadiness] = useState<GhReadiness | "error" | null>(null);
  const [items, setItems] = useState<{ number: number; label: string }[] | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState("claude");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const firstItemRef = useRef<HTMLInputElement>(null);

  useModalKeys(onClose);

  // Probe gh readiness first: a missing/unauthenticated gh or remote-less
  // repo gets the guided setup, never a raw gh error dump. A caught error
  // means the probe couldn't run — not that gh is absent — so it lands on a
  // retryable "error" state rather than being conflated with "notInstalled".
  function probeReadiness() {
    if (!selectedProjectId) return;
    setReadiness(null);
    ghReadiness(selectedProjectId).then(setReadiness).catch(() => setReadiness("error"));
  }

  useEffect(() => {
    if (!selectedProjectId) return;
    probeReadiness();
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

  // Autofocus the primary input (the first pickable item) once the list loads
  // so keyboard users can navigate the choices immediately.
  useEffect(() => {
    if (items && items.length > 0) firstItemRef.current?.focus();
  }, [items]);

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
    // Backdrop click closes; the panel stops propagation so clicks inside it
    // don't bubble out to the overlay's close handler.
    <div className="settings-overlay" onClick={onClose}>
      <div className="merge-modal race-dialog" role="dialog" aria-modal="true" aria-label={mode === "issue" ? "Start from GitHub issue" : "Review GitHub PR"} onClick={(e) => e.stopPropagation()}>
        <div className="settings-head">
          <h2>{mode === "issue" ? "Start from GitHub issue" : "Review GitHub PR"}</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <p className="merge-note">
          {mode === "issue"
            ? "The issue becomes the prompt for a fresh agent."
            : "The PR's branch is checked out into a worktree so an agent can review or amend it."}
        </p>
        {error && <div className="git-error">{error}</div>}
        {readiness === "error" ? (
          <div className="pr-setup">
            <p className="merge-note">Couldn't check GitHub CLI status.</p>
            <button onClick={probeReadiness}>Retry</button>
          </div>
        ) : readiness !== null && readiness !== "ready" ? (
          <GhSetupHint readiness={readiness} onLeave={onClose} />
        ) : readiness === null || (readiness === "ready" && items === null) ? (
          <p className="merge-note">Loading…</p>
        ) : items && items.length === 0 && !error ? (
          <p className="merge-note">Nothing open to pick from.</p>
        ) : (
          <div className="gh-pick-list">
            {items?.map((it, idx) => (
              <label key={it.number} className="gh-pick-item">
                <input
                  ref={idx === 0 ? firstItemRef : undefined}
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
                {busy ? "Creating…" : "Create agent"}
              </button>
              <button className="ghost" onClick={onClose}>Cancel</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
