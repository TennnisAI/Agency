import { useEffect, useRef, useState } from "react";
import { Issue, RaceAttempt, createRace, listProfiles, listProjectBranches, startIssueRace } from "../api";
import { agentLabel, nextRaceModel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";
import { useAgentModels } from "../hooks/useAgentModels";
import ModelSelect from "./ModelSelect";

// One row of the race: an agent and the model it runs on. `model` absent (as
// opposed to null) means nothing was picked, so the row falls back to whatever
// that agent last ran on — resolved at render, because the remembered map
// arrives after the rows do.
//
// Keyed by a counter rather than by agent: an agent can hold two rows, and even
// (agent, model) is not unique until both models are chosen.
interface Attempt {
  key: number;
  agent: string;
  model?: string | null;
}

// Fan one prompt out to several attempts in parallel workspaces. This is the
// one flow where composing the prompt up-front is the point: it's typed once
// and delivered to every agent at launch. With `issue` set the prompt comes
// from the issue instead (composed backend-side), racing agents on it.
//
// The unit is the attempt, not the agent, so a picked agent can be given a
// second row and raced against itself on another model (AGE-118).
export default function RaceDialog({ onClose, issue, issueLabel }: {
  onClose: () => void;
  issue?: Issue;
  issueLabel?: string;
}) {
  const { selectedProjectId, refreshRuns, setView, setTab, setFocusedRun } = useRuns();
  const [prompt, setPrompt] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [attempts, setAttempts] = useState<Attempt[]>([]);
  const nextKey = useRef(0);
  // A model per attempt, not one for the race: "opus" means nothing to Codex,
  // so each attempt carries its own choice, seeded from what its agent last
  // ran on.
  const { models, remembered } = useAgentModels();
  const modelFor = (a: Attempt) => (a.model === undefined ? remembered(a.agent) : a.model);
  const attemptsFor = (agent: string) => attempts.filter((a) => a.agent === agent);
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useModalKeys(onClose);

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        const names = ps.map((p) => p.name);
        setAgents(names);
        // Preselect the first two so the fastest path is prompt → Start.
        setAttempts(names.slice(0, 2).map((agent) => ({ key: nextKey.current++, agent })));
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

  // The checkbox is over the agent, so unticking it drops every row it has.
  function togglePick(name: string) {
    setAttempts(
      attemptsFor(name).length > 0
        ? attempts.filter((a) => a.agent !== name)
        : [...attempts, { key: nextKey.current++, agent: name }],
    );
  }

  // A second row for an agent already racing, seeded with a model it isn't
  // using yet, which is the whole point of the button.
  function addModel(name: string) {
    const info = models[name];
    const model = nextRaceModel(attemptsFor(name).map(modelFor), info?.suggested ?? [], info?.recent ?? []);
    setAttempts([...attempts, { key: nextKey.current++, agent: name, model }]);
  }

  const setModel = (key: number, model: string | null) =>
    setAttempts(attempts.map((a) => (a.key === key ? { ...a, model } : a)));
  const removeAttempt = (key: number) => setAttempts(attempts.filter((a) => a.key !== key));

  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const canStart = (issue ? true : prompt.trim().length > 0) && attempts.length >= 2 && !busy;

  // Sent in the order the dialog lists them (profile order, extra rows under
  // the agent they belong to), so the board comes up reading like the dialog.
  const raceAttempts = (): RaceAttempt[] =>
    agents.flatMap((name) => attemptsFor(name).map((a) => ({ agent: a.agent, model: modelFor(a) })));

  async function start() {
    if (!selectedProjectId || !canStart) return;
    setBusy(true);
    setError("");
    try {
      if (issue) {
        await startIssueRace(issue.id, raceAttempts(), base || null, mergeTarget);
        setTab("agents"); // jump from the board to where the attempts run
      } else {
        await createRace(selectedProjectId, prompt.trim(), raceAttempts(), base || "HEAD", mergeTarget);
      }
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
    <div className="settings-overlay" onClick={(e) => { e.stopPropagation(); onClose(); }}>
      <div className="merge-modal race-dialog" role="dialog" aria-modal="true" aria-label="Race agents" onClick={(e) => e.stopPropagation()}>
        <div className="settings-head">
          <h2>Race agents</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <p className="merge-note">
          {issue
            ? "The issue is sent to every attempt, each in its own isolated workspace. Compare the attempts, merge the winner, discard the rest."
            : "The same prompt is sent to every attempt, each in its own isolated workspace. Compare the attempts, merge the winner, discard the rest."}
        </p>
        {error && <div className="git-error">{error}</div>}
        {issue ? (
          <div className="issue-dialog-summary">
            <code>{issueLabel}</code>
            <span>{issue.title}</span>
          </div>
        ) : (
          <textarea
            className="settings-input race-prompt"
            placeholder="What should the agents build?"
            value={prompt}
            autoFocus
            onChange={(e) => setPrompt(e.target.value)}
          />
        )}
        <div className="race-agents">
          {agents.map((name) => {
            const mine = attemptsFor(name);
            const [first, ...extra] = mine;
            return (
              <div key={name} className="race-agent-block">
                <div className="race-agent-row">
                  <label className="race-agent">
                    <input type="checkbox" checked={mine.length > 0} onChange={() => togglePick(name)} />
                    <span>{agentLabel(name)}</span>
                  </label>
                  {/* Only for the agents actually racing: an unticked row's model
                      is not part of this race and would just be noise. */}
                  {first && (
                    <ModelSelect
                      compact
                      info={models[name]}
                      projectId={selectedProjectId}
                      value={modelFor(first)}
                      onChange={(m) => setModel(first.key, m)}
                    />
                  )}
                  {/* Offered only where a model can be pinned at all: for an
                      agent whose CLI takes no model flag, a second attempt
                      would be the same attempt twice. */}
                  {first && models[name]?.supported && (
                    <button
                      type="button"
                      className="settings-ghost-btn race-add-model"
                      title={`Race ${agentLabel(name)} against itself on another model`}
                      onClick={() => addModel(name)}
                    >
                      + model
                    </button>
                  )}
                </div>
                {extra.map((a) => (
                  <div key={a.key} className="race-agent-row race-attempt-row">
                    <span className="race-attempt-tick">↳</span>
                    <ModelSelect
                      compact
                      info={models[name]}
                      projectId={selectedProjectId}
                      value={modelFor(a)}
                      onChange={(m) => setModel(a.key, m)}
                    />
                    <button
                      type="button"
                      className="ghost-x"
                      title="Drop this attempt"
                      onClick={() => removeAttempt(a.key)}
                    >
                      ✕
                    </button>
                  </div>
                ))}
              </div>
            );
          })}
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
            {busy ? "Starting…" : `Start race (${attempts.length} attempt${attempts.length === 1 ? "" : "s"})`}
          </button>
          <button className="ghost" onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
