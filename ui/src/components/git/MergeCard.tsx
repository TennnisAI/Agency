import { useEffect, useState } from "react";
import {
  CloneProgress, MergeState, RunInfo, abortMergeTask, gitStatus, mergePreview, mergeStatus,
} from "../../api";
import { useRuns } from "../../store/runs";
import ConfirmDialog from "../ConfirmDialog";
import PillSelect from "../PillSelect";
import {
  endMergeFlow, handoffNote, mergeCardPhase, mergeFlow, patchMergeFlow, reconcileFlow, useMergeFlows,
} from "./mergeFlow";
import { useMergeHandoff } from "./useMergeHandoff";

// A conflicted merge, in the source control panel, for as long as it lasts. It
// used to live only in the Approve window, which meant keeping a modal up over
// the agent you were trying to watch, or closing it and having nothing on screen
// to say a merge was still open in your checkout (AGE-273).
//
// `own` is false when this panel shows the project's checkout rather than the
// run's worktree. The buttons act through that run's window and its tabs, which
// only exist with the run focused, so there the card offers the way to it.
export default function MergeCard({
  run,
  own,
  selectedPath,
  onSelectFile,
  onChanged,
}: {
  run: RunInfo;
  own: boolean;
  selectedPath: string | null;
  // Null drops the selection, once the file it names has been settled.
  onSelectFile: (sel: { path: string; code?: string } | null) => void;
  onChanged: () => void;
}) {
  const { setApproveRun, setFocusedRun, setPendingSession, setTab, setView } = useRuns();
  const flow = useMergeFlows().get(run.id) ?? null;
  const hand = useMergeHandoff(run.id);
  const [state, setState] = useState<MergeState | null>(null);
  const [base, setBase] = useState("");
  // The conflicted rows' porcelain letters, read from the checkout the merge is
  // in. This panel's own status is the worktree's, which has no such rows.
  const [codes, setCodes] = useState<Record<string, string>>({});
  const [confirmAbort, setConfirmAbort] = useState(false);
  const [aborting, setAborting] = useState(false);
  const [abortStep, setAbortStep] = useState<CloneProgress | null>(null);
  const [abortError, setAbortError] = useState("");

  // The resolver is the agent, or you by hand in the checkout, so the only way
  // to learn of a resolution is to look. A failed read keeps the last answer: a
  // blip here (the index locked while the agent commits, say) clears itself on
  // the next tick.
  useEffect(() => {
    let live = true;
    const poll = () => {
      const asked = mergeFlow(run.id);
      mergeStatus(run.id)
        .then((s) => {
          // The window changed the flow while this was out, so this answer may
          // predate it. Read before a merge began and landed after, it said
          // "no merge" and ended the entry the conflict had just created, taking
          // the remote-branch answers with it.
          if (!live || mergeFlow(run.id) !== asked) return;
          const step = reconcileFlow(asked, s);
          if (step === "start") patchMergeFlow(run.id, {});
          if (step === "end") endMergeFlow(run.id);
          setState(s);
        })
        .catch(() => {});
    };
    poll();
    const timer = window.setInterval(poll, 3000);
    return () => { live = false; window.clearInterval(timer); };
  }, [run.id]);

  const phase = mergeCardPhase(flow, state);
  const unresolved = state?.unresolved ?? [];
  const unresolvedKey = unresolved.join("\n");

  useEffect(() => {
    if (phase === "hidden") return;
    let live = true;
    mergePreview(run.id).then((p) => { if (live) setBase(p.base); }).catch(() => {});
    return () => { live = false; };
  }, [phase === "hidden", run.id]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!unresolvedKey) return;
    let live = true;
    gitStatus(`project:${run.projectId}`)
      .then((rows) => {
        if (live) setCodes(Object.fromEntries(rows.map((r) => [r.path, `${r.index}${r.worktree}`])));
      })
      .catch(() => {});
    return () => { live = false; };
  }, [unresolvedKey, run.projectId]);

  // A file settled, by you or by the agent, is no longer a conflict to look at.
  // The pane would otherwise sit on "Mark it resolved" for what just was.
  const stale = !!state && !!selectedPath && !unresolved.includes(selectedPath);
  useEffect(() => {
    if (stale) onSelectFile(null);
  }, [stale]); // eslint-disable-line react-hooks/exhaustive-deps

  async function abort() {
    setAborting(true);
    setAbortStep(null);
    setAbortError("");
    try {
      await abortMergeTask(run.id, setAbortStep);
      endMergeFlow(run.id);
      onChanged();
    } catch (e) {
      setAbortError(String(e));
    } finally {
      setAborting(false);
      setAbortStep(null);
      setConfirmAbort(false);
    }
  }

  // The window is what offers the cleanup that follows a landed merge, so it is
  // the window that lands it, opening straight on that work.
  function finish() {
    patchMergeFlow(run.id, { finishOnOpen: true });
    setApproveRun(run.id);
  }

  function goToAgent(session?: string) {
    setFocusedRun(run.id);
    if (session) setPendingSession(session);
    setTab("agents");
    setView("focus");
  }

  if (phase === "hidden") return null;
  const ready = phase === "ready";
  const handoff = hand.handoff;

  return (
    <section className={`merge-card ${phase}`} aria-label="Merge in progress">
      <div className="merge-card-head">
        <span className="merge-card-glyph" aria-hidden>{ready ? "✓" : "⇋"}</span>
        <span className="merge-card-title">{ready ? "Ready to finish" : "Merge conflicts"}</span>
        {!ready && <span className="merge-card-count">{unresolved.length}</span>}
      </div>
      <div className="merge-card-route" title={`${run.branch} into ${base}`}>
        <code>{run.branch}</code>
        {base && <span className="merge-card-into"> → <code>{base}</code></span>}
      </div>

      {ready ? (
        <p className="merge-card-note">
          {state?.merging
            ? "No conflicts remain. Finishing commits the merge and puts your checkout back."
            : "The agent committed the merge. Finishing records it and puts your checkout back."}
        </p>
      ) : (
        <>
          {handoff ? (
            <p className="merge-card-note">
              <span className="merge-card-who">{handoffNote(handoff)}</span>{" "}
              {own && (
                <button className="hush-link" onClick={() => goToAgent(handoff.session)}>Open tab</button>
              )}
              {/* A tab created a moment ago can open on the agent's own first-run
                  gate rather than on the prompt it was launched with. All three
                  of the agents checked ask before reading their argv: cursor-agent
                  draws "Workspace Trust Required", copilot "Do you trust the files
                  in this folder?", and Claude Code "Is this a project you created
                  or one you trust?" (verified in a fresh worktree against
                  cursor-agent 2026.09.02, copilot 1.0.78 and Claude Code). Only for
                  a tab this flow opened: anything already in the strip is long past
                  its own first run (AGE-199). */}
              {handoff.spawned && (
                <span className="merge-card-hint">
                  If nothing starts, check the tab: a new agent can stop on a first-run question,
                  like whether it trusts this folder.
                </span>
              )}
            </p>
          ) : (
            <p className="merge-card-note">
              {own
                ? "Hand these to an agent, or pick a file to resolve it yourself. They're in your project's checkout, not this agent's worktree."
                : `Pick a file to resolve it yourself, or go to the agent that started this merge to hand them over.`}
            </p>
          )}
          <ul className="merge-card-files">
            {unresolved.map((f) => {
              const slash = f.lastIndexOf("/");
              return (
                <li key={f}>
                  <button
                    className={`merge-card-file${f === selectedPath ? " on" : ""}`}
                    title={f}
                    onClick={() => onSelectFile({ path: f, code: codes[f] })}
                  >
                    <span className="merge-card-name">{f.slice(slash + 1)}</span>
                    {slash > 0 && <span className="merge-card-dir">{f.slice(0, slash)}</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      )}

      {(hand.error || abortError) && <div className="merge-card-error">{hand.error || abortError}</div>}

      {own ? (
        <div className="merge-card-actions">
          {!ready && hand.target && (
            <PillSelect
              value={hand.target}
              options={hand.targets}
              onChange={hand.setTarget}
              title="Which agent in this workspace gets the conflict"
            />
          )}
          {ready ? (
            <button className="btn-primary" disabled={aborting} onClick={finish}>Finish merge</button>
          ) : (
            <button
              className={hand.sent ? "ghost" : "btn-primary"}
              disabled={hand.sending || !hand.target}
              onClick={() => { void hand.send(); }}
            >
              {hand.sending ? hand.sendingLabel : hand.sent ? "Send again" : hand.sendLabel}
            </button>
          )}
          <button
            className="ghost merge-card-abort"
            disabled={aborting}
            title="Undo the merge and put the project's checkout back"
            onClick={() => { setAbortError(""); setConfirmAbort(true); }}
          >
            Abort
          </button>
        </div>
      ) : (
        <div className="merge-card-actions">
          <button className="ghost" onClick={() => goToAgent()}>Go to agent</button>
        </div>
      )}

      {/* A confirm the window never needed: a card that sits beside your work
          all day is much easier to hit by accident than a button in a dialog you
          opened on purpose, and this one throws away a finished resolution. */}
      {confirmAbort && (
        <ConfirmDialog
          title="Abort this merge?"
          body="Your project's checkout goes back to where it was, and any conflicts resolved so far are lost. The branch keeps its commits, so you can merge again later."
          confirmLabel="Abort merge"
          danger
          busy={aborting}
          progress={abortStep}
          progressLabel="Undoing the merge…"
          onConfirm={abort}
          onCancel={() => setConfirmAbort(false)}
        />
      )}
    </section>
  );
}
