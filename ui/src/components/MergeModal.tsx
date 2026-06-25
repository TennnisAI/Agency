import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { currentXtermTheme, minContrastRatio } from "../lib/themes";
import {
  MergeOutcome,
  MergePreview,
  abortMergeTask,
  mergePreview,
  mergeTask,
  resolveMerge,
  resolverResize,
  resolverStatus,
} from "../api";

export default function MergeModal({ taskId, onClose }: { taskId: string; onClose: () => void }) {
  const [preview, setPreview] = useState<MergePreview | null>(null);
  const [outcome, setOutcome] = useState<MergeOutcome | null>(null);
  const [merging, setMerging] = useState(false);
  const [error, setError] = useState("");
  const [resolving, setResolving] = useState(false);
  const [resolverDone, setResolverDone] = useState(false);
  const termRef = useRef<HTMLDivElement>(null);
  const termInstanceRef = useRef<Terminal | null>(null);
  const timerRef = useRef<ReturnType<typeof window.setInterval> | null>(null);
  const roRef = useRef<ResizeObserver | null>(null);

  // Load the preview first so we can explain what a merge would do instead of
  // silently running it the moment the modal opens.
  useEffect(() => {
    mergePreview(taskId).then(setPreview).catch((e) => setError(String(e)));
  }, [taskId]);

  async function attempt() {
    setError("");
    setMerging(true);
    try {
      setOutcome(await mergeTask(taskId));
    } catch (e) {
      setError(String(e));
    } finally {
      setMerging(false);
    }
  }

  function startResolver() {
    setResolving(true);
    setResolverDone(false);
    const term = new Terminal({ convertEol: true, fontSize: 12, theme: currentXtermTheme(), minimumContrastRatio: minContrastRatio() });
    termInstanceRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    // Fit the terminal to its container, then push the size to the resolver PTY
    // so its agent reflows. resolver_resize is a no-op until the resolver spawns,
    // so it's safe to call before/while resolveMerge starts it.
    const doFit = () => {
      try {
        fit.fit();
        if (term.cols > 0 && term.rows > 0) resolverResize(taskId, term.cols, term.rows).catch(() => {});
      } catch {
        /* not laid out */
      }
    };
    if (termRef.current) {
      term.open(termRef.current);
      requestAnimationFrame(doFit);
      const ro = new ResizeObserver(doFit);
      ro.observe(termRef.current);
      roRef.current = ro;
    }
    resolveMerge(taskId, "claude", (bytes) => term.write(bytes))
      .then(doFit)
      .catch((e) => setError(String(e)));
    const timer = window.setInterval(async () => {
      try {
        const s = await resolverStatus(taskId);
        if (s.state === "exited" || s.state === "crashed") {
          setResolverDone(true);
          window.clearInterval(timer);
          timerRef.current = null;
        }
      } catch {
        /* not started yet */
      }
    }, 1000);
    timerRef.current = timer;
  }

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
      roRef.current?.disconnect();
      termInstanceRef.current?.dispose();
    };
  }, []);

  const conflicts = outcome?.kind === "conflicts";
  const step = outcome?.kind === "clean"
    ? "done"
    : conflicts
      ? (resolverDone ? "done" : resolving ? "resolve" : "merge")
      : merging
        ? "merge"
        : "review";
  const steps: { key: string; label: string }[] = [
    { key: "review", label: "Review" },
    { key: "merge", label: "Merge" },
    { key: "resolve", label: "Resolve" },
    { key: "done", label: "Done" },
  ];

  const nothingToMerge = !!preview && preview.commitsAhead === 0;

  return (
    <div className="settings-overlay">
      <div className="merge-modal">
        <div className="settings-head">
          <h2>Approve &amp; merge</h2>
          <button onClick={onClose}>Close</button>
        </div>
        <div className="merge-timeline">
          {steps.map((s) => (
            <span key={s.key} className={`mt-step ${s.key === step ? "on" : ""}`}>{s.label}</span>
          ))}
        </div>
        {error && <div className="git-error">{error}</div>}

        {/* Review: explain what will happen before merging. */}
        {!outcome && !merging && (
          !preview && !error ? (
            <p>Checking…</p>
          ) : preview ? (
            <div className="merge-review">
              <p className="merge-summary">
                Merge <code>{preview.branch}</code> → <code>{preview.base}</code>
              </p>
              {nothingToMerge ? (
                <p className="merge-warn">
                  Nothing to merge — this agent has no committed changes on top of <code>{preview.base}</code>.
                </p>
              ) : (
                <p className="merge-note">
                  {preview.commitsAhead} commit{preview.commitsAhead === 1 ? "" : "s"} ahead of <code>{preview.base}</code>.
                </p>
              )}
              {preview.worktreeDirty && (
                <p className="merge-warn">
                  {preview.dirtyFiles.length} uncommitted change{preview.dirtyFiles.length === 1 ? "" : "s"} in this
                  agent's worktree {nothingToMerge ? "exist but aren't committed" : "won't be included"} — only committed
                  work is merged. Commit them in Source Control first to include them.
                </p>
              )}
              {!nothingToMerge && (
                <div className="git-actions">
                  <button onClick={attempt}>Merge into {preview.base}</button>
                  <button onClick={onClose}>Cancel</button>
                </div>
              )}
            </div>
          ) : null
        )}

        {merging && <p>Merging…</p>}

        {outcome?.kind === "clean" && (
          <div>
            <p className="merge-ok">✓ Merged cleanly into {preview?.base ?? "main"}.</p>
            <code>{outcome.commit.slice(0, 10)}</code>
            <div className="git-actions">
              <button onClick={onClose}>Done</button>
            </div>
          </div>
        )}

        {outcome?.kind === "conflicts" && (
          <div>
            <p className="merge-warn">Conflicts in {outcome.files.length} file(s):</p>
            <ul className="profile-list">
              {outcome.files.map((f) => (
                <li key={f}>
                  <code>{f}</code>
                </li>
              ))}
            </ul>
            {!resolving ? (
              <div className="git-actions">
                <button onClick={startResolver}>Resolve with agent</button>
                <button onClick={() => abortMergeTask(taskId).then(onClose)}>Abort merge</button>
              </div>
            ) : (
              <div className="resolver">
                <div className="terminal merge-term" ref={termRef} />
                <div className="git-actions">
                  <button disabled={!resolverDone} onClick={attempt}>
                    Re-check merge
                  </button>
                  <button onClick={() => abortMergeTask(taskId).then(onClose)}>Abort merge</button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
