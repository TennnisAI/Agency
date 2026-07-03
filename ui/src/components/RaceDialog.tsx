import { useEffect, useState } from "react";
import { createRace, listProfiles, listProjectBranches } from "../api";
import { agentLabel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useRuns } from "../store/runs";

// Fan one prompt out to several agents in parallel workspaces. This is the
// one flow where composing the prompt up-front is the point: it's typed once
// and delivered to every agent at launch.
export default function RaceDialog({ onClose }: { onClose: () => void }) {
  const { selectedProjectId, refreshRuns, setView, setFocusedRun } = useRuns();
  const [prompt, setPrompt] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        const names = ps.map((p) => p.name).filter((n) => n !== "shell");
        setAgents(names);
        // Preselect the first two so the fastest path is prompt → Start.
        setPicked(new Set(names.slice(0, 2)));
      })
      .catch(() => {});
    if (selectedProjectId) {
      listProjectBranches(selectedProjectId)
        .then((pb) => {
          setBranches(pb.branches);
          setBase(pb.current);
        })
        .catch(() => {});
    }
  }, [selectedProjectId]);

  function togglePick(name: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const canStart = prompt.trim().length > 0 && picked.size >= 2 && !busy;

  async function start() {
    if (!selectedProjectId || !canStart) return;
    setBusy(true);
    setError("");
    try {
      await createRace(selectedProjectId, prompt.trim(), [...picked], base || "HEAD", mergeTarget);
      await refreshRuns();
      // Grid view is the natural place to watch attempts side by side.
      setFocusedRun(null);
      setView("grid");
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="settings-overlay">
      <div className="merge-modal race-dialog">
        <div className="settings-head">
          <h2>Race agents</h2>
          <button onClick={onClose}>Close</button>
        </div>
        <p className="merge-note">
          The same prompt is sent to every selected agent, each in its own isolated workspace.
          Compare the attempts, merge the winner, discard the rest.
        </p>
        {error && <div className="git-error">{error}</div>}
        <textarea
          className="settings-input race-prompt"
          placeholder="What should the agents build?"
          value={prompt}
          autoFocus
          onChange={(e) => setPrompt(e.target.value)}
        />
        <div className="race-agents">
          {agents.map((name) => (
            <label key={name} className="race-agent">
              <input type="checkbox" checked={picked.has(name)} onChange={() => togglePick(name)} />
              <span>{agentLabel(name)}</span>
            </label>
          ))}
        </div>
        {branches.length > 0 && (
          <div className="branch-picker race-branches">
            <label className="branch-row">
              <span>from</span>
              <select value={base} onChange={(e) => setBase(e.target.value)}>
                {branches.map((b) => (
                  <option key={b} value={b}>{b}</option>
                ))}
              </select>
            </label>
            <label className="branch-row">
              <span>into</span>
              <select
                value={mergeTarget}
                onChange={(e) => setTargetOverride(e.target.value === base ? null : e.target.value)}
              >
                {branches.map((b) => (
                  <option key={b} value={b}>{b}</option>
                ))}
              </select>
            </label>
          </div>
        )}
        <div className="git-actions">
          <button disabled={!canStart} onClick={start}>
            {busy ? "Starting…" : `Start race (${picked.size} agents)`}
          </button>
          <button onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
