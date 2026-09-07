import { useCallback, useEffect, useMemo, useState } from "react";
import { fileRootOf, gitStage, readFile, trashPath, writeFile } from "../../api";
import { Side, parseConflicts, resolveAll, resolveBlock } from "../../lib/conflictFile";

// The five unmerged states that carry no conflict markers, because one side has
// the file and the other has nothing to merge into it. `AA` and `UU` are the two
// that do, and they are the ones the block list is for.
//
// `AU` and `UA` were missing at first, when this was `code.includes("D")`. An
// add against nothing then fell through to "No conflict markers left in this
// file", which quietly means "keep this side" and offered no way to drop the
// added file — the mirror of the button the `D` states get.
const MARKERLESS = ["DD", "AU", "UD", "UA", "DU"];

// What the last completed read found, held with the path it came from. The pane
// is reused as the selection moves between rows, so the text is this file's text
// only if it was read from this file. This used to be a bare string, and file
// A's content stayed rendered with the keep-a-side buttons live until B's read
// came back; a click in that window wrote A's text to B's path, and the
// block-count check below could not see it, because A's rewritten text parses to
// exactly the expected number of blocks.
interface Loaded {
  path: string;
  // Null when there is nothing to edit: the file is gone, binary, or too large.
  text: string | null;
  // Why there is nothing to show, when the file is one this pane cannot edit.
  unreadable: string;
}

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
  // The row's two porcelain status letters (`UU`, `UD`, …). Five of the seven
  // unmerged states carry no markers at all: what is on disk is simply the side
  // that survived, and the choice is whether to keep it. A string, not the pair
  // as an object, so it can be a dependency of the read below without changing
  // identity every render.
  code?: string;
  // Re-read git's status: taking a side changes the file, staging changes the
  // row, and the merge window one tab over is watching for both.
  onChanged: () => void;
  onRevealInFiles?: (path: string) => void;
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const root = useMemo(() => fileRootOf(taskId), [taskId]);
  // Anything read from another row describes the row before this one, so it is
  // not this file's state and nothing below may act on it.
  const here = loaded?.path === path ? loaded : null;
  const text = here?.text ?? null;
  const unreadable = here?.unreadable ?? "";
  // One side of the merge has the file and the other has nothing, which is why
  // there is nothing in it to pick between.
  const markerless = !!code && MARKERLESS.includes(code);

  const load = useCallback(async () => {
    setError("");
    try {
      const f = await readFile(root, path);
      setLoaded({
        path,
        text: f.binary || f.tooLarge ? null : f.text,
        unreadable: f.binary
          ? "Git can't merge this file's contents, so there are no sides to pick. Keep one version by checking it out in a terminal, then mark it resolved."
          : f.tooLarge
            ? "This file is too large for Agency to open. Resolve it in your editor, then mark it resolved."
            : "",
      });
    } catch (e) {
      // Both sides deleted it (`DD`): there is no file to read, and that is the
      // state, not a failure. Every other unmerged state leaves a file on disk,
      // so a failure there is a real one — and either way the read has to land
      // as this file's result, or a failed Reload leaves the previous content
      // standing with the buttons live.
      setLoaded({ path, text: null, unreadable: "" });
      if (code !== "DD") setError(String(e));
    }
  }, [root, path, code]);

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
      // `next` is a whole-file rewrite of the text this pane read when it
      // opened, so writing it without looking would put that snapshot back over
      // anything that has happened since. "Fix with agent" hands this same
      // conflict to an agent working in this same checkout — the two features
      // are meant to be used together — and the Files tab, the user's editor
      // and a terminal all reach the file too. Writing blind would replace an
      // agent's finished resolution with the conflicted version it started
      // from, markers and all. The block-count check above cannot catch that:
      // it only says the text being written is well shaped, not that it is
      // still a rewrite of what is on disk.
      const fresh = await readFile(root, path);
      if (fresh.binary || fresh.tooLarge || fresh.text !== text) {
        await load();
        setError("This file changed on disk after Agency read it, so nothing was written. Check it over, then pick a side again.");
        return;
      }
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
  // record the removal; every other markerless state leaves a side on disk to
  // keep or throw away. Read off the completed load, never off a pending one:
  // opening a `UD` row used to flash "Both sides of this merge deleted this
  // file" for a file that is on disk, because the read had not come back yet.
  const onDisk = here != null && (text != null || !!unreadable);

  // Markers are in the file but none of them parsed. Saying so is the honest
  // answer: this pane will not touch a file it cannot account for, and the
  // editor can.
  const tangled = text != null && blocks.length === 0 && text.includes("<<<<<<<");

  // Which of the markerless states this is, in the user's terms.
  const markerlessIntro = code === "DD"
    ? "Both sides of this merge deleted this file, so there is nothing to keep. Mark it resolved to record the removal."
    : code === "AU" || code === "UA"
      ? "One side of this merge added this file and the other doesn't have it, so there are no sides to pick between. Keep it, or drop it."
      : "One side of this merge deleted this file and the other changed it, so there are no sides to pick between. Keep the version that survived, or accept the deletion.";

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
            Files tab), and this pane only reads it when it opens. A write
            re-reads first, so a stale pane refuses rather than reverts, but
            this is how you see the new state. */}
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
        <button className="git-iconbtn"
          disabled={busy || blocks.length > 0 || !!tangled || markerless}
          onClick={markResolved}
          title={blocks.length > 0
            ? "Pick a side for every conflict first"
            : "Stage this file, which is how git is told the conflict is settled"}>
          Mark resolved
        </button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="conflict-body">
        {/* Not for a markerless state: there the file's contents are beside the
            point, the choice is whether to keep the file at all, and the
            paragraph below already says there are no sides to pick. A binary
            `UD` used to render both, which read as two different explanations
            of the same row. */}
        {unreadable && !markerless && <p className="diff-empty">{unreadable}</p>}
        {tangled && (
          <p className="diff-empty">
            This file has conflict markers Agency can't read, so it won't rewrite them. Open it in
            the Files tab, resolve it there, then come back and mark it resolved.
          </p>
        )}
        {text != null && !tangled && blocks.length === 0 && !markerless && (
          <p className="diff-empty">
            No conflict markers left in this file. Mark it resolved to stage it, then finish the
            merge from the Approve window.
          </p>
        )}
        {here && blocks.length === 0 && markerless && (
          <>
            <p className="conflict-intro">{markerlessIntro}</p>
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
