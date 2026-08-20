import { useEffect, useState } from "react";
import { MergeMethod, MergeMethods, PrMergeResult, mergePr, prMergeMethods } from "../../api";
import { useModalKeys } from "../../hooks/useModalKeys";
import ModalBackdrop from "../ModalBackdrop";

const METHOD_LABEL: Record<MergeMethod, string> = {
  merge: "Create a merge commit",
  squash: "Squash and merge",
  rebase: "Rebase and merge",
};

// Confirm + configure a GitHub PR merge (method + delete-branch). Only offers
// the merge methods the repo actually allows, fetched on open.
export default function MergePrDialog({
  projectId,
  number,
  title,
  onMerged,
  onCancel,
}: {
  projectId: string;
  number: number;
  title: string;
  // Carries the merge result so the host can note branch cleanup that didn't run.
  onMerged: (result: PrMergeResult) => void;
  onCancel: () => void;
}) {
  const [methods, setMethods] = useState<MergeMethods | null>(null);
  const [method, setMethod] = useState<MergeMethod>("merge");
  const [deleteBranch, setDeleteBranch] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useModalKeys(onCancel, !busy);

  useEffect(() => {
    prMergeMethods(projectId)
      .then((m) => {
        setMethods(m);
        // Default to the first allowed method, preferring a plain merge commit.
        setMethod(m.merge ? "merge" : m.squash ? "squash" : "rebase");
      })
      .catch((e) => setError(String(e)));
  }, [projectId]);

  const allowed: MergeMethod[] = methods
    ? (["merge", "squash", "rebase"] as MergeMethod[]).filter((k) => methods[k])
    : [];

  async function confirm() {
    setBusy(true);
    setError("");
    try {
      onMerged(await mergePr(projectId, number, method, deleteBranch));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onCancel}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label="Merge pull request" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>Merge pull request</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">Merge #{number} “{title}” into its base branch.</p>
          {error && <div className="git-error">{error}</div>}
          {methods === null && !error ? (
            <p className="modal-note"><span className="spinner" /> Checking merge options…</p>
          ) : allowed.length === 0 ? (
            <p className="modal-note">This repository has no enabled merge methods.</p>
          ) : (
            <div className="merge-methods">
              {allowed.map((k) => (
                <label key={k} className="merge-method">
                  <input type="radio" name="merge-method" checked={method === k} onChange={() => setMethod(k)} />
                  <span>{METHOD_LABEL[k]}</span>
                </label>
              ))}
              <label
                className="merge-method"
                title="Deletes the branch on GitHub. The agent\u2019s worktree and its local branch stay until you archive the agent, which removes both once the work is on the remote."
              >
                <input type="checkbox" checked={deleteBranch} onChange={(e) => setDeleteBranch(e.target.checked)} />
                <span>Delete remote branch after merge</span>
              </label>
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          <button className="btn-primary" disabled={busy || allowed.length === 0} onClick={confirm}>
            {busy ? "Merging…" : "Merge"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
