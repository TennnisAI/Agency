import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  UpdateCheck,
  UpdateProgress,
  checkForUpdate,
  installUpdate,
  lastUpdateCheck,
  restartApp,
  runningSessions,
} from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import { onMarkdownLinkClick, renderMarkdown } from "../lib/mdHtml";
import { toastSuccess } from "../lib/toast";
import { EMPTY_CHECK, Phase, afterUpdateChecked, headline, phaseFor, subhead } from "../lib/updatePhase";
import ModalBackdrop from "./ModalBackdrop";

/**
 * The update panel: what version is out, what changed, and the one button that
 * applies it.
 *
 * Opened by Help ▸ Check for Updates…, by the command palette, and by the
 * Settings ▸ Diagnostics row. Opening it with no `initial` runs a fresh check;
 * opening it from a check that has already happened shows that answer and does
 * not ask GitHub again.
 *
 * Installing and restarting are two presses, never one. Agents live in the
 * terminal daemon and normally survive a relaunch, but a release that changes
 * the daemon's protocol does not, so the moment Agency goes away has to be a
 * moment the user picked (AGE-229).
 *
 * Closing it never stops an install. The download runs in the backend, and any
 * dialog opened later picks it up from `installing` on the check.
 */

// Update dialogs currently mounted. App toasts an install's outcome only when
// there are none, since an open dialog already shows it.
let mounted = 0;
export function updateDialogOpen(): boolean {
  return mounted > 0;
}

export default function UpdateDialog({
  initial,
  onClose,
}: {
  /** A check that has already run, to show without asking again. */
  initial?: UpdateCheck | null;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<Phase>(initial ? phaseFor(initial) : { kind: "checking" });
  // Read when the restart is offered, not on mount: the download takes long
  // enough that a count from before it would be describing a different moment.
  // null until it arrives, so the prompt never claims "nothing is running"
  // before it knows.
  const [sessions, setSessions] = useState<number | null>(null);
  // Closable in every phase, the download included. It used to lock while
  // downloading, and the download had no timeout, so a stalled connection left
  // a dialog nobody could dismiss over the whole app; found in review.
  useModalKeys(onClose);

  useEffect(() => {
    mounted += 1;
    return () => { mounted -= 1; };
  }, []);

  // Bumped by "Try again", which is what re-runs the effect below. Without a
  // counter the effect's only dependency is `initial`, which does not change,
  // so retrying left the dialog spinning on a check nothing had started.
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (initial && attempt === 0) return;
    let live = true;
    setPhase({ kind: "checking" });
    checkForUpdate()
      .then((check) => {
        if (live) setPhase(phaseFor(check));
      })
      .catch((e) => {
        if (!live) return;
        // The command itself failing (not the network) is rare enough to have
        // no shaped state: report it in the same place the feed's own errors go.
        setPhase({
          kind: "failed",
          check: { ...EMPTY_CHECK, error: String(e) },
          message: String(e),
        });
      });
    return () => { live = false; };
  }, [initial, attempt]);

  useEffect(() => {
    if (phase.kind !== "installed") return;
    let live = true;
    runningSessions()
      .then((n) => { if (live) setSessions(n); })
      .catch(() => {});
    return () => { live = false; };
  }, [phase.kind]);

  useEffect(() => {
    // Download progress arrives as an event because the install command does
    // not return until the whole thing is on disk.
    const progress = listen<UpdateProgress>("update-progress", (e) => {
      setPhase((p) => (p.kind === "installing" ? { ...p, progress: e.payload } : p));
    });
    // The backend announces an install's start and its end. A dialog that did
    // not start the install, or was reopened on one, learns the outcome here.
    const checked = listen<UpdateCheck>("update-checked", (e) => {
      setPhase((p) => afterUpdateChecked(p, e.payload));
    });
    return () => {
      progress.then((un) => un()).catch(() => {});
      checked.then((un) => un()).catch(() => {});
    };
  }, []);

  async function install(check: UpdateCheck) {
    setPhase({ kind: "installing", check, progress: null });
    try {
      const version = await installUpdate();
      setPhase({ kind: "installed", version });
    } catch (e) {
      // Refused because another dialog's install is already running: follow
      // that one rather than report a failure that did not happen.
      const now = await lastUpdateCheck().catch(() => null);
      setPhase(now?.installing ? phaseFor(now) : { kind: "failed", check, message: String(e) });
    }
  }

  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div
        className="modal modal-update"
        role="dialog"
        aria-modal="true"
        aria-label="Software update"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <div className="modal-head-text">
            <h3>{headline(phase)}</h3>
            <p className="modal-sub">{subhead(phase, sessions)}</p>
          </div>
          <button type="button" className="modal-x" onClick={onClose}>✕</button>
        </div>
        <Body phase={phase} />
        <div className="modal-foot">
          <Actions
            phase={phase}
            onClose={onClose}
            onInstall={install}
            onRetry={() => setAttempt((n) => n + 1)}
          />
        </div>
      </div>
    </ModalBackdrop>
  );
}

function Body({ phase }: { phase: Phase }) {
  const notes = phase.kind === "report" ? phase.check.notes : null;
  const html = useMemo(() => (notes ? renderMarkdown(notes) : ""), [notes]);

  if (phase.kind === "installing") {
    const { downloaded, total } = phase.progress ?? { downloaded: 0, total: null };
    const pct = total ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
    return (
      <div className="modal-body">
        <div className="clone-progress">
          <div className="clone-progress-head">
            <span className="clone-progress-phase">
              {mib(downloaded)}{total ? ` of ${mib(total)}` : ""}
            </span>
            {pct !== null && <span className="clone-progress-pct">{pct}%</span>}
          </div>
          <div className="clone-progress-track">
            <div
              className={`clone-progress-bar${pct === null ? " indeterminate" : ""}`}
              style={pct === null ? undefined : { width: `${pct}%` }}
            />
          </div>
        </div>
      </div>
    );
  }

  if (phase.kind === "failed") {
    return (
      <div className="modal-body">
        <p className="settings-section-hint update-error">{phase.message}</p>
      </div>
    );
  }

  if (phase.kind !== "report") return null;

  const { check } = phase;
  if (check.error) {
    return (
      <div className="modal-body">
        <p className="settings-section-hint update-error">{check.error}</p>
      </div>
    );
  }

  if (!check.updateAvailable && !check.notes) return null;

  return (
    <div className="modal-body modal-update-body">
      {check.manualHint && (
        <div className="update-hint">
          <code className="update-hint-cmd">{check.manualHint}</code>
          <button
            type="button"
            className="settings-ghost-btn"
            onClick={() => {
              navigator.clipboard.writeText(check.manualHint!)
                .then(() => toastSuccess(`Copied: ${check.manualHint}`))
                .catch(() => {});
            }}
          >Copy</button>
        </div>
      )}
      {html && (
        <>
          <h4 className="update-notes-head">What's new</h4>
          <div className="md" onClick={onMarkdownLinkClick} dangerouslySetInnerHTML={{ __html: html }} />
        </>
      )}
    </div>
  );
}

function Actions({
  phase,
  onClose,
  onInstall,
  onRetry,
}: {
  phase: Phase;
  onClose: () => void;
  onInstall: (check: UpdateCheck) => void;
  onRetry: () => void;
}) {
  switch (phase.kind) {
    case "checking":
      return <button type="button" className="settings-ghost-btn" onClick={onClose}>Cancel</button>;

    case "installing":
      // No cancel: the plugin's download has no handle to stop it, and a button
      // that only looks like it stops something is worse than none. Hide says
      // what closing does; the Settings row and a toast carry the outcome.
      return <button type="button" className="settings-ghost-btn" onClick={onClose}>Hide</button>;

    case "installed":
      return (
        <>
          <button type="button" className="settings-ghost-btn" onClick={onClose}>Restart later</button>
          <button
            type="button"
            className="settings-save"
            onClick={() => { restartApp().catch(() => {}); }}
          >Restart now</button>
        </>
      );

    case "failed":
      return (
        <>
          <button type="button" className="settings-ghost-btn" onClick={onClose}>Close</button>
          <button
            type="button"
            className="settings-save"
            onClick={() => { openUrl(phase.check.url).catch(() => {}); }}
          >Open downloads</button>
        </>
      );

    case "report": {
      const { check } = phase;
      if (check.error) {
        return (
          <>
            <button type="button" className="settings-ghost-btn" onClick={onClose}>Close</button>
            <button type="button" className="settings-save" onClick={onRetry}>Try again</button>
          </>
        );
      }
      if (!check.updateAvailable) {
        return <button type="button" className="settings-save" onClick={onClose}>Close</button>;
      }
      return (
        <>
          <button type="button" className="settings-ghost-btn" onClick={onClose}>Not now</button>
          {check.canInstall ? (
            <button type="button" className="settings-save" onClick={() => onInstall(check)}>
              Install
            </button>
          ) : (
            <button
              type="button"
              className="settings-save"
              onClick={() => { openUrl(check.url).catch(() => {}); }}
            >Open downloads</button>
          )}
        </>
      );
    }
  }
}

/** Download sizes, to one decimal. Bytes are not a unit anyone reads. */
function mib(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
