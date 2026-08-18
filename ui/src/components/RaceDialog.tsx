import { useEffect, useState } from "react";
import { AgentModels, Issue, createRace, listProfiles, listProjectBranches, startIssueRace } from "../api";
import { agentLabel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";
import { useAgentModels } from "../hooks/useAgentModels";
import ModelSelect from "./ModelSelect";

// Fan one prompt out to several agents in parallel workspaces. This is the
// one flow where composing the prompt up-front is the point: it's typed once
// and delivered to every agent at launch. With `issue` set the prompt comes
// from the issue instead (composed backend-side), racing agents on it.
export default function RaceDialog({ onClose, issue, issueLabel }: {
  onClose: () => void;
  issue?: Issue;
  issueLabel?: string;
}) {
  const { selectedProjectId, refreshRuns, setView, setTab, setFocusedRun } = useRuns();
  const [prompt, setPrompt] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  // A model per attempt, not one for the race: "opus" means nothing to Codex,
  // so each agent carries its own choice, seeded from what it last ran on.
  const { models, remembered } = useAgentModels();
  const [chosen, setChosen] = useState<Record<string, string | null>>({});
  const modelFor = (agent: string) => (agent in chosen ? chosen[agent] : remembered(agent));
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
  const canStart = (issue ? true : prompt.trim().length > 0) && picked.size >= 2 && !busy;

  // Only the agents actually racing, and only where a model was chosen: an
  // agent left on its own default is simply absent from the map.
  const raceModels = (): AgentModels =>
    Object.fromEntries(
      [...picked].flatMap((a) => {
        const m = modelFor(a);
        return m ? [[a, m] as [string, string]] : [];
      }),
    );

  async function start() {
    if (!selectedProjectId || !canStart) return;
    setBusy(true);
    setError("");
    try {
      if (issue) {
        await startIssueRace(issue.id, [...picked], raceModels(), base || null, mergeTarget);
        setTab("agents"); // jump from the board to where the attempts run
      } else {
        await createRace(selectedProjectId, prompt.trim(), [...picked], raceModels(), base || "HEAD", mergeTarget);
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
            ? "The issue is sent to every selected agent, each in its own isolated workspace. Compare the attempts, merge the winner, discard the rest."
            : "The same prompt is sent to every selected agent, each in its own isolated workspace. Compare the attempts, merge the winner, discard the rest."}
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
          {agents.map((name) => (
            <div key={name} className="race-agent-row">
              <label className="race-agent">
                <input type="checkbox" checked={picked.has(name)} onChange={() => togglePick(name)} />
                <span>{agentLabel(name)}</span>
              </label>
              {/* Only for the agents actually racing: an unticked row's model
                  is not part of this race and would just be noise. */}
              {picked.has(name) && (
                <ModelSelect
                  compact
                  info={models[name]}
                  value={modelFor(name)}
                  onChange={(m) => setChosen((c) => ({ ...c, [name]: m }))}
                />
              )}
            </div>
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
          <button className="ghost" onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
