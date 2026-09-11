import { useEffect, useRef } from "react";
import { useModalKeys } from "../hooks/useModalKeys";
import { useToolInstalls, jobFor } from "../hooks/useToolInstalls";
import { ToolId } from "../lib/missingTool";
import ModalBackdrop from "./ModalBackdrop";
import ToolRow from "./ToolRow";

/**
 * "This needs git, and git isn't here": the dialog an error opens instead of
 * a toast when the error says a tool is missing (lib/missingTool.ts). The
 * reason is the error's own sentence; the row under it is the same one
 * onboarding shows, install button and all. Closes itself once the tool
 * turns up, whether Agency installed it or the user did.
 */
export default function ToolInstallDialog({
  tool,
  reason,
  onClose,
}: {
  tool: ToolId;
  reason: string;
  onClose: () => void;
}) {
  const { tools, jobs, installTool } = useToolInstalls(true);
  const row = tools?.find((t) => t.id === tool) ?? null;

  useModalKeys(onClose);

  // Close once the tool turns up, and only then: on a false-to-true change
  // seen after mount. Closing on the first status too meant a dialog raised
  // by a stale error, or by a shell whose PATH lacks what the app's has,
  // flashed and vanished with the reason unread.
  const wasInstalled = useRef<boolean | null>(null);
  useEffect(() => {
    if (!row) return;
    if (wasInstalled.current === false && row.installed) onClose();
    wasInstalled.current = row.installed;
  }, [row, onClose]);

  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={`${row?.label ?? tool} is needed`} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{row ? `${row.label} isn't installed` : "A tool is missing"}</h3>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <div className="git-error">{reason}</div>
          {row ? (
            <ToolRow tool={row} job={jobFor(jobs, `tool:${row.id}`)} onInstall={() => void installTool(row.id)} />
          ) : (
            <p className="modal-note">Checking this machine…</p>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onClose}>Close</button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
