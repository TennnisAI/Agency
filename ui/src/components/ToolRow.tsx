import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { InstallJob, ToolStatus } from "../api";
import { toastError, toastInfo } from "../lib/toast";

/**
 * One tool (git, npm, gh): whether it is here, and the way to get it if not.
 * Shared by the onboarding Tools step and the dialog an error raises, so the
 * two never disagree about what a machine can do.
 *
 * The install runs in the background on a click; the command it will run is
 * printed beside the button first, so the click is the confirmation. A plan
 * the app cannot run unattended (no polkit, no Homebrew) shows the line to
 * copy and where to read more instead.
 */
export default function ToolRow({
  tool,
  job,
  onInstall,
}: {
  tool: ToolStatus;
  job: InstallJob | null;
  onInstall: () => void;
}) {
  const [showOutput, setShowOutput] = useState(false);
  const running = job?.state === "running";
  const queued = job?.state === "queued";
  const failed = job?.state === "failed";
  // A job that succeeded but left the tool missing is macOS's Command Line
  // Tools installer: the command returns once the window opens.
  const finishedButMissing = job?.state === "succeeded" && !tool.installed;

  async function copy(cmd: string) {
    try {
      await navigator.clipboard.writeText(cmd);
      toastInfo("Command copied");
    } catch (e) {
      toastError(e, "Couldn't copy to clipboard");
    }
  }

  return (
    <div className={`tool-row${tool.installed ? " is-ok" : ""}`}>
      <div className="tool-row-main">
        <div className="tool-row-head">
          <span className="tool-name">{tool.label}</span>
          {tool.installed ? (
            <span className="tool-state is-ok">Found</span>
          ) : queued ? (
            <span className="tool-state is-busy"><span className="spinner small" /> Waiting…</span>
          ) : running ? (
            <span className="tool-state is-busy"><span className="spinner small" /> Installing…</span>
          ) : failed ? (
            <span className="tool-state is-bad">Install failed</span>
          ) : finishedButMissing ? (
            <span className="tool-state is-busy"><span className="spinner small" /> Waiting for the installer…</span>
          ) : (
            <span className="tool-state">{tool.required ? "Not installed" : "Not installed, optional"}</span>
          )}
        </div>
        <p className="tool-why">{tool.why}</p>
        {tool.installed && tool.path && <code className="tool-cmd">{tool.path}</code>}
        {!tool.installed && tool.plan.kind === "run" && (
          <>
            <code className="tool-cmd" title="What Install runs">{tool.plan.command}</code>
            {(running || (!failed && tool.plan.note)) && tool.plan.note && (
              <p className="tool-note">{tool.plan.note}</p>
            )}
          </>
        )}
        {!tool.installed && tool.plan.kind === "manual" && (
          <>
            <p className="tool-note">{tool.plan.hint}</p>
            {tool.plan.command && <code className="tool-cmd">{tool.plan.command}</code>}
          </>
        )}
        {failed && job && (
          <>
            <button type="button" className="tool-link" onClick={() => setShowOutput((s) => !s)}>
              {showOutput ? "Hide the installer's output" : "Show the installer's output"}
            </button>
            {showOutput && <pre className="tool-output">{job.output || `Exited with code ${job.exitCode ?? "?"}`}</pre>}
          </>
        )}
      </div>
      <div className="tool-actions">
        {!tool.installed && tool.plan.kind === "run" && !running && !queued && !finishedButMissing && (
          <button type="button" className="btn-primary" onClick={onInstall}>
            {failed ? "Try again" : "Install"}
          </button>
        )}
        {!tool.installed && tool.plan.kind === "run" && failed && (
          <button type="button" className="btn-secondary" onClick={() => void copy((tool.plan as { command: string }).command)}>
            Copy command
          </button>
        )}
        {!tool.installed && tool.plan.kind === "manual" && tool.plan.command && (
          <button type="button" className="btn-primary" onClick={() => void copy((tool.plan as { command: string }).command)}>
            Copy command
          </button>
        )}
        {!tool.installed && tool.plan.kind === "manual" && (
          <button type="button" className="btn-secondary" onClick={() => openUrl((tool.plan as { url: string }).url).catch(() => {})}>
            Open the download page
          </button>
        )}
      </div>
    </div>
  );
}
