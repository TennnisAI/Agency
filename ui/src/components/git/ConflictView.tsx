import { useCallback, useEffect, useMemo, useState } from "react";
import { fileRootOf, gitStage, readFile, trashPath, writeFile } from "../../api";
import { Side, parseConflicts, resolveAll, resolveBlock } from "../../lib/conflictFile";

// Resolving a merge conflict by hand, in the file rather than in a diff.
//
// The pane behind a row under "Merge Changes" used to be the ordinary diff
// viewer, and its stage/revert buttons were the only thing offered. But `git
// diff` answers for an unmerged path with a combined diff, whose lines are not
// the file's lines, so picking the side you wanted and staging it wrote
// `<<<<<<< HEAD` and the branch name into the file (AGE-199). The buttons here
// read the file, drop the markers and the side you did not pick, and write it
// back; `../../lib/conflictFile` is the whole of that transformation and is
// tested on its own.
export default function ConflictView({
  taskId, path, code, onChanged, onRevealInFiles,
}: {
  taskId: string;
  path: string;
  // The row's two porcelain status letters (`UU`, `UD`, …). Three of the seven
  // unmerged states are a delete against an edit, and those carry no markers
  // at all: what is on disk is simply the side that survived, and the choice
  // is whether to keep it. A string, not the pair as an object, so it can be a
  // dependency of the read below without changing identity every render.
  code?: string;
  // Re-read git's status: taking a side changes the file, staging changes the
  // row, and the merge window one tab over is watching for both.
  onChanged: () => void;
  onRevealInFiles?: (path: string) => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState("");
  // Why there is nothing to show, when the file is one this pane cannot edit.
  const [unreadable, setUnreadable] = useState("");
  const [busy, setBusy] = useState(false);
  const root = useMemo(() => fileRootOf(taskId), [taskId]);
  // One side of the merge deleted the file, which is why there is nothing in
  // it to pick between.
  const deleted = !!code && code.includes("D");

  const load = useCallback(async () => {
    // Every read starts from nothing known about this file: the pane is reused
    // as the selection moves between rows, and a stale "too large" or a stale
    // error would describe the file before this one.
    setUnreadable("");
    setError("");
    try {
      const f = await readFile(root, path);
      if (f.binary || f.tooLarge) {
        setText(null);
        setUnreadable(
          f.binary
            ? "Git can't merge this file's contents, so there are no sides to pick. Keep one version by checking it out in a terminal, then mark it resolved."
            : "This file is too large for Agency to open. Resolve it in your editor, then mark it resolved.",
        );
        return;
      }
      setText(f.text);
    } catch (e) {
      // Both sides deleted it (`DD`): there is no file to read, and that is
      // the state, not a failure. The buttons below still have something to
      // say about it.
      if (deleted) setText(null);
      else setError(String(e));
    }
  }, [root, path, deleted]);

  useEffect(() => { load(); }, [load]);

  const blocks = useMemo(() => (text == null ? [] : parseConflicts(text)), [text]);

  // Write a resolved copy of the file, having checked it is one: the
  // transformation is pure and tested, but this is a file the user owns, and
  // the one thing that must never happen here is markers left behind (or
  // multiplied) by a write nobody looked at.
  async function apply(next: string, expected: number) {
    if (text == null || busy) return;
    if (parseConflicts(next).length !== expected) {
      setError("Agency didn't rewrite the file: the result still holds conflicts it can't read.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      await writeFile(root, path, next);
      await load();
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const takeOne = (index: number, side: Side) =>
    text != null && apply(resolveBlock(text, index, side), blocks.length - 1);
  const takeAll = (side: Side) => text != null && apply(resolveAll(text, side), 0);

  async function markResolved() {
    setBusy(true);
    setError("");
    try {
      await gitStage(taskId, path);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // The other half of a delete-against-edit conflict: accept the deletion.
  // To the Trash rather than unlinked, like every other delete in Agency, and
  // then staged, which is how git is told the removal is the resolution.
  async function acceptDeletion() {
    setBusy(true);
    setError("");
    try {
      await trashPath(root, path);
      await gitStage(taskId, path);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // A `DD` conflict has no file left to read, so the only resolution is to
  // record the removal; every other delete conflict leaves the surviving side
  // on disk to keep or throw away.
  const onDisk = text != null || !!unreadable;

  // Markers are in the file but none of them parsed. Saying so is the honest
  // answer: this pane will not touch a file it cannot account for, and the
  // editor can.
  const tangled = text != null && blocks.length === 0 && text.includes("<<<<<<<");

  return (
    <div className="diffviewer">
      <div className="diff-toolbar">
        {onRevealInFiles ? (
          <button className="diff-path" title="Show this file in the Files tab"
            onClick={() => onRevealInFiles(path)}>{path}</button>
        ) : (
          <span className="diff-path">{path}</span>
        )}
        <span className="spacer" style={{ flex: 1 }} />
        {/* The file can be resolved elsewhere too (an agent, your editor, the
            Files tab), and this pane only reads it when it opens. */}
        <button className="git-iconbtn" disabled={busy} onClick={load} title="Re-read this file">
          Reload
        </button>
        {blocks.length > 1 && (
          <>
            <button className="git-iconbtn" disabled={busy} onClick={() => takeAll("current")}
              title="Take the checkout's side of every conflict in this file">
              Keep all current
            </button>
            <button className="git-iconbtn" disabled={busy} onClick={() => takeAll("incoming")}
              title="Take the incoming branch's side of every conflict in this file">
              Keep all incoming
            </button>
          </>
        )}
        <button className="git-iconbtn" disabled={busy || blocks.length > 0 || !!tangled || deleted}
          onClick={markResolved}
          title={blocks.length > 0
            ? "Pick a side for every conflict first"
            : "Stage this file, which is how git is told the conflict is settled"}>
          Mark resolved
        </button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="conflict-body">
        {unreadable && <p className="diff-empty">{unreadable}</p>}
        {tangled && (
          <p className="diff-empty">
            This file has conflict markers Agency can't read, so it won't rewrite them. Open it in
            the Files tab, resolve it there, then come back and mark it resolved.
          </p>
        )}
        {text != null && !tangled && blocks.length === 0 && !deleted && (
          <p className="diff-empty">
            No conflict markers left in this file. Mark it resolved to stage it, then finish the
            merge from the Approve window.
          </p>
        )}
        {blocks.length === 0 && deleted && (
          <>
            <p className="conflict-intro">
              {onDisk
                ? "One side of this merge deleted this file and the other changed it, so there are no sides to pick between. Keep the version that survived, or accept the deletion."
                : "Both sides of this merge deleted this file, so there is nothing to keep. Mark it resolved to record the removal."}
            </p>
            <div className="conflict-actions">
              <button className="git-iconbtn" disabled={busy} onClick={markResolved}>
                {onDisk ? "Keep the file" : "Mark resolved"}
              </button>
              {onDisk && (
                <button className="git-iconbtn" disabled={busy} onClick={acceptDeletion}
                  title="Move it to the Trash and stage the removal">
                  Delete the file
                </button>
              )}
            </div>
          </>
        )}
        {blocks.length > 0 && (
          <p className="conflict-intro">
            Pick a side for each conflict. Current is what the checkout already had, incoming is
            what the branch being merged brings. Agency rewrites the file and takes the markers
            with it.
          </p>
        )}
        {blocks.map((b, i) => (
          <div className="conflict-block" key={`${b.start}-${i}`}>
            <div className="conflict-head">
              <span className="conflict-count">Conflict {i + 1} of {blocks.length}</span>
              <span className="spacer" style={{ flex: 1 }} />
              <button className="git-iconbtn" disabled={busy} onClick={() => takeOne(i, "current")}>
                Keep current
              </button>
              <button className="git-iconbtn" disabled={busy} onClick={() => takeOne(i, "incoming")}>
                Keep incoming
              </button>
              <button className="git-iconbtn" disabled={busy} onClick={() => takeOne(i, "both")}
                title="Keep both sides, current first">
                Keep both
              </button>
            </div>
            <ConflictSide side="current" label={b.currentLabel} lines={b.current} />
            <ConflictSide side="incoming" label={b.incomingLabel} lines={b.incoming} />
          </div>
        ))}
      </div>
    </div>
  );
}

function ConflictSide({ side, label, lines }: { side: "current" | "incoming"; label: string; lines: string[] }) {
  return (
    <div className={`conflict-side ${side}`}>
      <div className="conflict-label">
        {side === "current" ? "Current" : "Incoming"}
        {label && <span className="conflict-ref">{label}</span>}
      </div>
      {lines.length === 0 ? (
        <div className="conflict-lines empty">(this side is empty here)</div>
      ) : (
        <pre className="conflict-lines">{lines.join("\n")}</pre>
      )}
    </div>
  );
}
