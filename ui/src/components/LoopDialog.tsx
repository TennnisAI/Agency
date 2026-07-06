import { useEffect, useState } from "react";
import { createLoop, listProfiles, listProjectBranches } from "../api";
import { agentLabel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useRuns } from "../store/runs";

const PROMPT_TEMPLATE =
  "Read PROGRESS.md if it exists. Pick the single most important unfinished piece of: <task>. " +
  "Implement it, run the tests, commit with a clear message, and append one line to PROGRESS.md describing what you did.";

// Run one agent in a loop: the same prompt is re-sent to a fresh headless
// session until the check command exits 0 (or the attempt cap is spent).
// Racing is breadth; this is depth.
export default function LoopDialog({ onClose }: { onClose: () => void }) {
  const { selectedProjectId, refreshRuns, setView, setFocusedRun } = useRuns();
  const [prompt, setPrompt] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState("");
  const [checkCommand, setCheckCommand] = useState("");
  const [maxAttempts, setMaxAttempts] = useState(10);
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        // Only agents with a headless one-shot recipe can loop.
        const names = ps.filter((p) => p.loop_args && p.loop_args.length > 0).map((p) => p.name);
        setAgents(names);
        setAgent((a) => (a && names.includes(a) ? a : names[0] ?? ""));
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

  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const canStart = prompt.trim().length > 0 && agent !== "" && !busy;
  const fixedIterations = checkCommand.trim() === "";

  async function start() {
    if (!selectedProjectId || !canStart) return;
    setBusy(true);
    setError("");
    try {
      const run = await createLoop(
        selectedProjectId,
        prompt.trim(),
        agent,
        base || "HEAD",
        mergeTarget,
        checkCommand.trim(),
        maxAttempts,
      );
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
      <div className="merge-modal race-dialog">
        <div className="settings-head">
          <h2>Loop agent</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <p className="merge-note">
          The prompt is re-sent to a fresh headless session until the check command exits 0.
          Each attempt is committed to the run's branch; review and merge when the loop completes.
        </p>
        {error && <div className="git-error">{error}</div>}
        <textarea
          className="settings-input race-prompt"
          placeholder="What should the agent keep working on? Best prompts pick one task per attempt, run the tests, and commit."
          value={prompt}
          autoFocus
          onChange={(e) => setPrompt(e.target.value)}
        />
        <button
          className="ghost loop-template"
          type="button"
          onClick={() => setPrompt(PROMPT_TEMPLATE)}
        >Use prompt template</button>
        <div className="loop-fields">
          <label className="branch-row">
            <span>agent</span>
            <select value={agent} onChange={(e) => setAgent(e.target.value)}>
              {agents.map((name) => (
                <option key={name} value={name}>{agentLabel(name)}</option>
              ))}
            </select>
          </label>
          <label className="branch-row">
            <span>done when</span>
            <input
              className="settings-input loop-check"
              placeholder="e.g. pnpm test — exits 0 when done (empty = fixed iterations)"
              value={checkCommand}
              onChange={(e) => setCheckCommand(e.target.value)}
            />
          </label>
          <label className="branch-row">
            <span>max attempts</span>
            <input
              className="settings-input settings-notif-secs"
              type="number"
              min={1}
              max={100}
              value={maxAttempts}
              onChange={(e) => setMaxAttempts(Math.max(1, Math.min(100, Number(e.target.value) || 10)))}
            />
          </label>
        </div>
        {fixedIterations && (
          <p className="merge-note">
            No check command: the loop runs exactly {maxAttempts} attempts, without verifying completion.
          </p>
        )}
        {agents.length === 0 && (
          <p className="merge-note">
            No loop-capable agents. Add loop args (a headless one-shot recipe) to an agent profile first.
          </p>
        )}
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
            {busy ? "Starting…" : "Start loop"}
          </button>
          <button className="ghost" onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
