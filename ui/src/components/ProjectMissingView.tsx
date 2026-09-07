import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CloneProgress, Project, closeProject, folderMissing, relocateProject } from "../api";
import { toastError, toastInfo } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";

/**
 * What a project shows once its source folder has gone (AGE-203): moved,
 * renamed, deleted, or on a volume that is no longer mounted.
 *
 * It replaces the tabs rather than sitting alongside them. Every one of them
 * reads the folder, so with it gone the Agents grid, Source Control, Files,
 * Docs and Run each failed on their own timer and in their own words, and none
 * of them said the one thing that was actually wrong or offered the two things
 * that fix it.
 */
export default function ProjectMissingView({
  project,
  onReconnected,
  onRemoved,
}: {
  project: Project;
  /** The folder was found again; the caller re-reads the project and its tabs. */
  onReconnected: (project: Project) => void;
  /** The project was closed; the caller drops back to the overview. */
  onRemoved: () => void;
}) {
  // The workspace is pinned: Settings hides it rather than closing it, and
  // ProjectTree's menus leave "Close project…" off it for the same reason.
  // Offering it here took the ◈ row away for a workspace whose disk was merely
  // unmounted, and the only way back, "Create your workspace", repoints the row
  // at the default location: the recorded path of the folder the user was about
  // to reconnect is then gone.
  const removable = project.kind !== "workspace";
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [progress, setProgress] = useState<CloneProgress | null>(null);

  // Point the project at wherever the folder went. Its id is unchanged, so the
  // agents, issues, notes and settings kept against it all come back with it.
  async function locate() {
    const sel = await open({
      directory: true,
      multiple: false,
      title: `Locate the folder for "${project.name}"`,
    });
    if (typeof sel !== "string") return;
    setBusy(true);
    try {
      onReconnected(await relocateProject(project.id, sel));
    } catch (e) {
      toastError(e, "Couldn't reconnect the project");
    } finally {
      setBusy(false);
    }
  }

  // The folder may simply have come back: a disk remounted, or a move undone.
  async function checkAgain() {
    setBusy(true);
    try {
      if (await folderMissing(project.repo_path)) {
        toastInfo("The folder still isn't there.");
      } else {
        onReconnected(project);
      }
    } catch (e) {
      toastError(e, "Couldn't check the folder");
    } finally {
      setBusy(false);
    }
  }

  // Close, never delete: "Delete worktrees & close" tears down each agent's
  // worktree on disk, and there is no disk here to tear anything down on. The
  // records are kept, and adding the folder back revives every one of them,
  // but only at the path recorded here: `Registry::add_project` looks a closed
  // project up with `WHERE repo_path = ?1 AND closed = 1`, so the folder this
  // screen is usually about, one that has *moved*, matches nothing when it is
  // added at its new home and comes back as an empty project instead. The
  // dialog says so, and points at Locate, which keeps the project's id.
  async function remove() {
    setBusy(true);
    setProgress(null);
    try {
      await closeProject(project.id, setProgress);
      onRemoved();
    } catch (e) {
      toastError(e, "Couldn't remove the project");
    } finally {
      setBusy(false);
      setProgress(null);
      setConfirmRemove(false);
    }
  }

  return (
    <div className="board empty project-missing">
      <div className="project-missing-icon" aria-hidden>◇</div>
      <h2>This project's folder is missing</h2>
      <div className="project-missing-path" title={project.repo_path}>
        {project.repo_path}
      </div>
      <p>
        Agency can't find it. The folder may have been moved or renamed, it may have been
        deleted, or it may be on a disk that isn't connected. Nothing has been changed here:
        your agents, issues and notes are kept, and they come back with the folder.
      </p>
      <div className="project-missing-actions">
        <button className="btn-primary" disabled={busy} onClick={() => void locate()}>
          Locate folder…
        </button>
        <button className="ghost" disabled={busy} onClick={() => void checkAgain()}>
          Check again
        </button>
        {removable && (
          <button className="ghost" disabled={busy} onClick={() => setConfirmRemove(true)}>
            Remove project…
          </button>
        )}
      </div>
      {confirmRemove && (
        <ConfirmDialog
          title="Remove project?"
          body={(
            <>
              Stop everything running in "{project.name}" and take it out of the sidebar. Its
              agents, issues and notes are kept: add the folder again at{" "}
              <code>{project.repo_path}</code> and they all come back. If it has moved somewhere
              else, use "Locate folder…" instead; added at a new path it comes back as a new,
              empty project.
            </>
          )}
          confirmLabel="Remove project"
          busy={busy}
          progress={progress}
          progressLabel="Removing…"
          onConfirm={() => void remove()}
          onCancel={() => setConfirmRemove(false)}
        />
      )}
    </div>
  );
}
