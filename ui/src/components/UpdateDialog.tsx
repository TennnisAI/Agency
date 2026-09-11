import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  UpdateCheck,
  UpdateProgress,
  checkForUpdate,
  installUpdate,
  restartApp,
  runningSessions,
} from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import { onMarkdownLinkClick, renderMarkdown } from "../lib/mdHtml";
import { toastSuccess } from "../lib/toast";
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
 */

/** Where the dialog is in the download → install → restart sequence. */
type Phase =
  | { kind: "checking" }
  | { kind: "report"; check: UpdateCheck }
  | { kind: "installing"; check: UpdateCheck; progress: UpdateProgress | null }
  | { kind: "installed"; version: string }
  | { kind: "failed"; check: UpdateCheck; message: string };

export default function UpdateDialog({
  initial,
  onClose,
  onChecked,
}: {
  /** A check that has already run, to show without asking again. */
  initial?: UpdateCheck | null;
  onClose: () => void;
  /** Hands a fresh check back so the caller can keep its own dot in step. */
  onChecked?: (check: UpdateCheck) => void;
}) {
  const [phase, setPhase] = useState<Phase>(
    initial ? { kind: "report", check: initial } : { kind: "checking" },
  );
  // Read when the restart is offered, not on mount: the download takes long
  // enough that a count from before it would be describing a different moment.
  const [sessions, setSessions] = useState(0);
  const busy = phase.kind === "installing";
  useModalKeys(onClose, !busy);

  const checkedRef = useRef(onChecked);
  checkedRef.current = onChecked;

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
        if (!live) return;
        setPhase({ kind: "report", check });
        checkedRef.current?.(check);
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

  // Download progress arrives as an event because the install command does not
  // return until the whole thing is on disk.
  useEffect(() => {
    const sub = listen<UpdateProgress>("update-progress", (e) => {
      setPhase((p) => (p.kind === "installing" ? { ...p, progress: e.payload } : p));
    });
    return () => { sub.then((un) => un()).catch(() => {}); };
  }, []);

  async function install(check: UpdateCheck) {
    setPhase({ kind: "installing", check, progress: null });
    try {
      const version = await installUpdate();
      setSessions(await runningSessions().catch(() => 0));
      setPhase({ kind: "installed", version });
    } catch (e) {
      setPhase({ kind: "failed", check, message: String(e) });
    }
  }

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onClose}>
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
          {!busy && <button type="button" className="modal-x" onClick={onClose}>✕</button>}
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

/** Stand-in for a check that never completed, so the failed state still has
 *  the shape the rest of the dialog reads. */
const EMPTY_CHECK: UpdateCheck = {
  current: "",
  latest: null,
  updateAvailable: false,
  url: "https://github.com/TennnisAI/Agency/releases/latest",
  notes: null,
  installKind: "unknown",
  canInstall: false,
  manualHint: null,
  error: null,
};

function headline(phase: Phase): string {
  switch (phase.kind) {
    case "checking": return "Checking for updates";
    case "report":
      if (phase.check.error) return "Couldn't check for updates";
      return phase.check.updateAvailable ? `Agency ${phase.check.latest} is available` : "Agency is up to date";
    case "installing": return `Downloading Agency ${phase.check.latest}`;
    case "installed": return `Agency ${phase.version} is installed`;
    case "failed": return "The update didn't install";
  }
}

function subhead(phase: Phase, sessions: number): string {
  switch (phase.kind) {
    case "checking":
      return "Asking GitHub for the latest release.";
    case "report": {
      const c = phase.check;
      if (c.error) return "Agency couldn't reach the releases feed. Your network, or GitHub.";
      if (!c.updateAvailable) return `You're running ${c.current}, which is the newest release.`;
      if (c.canInstall) return `You're running ${c.current}. Agency can download and install this one for you.`;
      return `You're running ${c.current}. ${installerNote(c)}`;
    }
    case "installing":
      return "Agency checks the download's signature before it puts anything in place.";
    case "installed":
      return sessions > 0
        ? `Restart when it suits you. ${sessions} session${sessions === 1 ? "" : "s"} ${sessions === 1 ? "is" : "are"} running; they normally carry on through a restart, but a release that changes the terminal daemon will end them.`
        : "Restart when it suits you. Nothing is running.";
    case "failed":
      return "Nothing was changed. You can download the release and install it yourself.";
  }
}

/** Why the Install button is missing, in the user's own terms. */
function installerNote(c: UpdateCheck): string {
  switch (c.installKind) {
    case "deb": return "This copy came from a .deb, so apt owns it and Agency must not write over it.";
    case "rpm": return "This copy came from an .rpm, so your package manager owns it and Agency must not write over it.";
    case "pacman": return "This copy came from the AUR package, so pacman owns it and Agency must not write over it.";
    default: return "This build isn't one Agency can replace, so install the new version yourself.";
  }
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
      // that only looks like it stops something is worse than none.
      return <button type="button" className="settings-ghost-btn" disabled>Downloading…</button>;

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
