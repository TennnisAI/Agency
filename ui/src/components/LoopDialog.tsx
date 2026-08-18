import { useEffect, useState } from "react";
import { Issue, createLoop, listProfiles, listProjectBranches, startIssueLoop } from "../api";
import { agentLabel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";
import { useAgentModels } from "../hooks/useAgentModels";
import ModelSelect from "./ModelSelect";

const PROMPT_TEMPLATE =
  "Read PROGRESS.md if it exists. Pick the single most important unfinished piece of: <task>. " +
  "Implement it, run the tests, commit with a clear message, and append one line to PROGRESS.md describing what you did.";

// Run one agent in a loop: the same prompt is re-sent to a fresh headless
// session until the check command exits 0 (or the attempt cap is spent).
// Racing is breadth; this is depth. With `issue` set the prompt comes from
// the issue instead (composed backend-side).
export default function LoopDialog({ onClose, issue, issueLabel }: {
  onClose: () => void;
  issue?: Issue;
  issueLabel?: string;
}) {
  const { selectedProjectId, refreshRuns, setView, setTab, setFocusedRun } = useRuns();
  const [prompt, setPrompt] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState("");
  // Seeded from the model this agent last ran on, and re-seeded when the agent
  // changes: model ids live in each CLI's own namespace, so one agent's pick
  // means nothing to the next.
  const { models, remembered } = useAgentModels();
  const [model, setModel] = useState<string | null>(null);
  const [modelTouched, setModelTouched] = useState(false);
  useEffect(() => {
    if (modelTouched) return;
    setModel(remembered(agent));
  }, [agent, remembered, modelTouched]);
  const [checkCommand, setCheckCommand] = useState("");
  // Kept as a raw string while editing so the field can be cleared/retyped;
  // clamped to [1,100] only on blur and at submit.
  const [maxAttempts, setMaxAttempts] = useState("10");
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useModalKeys(onClose);

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
  const canStart = (issue ? true : prompt.trim().length > 0) && agent !== "" && !busy;
  const fixedIterations = checkCommand.trim() === "";
  const clampAttempts = (v: string) => {
    const n = Math.floor(Number(v));
    return Number.isFinite(n) && n > 0 ? Math.max(1, Math.min(100, n)) : 10;
  };

  async function start() {
    if (!selectedProjectId || !canStart) return;
    setBusy(true);
    setError("");
    try {
      const run = issue
        ? await startIssueLoop(issue.id, agent, model, checkCommand.trim(), clampAttempts(maxAttempts), base || null, mergeTarget)
        : await createLoop(
            selectedProjectId,
            prompt.trim(),
            agent,
            model,
            base || "HEAD",
            mergeTarget,
            checkCommand.trim(),
            clampAttempts(maxAttempts),
          );
      if (issue) setTab("agents"); // jump from the board to the running loop
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
    <div className="settings-overlay" onClick={(e) => { e.stopPropagation(); onClose(); }}>
      <div className="merge-modal race-dialog" role="dialog" aria-modal="true" aria-label="Loop agent" onClick={(e) => e.stopPropagation()}>
        <div className="settings-head">
          <h2>Loop agent</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <p className="merge-note">
          The prompt is re-sent to a fresh headless session until the check command exits 0.
          Each attempt is committed to the run's branch; review and merge when the loop completes.
        </p>
        {error && <div className="git-error">{error}</div>}
        {issue ? (
          <div className="issue-dialog-summary">
            <code>{issueLabel}</code>
            <span>{issue.title}</span>
          </div>
        ) : (
          <>
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
          </>
        )}
        <div className="loop-fields">
          <label className="branch-row">
            <span>agent</span>
            <select
              value={agent}
              onChange={(e) => { setAgent(e.target.value); setModelTouched(false); }}
            >
              {agents.map((name) => (
                <option key={name} value={name}>{agentLabel(name)}</option>
              ))}
            </select>
          </label>
          {models[agent]?.supported && (
            <div className="branch-row">
              <span>model</span>
              <ModelSelect
                info={models[agent]}
                value={model}
                onChange={(m) => { setModel(m); setModelTouched(true); }}
              />
            </div>
          )}
          <label className="branch-row">
            <span>done when</span>
            <input
              className="settings-input loop-check"
              placeholder="e.g. pnpm test (exits 0 when done; empty = fixed iterations)"
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
              onChange={(e) => setMaxAttempts(e.target.value)}
              onBlur={() => setMaxAttempts(String(clampAttempts(maxAttempts)))}
            />
          </label>
        </div>
        {fixedIterations && (
          <p className="merge-note">
            No check command: the loop runs exactly {clampAttempts(maxAttempts)} attempts, without verifying completion.
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
