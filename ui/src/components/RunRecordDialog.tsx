import { useEffect, useState } from "react";
import { RunInfo, readRunRecord } from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import ModalBackdrop from "./ModalBackdrop";
import Markdown from "./Markdown";

/**
 * What an archived agent left behind, read from its record file.
 *
 * This is the answer to the part of archiving that used to need the branch: a
 * merged run's branch is deleted with its worktree, and this is where "what did
 * that agent actually do" now lives. The file is markdown in the project's own
 * `.agency/records/`, so it outlives Agency and can be read without it; this
 * dialog is a convenience, not the only way in.
 */
export default function RunRecordDialog({
  run,
  onClose,
}: {
  run: RunInfo;
  onClose: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState("");
  useModalKeys(onClose, true);

  useEffect(() => {
    let live = true;
    readRunRecord(run.id)
      .then((t) => live && setText(t ?? ""))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [run.id]);

  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div
        className="modal run-record"
        role="dialog"
        aria-modal="true"
        aria-label="Run record"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>{run.title || run.branch}</h3>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body run-record-body">
          {error ? (
            <p className="git-error">{error}</p>
          ) : text === null ? (
            <p>Reading…</p>
          ) : text === "" ? (
            // A run archived before records existed, or one whose project
            // folder has moved. Say which is missing rather than showing an
            // empty pane that reads as a failure.
            <p>
              No record for this run. It was archived before Agency kept them, or the project
              folder has moved since.
            </p>
          ) : (
            <Markdown text={text} />
          )}
        </div>
      </div>
    </ModalBackdrop>
  );
}
