// The update dialog's state and the words it shows for it, kept apart from the
// component so the transitions and the copy are testable without Tauri.
import { UpdateCheck, UpdateProgress } from "../api";

/** Where the dialog is in the download → install → restart sequence. */
export type Phase =
  | { kind: "checking" }
  | { kind: "report"; check: UpdateCheck }
  | { kind: "installing"; check: UpdateCheck; progress: UpdateProgress | null }
  | { kind: "installed"; version: string }
  | { kind: "failed"; check: UpdateCheck; message: string };

/** Stand-in for a check that never completed, so the failed state still has
 *  the shape the rest of the dialog reads. */
export const EMPTY_CHECK: UpdateCheck = {
  current: "",
  latest: null,
  updateAvailable: false,
  url: "https://github.com/TennnisAI/Agency/releases/latest",
  notes: null,
  installKind: "unknown",
  canInstall: false,
  manualHint: null,
  staged: null,
  installing: false,
  installError: null,
  error: null,
};

/** The phase a dialog opens on for `check`. A download already under way
 *  opens on its progress: the dialog closes mid-download, and reopening it
 *  offered Install again, which the backend refuses. A release already staged
 *  opens on the restart prompt: offering Install again downloaded it twice. */
export function phaseFor(check: UpdateCheck): Phase {
  if (check.installing) return { kind: "installing", check, progress: null };
  return check.staged && !check.updateAvailable
    ? { kind: "installed", version: check.staged }
    : { kind: "report", check };
}

/** What an `update-checked` event does to an open dialog. The backend sends one
 *  when an install starts and when it ends, which is how a dialog that did not
 *  start the install (or was reopened on it) learns the outcome. */
export function afterUpdateChecked(phase: Phase, shown: UpdateCheck): Phase {
  switch (phase.kind) {
    case "report":
      // Another dialog started the install: follow it.
      return shown.installing ? { kind: "installing", check: shown, progress: null } : phase;
    case "installing":
      if (shown.installing) return phase;
      if (shown.installError) return { kind: "failed", check: phase.check, message: shown.installError };
      if (shown.staged) return { kind: "installed", version: shown.staged };
      return phase;
    default:
      return phase;
  }
}

export function headline(phase: Phase): string {
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

export function subhead(phase: Phase, sessions: number | null): string {
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
      return "Agency checks the download's signature before it puts anything in place. You can close this; the download carries on.";
    case "installed":
      if (sessions === null) return "Restart when it suits you.";
      return sessions > 0
        ? `Restart when it suits you. ${sessions} session${sessions === 1 ? "" : "s"} ${sessions === 1 ? "is" : "are"} running; they normally carry on through a restart, but a release that changes the terminal daemon will end them.`
        : "Restart when it suits you. Nothing is running.";
    case "failed":
      return "Nothing was changed. You can download the release and install it yourself.";
  }
}

/** Why the Install button is missing, in the user's own terms. */
export function installerNote(c: UpdateCheck): string {
  switch (c.installKind) {
    case "mac-read-only": return "This copy is running from the disk image or a read-only location, so Agency can't replace it. Move Agency into Applications, open it from there, and update again.";
    case "deb": return "This copy came from a .deb, so apt owns it and Agency must not write over it.";
    case "rpm": return "This copy came from an .rpm, so your package manager owns it and Agency must not write over it.";
    case "pacman": return "This copy was built from the agency-bin PKGBUILD, so pacman owns it and Agency must not write over it. Run this in packaging/aur/agency-bin in your Agency checkout.";
    default: return "This build isn't one Agency can replace, so install the new version yourself.";
  }
}
