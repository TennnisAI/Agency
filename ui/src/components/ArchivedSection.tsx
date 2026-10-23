import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArchiveHit,
  CloneProgress,
  RunInfo,
  listArchivedRuns,
  searchArchivedRuns,
  restoreRun,
  discardRun,
  discardArchivedRuns,
} from "../api";
import { useRuns } from "../store/runs";
import { restorable, restoreTitle } from "../lib/restore";
import { toastError, toastInfo, toastSuccess } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import RunRecordDialog from "./RunRecordDialog";
import { TrashIcon } from "./icons";

/**
 * The archive: runs that are finished with, and what is left of each one.
 *
 * An archived run never has a worktree — archiving removes it, and always did.
 * What it may still have is its branch, and only when that branch is the last
 * copy of some work; a merged run's branch goes with the archive, since it is a
 * second name for commits that are on the base.
 *
 * Both endings restore. Where the branch survived, the worktree goes back onto
 * it; where it did not, the branch is cut again from the base its work landed
 * on and the rescued conversation is reinstated, so the agent resumes on top of
 * what it merged. Which of the two is about to happen is in the Restore
 * tooltip; see `lib/restore.ts`. Only a run whose base is gone as well has
 * nothing to come back from.
 *
 * The search box looks through each run's summary, record and conversation,
 * on the backend: the conversations are transcripts the frontend never holds.
 * A row whose title does not show why it matched carries a snippet that does.
 */
/** The label a search snippet opens with, naming where its match was. */
const WHERE: Record<ArchiveHit["field"], string> = {
  summary: "Summary",
  record: "Record",
  conversation: "Conversation",
};

export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  // Null until the first list arrives. The list asks git about every run, so
  // on a big archive it takes a moment, and an empty array in the meantime
  // read as "Nothing archived." over an archive with hundreds of runs in it.
  const [items, setItems] = useState<RunInfo[] | null>(null);
  const [query, setQuery] = useState("");
  // The latest answer's matches by run id, with the query they answer; null
  // while no search has answered. Kept while a newer query is on its way, so
  // the list does not blank between keystrokes, but dimmed. Passed off as the
  // new query's answer, it said "No archived agent matches" the new text
  // before that text had been searched for.
  const [hits, setHits] = useState<{ q: string; byId: Map<string, ArchiveHit> } | null>(null);
  // Answers can land out of order while typing; only the latest one counts.
  const searchSeq = useRef(0);
  // Permanent deletion is gated behind a confirm; `busy` covers restore,
  // delete, and the bulk cleanup so a slow op can't be double-fired.
  const [confirmDelete, setConfirmDelete] = useState<RunInfo | null>(null);
  const [confirmCleanup, setConfirmCleanup] = useState(false);
  const [reading, setReading] = useState<{
    run: RunInfo;
    tab?: "record" | "conversation";
  } | null>(null);
  const [busy, setBusy] = useState(false);
  // Teardown step of the delete or sweep in flight. A whole project's worth of
  // archived runs is the slowest removal in the app, and it used to show
  // nothing at all until it finished.
  const [progress, setProgress] = useState<CloneProgress | null>(null);

  const load = useCallback(async () => {
    if (!selectedProjectId) return setItems([]);
    try {
      setItems(await listArchivedRuns(selectedProjectId));
    } catch {
      // Settle on what we had, or an empty list, rather than loading forever.
      setItems((prev) => prev ?? []);
    }
  }, [selectedProjectId]);

  // Another project's archive is not this one's: forget the list and the
  // search so neither is shown against the wrong project while it loads.
  useEffect(() => {
    setItems(null);
    setQuery("");
  }, [selectedProjectId]);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  // Debounced so a search runs once typing pauses, not per keystroke. Re-runs
  // when the list reloads, so a restore or delete drops out of the results.
  const q = query.trim();
  useEffect(() => {
    if (!open || !selectedProjectId || !q) {
      searchSeq.current++;
      setHits(null);
      return;
    }
    const seq = ++searchSeq.current;
    const t = window.setTimeout(async () => {
      try {
        const found = await searchArchivedRuns(selectedProjectId, q);
        if (seq === searchSeq.current) setHits({ q, byId: new Map(found.map((h) => [h.id, h])) });
      } catch (e) {
        if (seq === searchSeq.current) {
          setHits({ q, byId: new Map() });
          toastError(e, "Search failed");
        }
      }
    }, 200);
    return () => window.clearTimeout(t);
  }, [open, selectedProjectId, q, items]);

  if (!selectedProjectId) return null;

  // Resolves whether the run came back, so the record dialog knows to close
  // on success and stay open, still readable, on failure.
  const restore = async (r: RunInfo): Promise<boolean> => {
    if (busy) return false;
    setBusy(true);
    try {
      const restored = await restoreRun(r.id);
      await refreshRuns();
      await load();
      setFocusedRun(restored.run.id);
      if (restored.notice) toastInfo(restored.notice, 15000);
      return true;
    } catch (e) {
      toastError(e, "Restore failed");
      return false;
    } finally {
      setBusy(false);
    }
  };

  const remove = async (r: RunInfo) => {
    setBusy(true);
    setProgress(null);
    try {
      await discardRun(r.id, setProgress);
      await load();
      setConfirmDelete(null);
    } catch (e) {
      toastError(e, "Delete failed");
      setConfirmDelete(null);
    } finally {
      setBusy(false);
    }
  };

  // Delete every archived run at once. Partially successful sweeps are
  // reported rather than swallowed: some runs can go while another's branch
  // refuses to delete.
  const cleanUp = async () => {
    setBusy(true);
    setProgress(null);
    try {
      const { discarded, failed } = await discardArchivedRuns(selectedProjectId, setProgress);
      await load();
      setConfirmCleanup(false);
      if (discarded) toastSuccess(`Cleared ${discarded} archived agent${discarded === 1 ? "" : "s"}`);
      if (failed.length) toastError(failed.join("; "), "Some agents could not be cleared");
    } catch (e) {
      toastError(e, "Cleanup failed");
      setConfirmCleanup(false);
    } finally {
      setBusy(false);
    }
  };

  // How many of the archived runs are still holding a branch. That is the only
  // part of a sweep that can lose anything, so it is the part the confirm
  // counts rather than talking about all of them at once.
  const all = items ?? [];
  const holding = all.filter((r) => r.archived?.branchKept).length;
  // A query with no answer yet shows nothing rather than the whole list; one
  // whose answer is for an older query shows that answer, dimmed.
  const shown = !q ? all : hits ? all.filter((r) => hits.byId.has(r.id)) : [];
  const pending = !!q && hits?.q !== q;

  return (
    <div className="archived">
      <div className="archived-head">
        <button className="archived-toggle" onClick={() => setOpen((o) => !o)}>
          {open ? "▾" : "▸"} Archived{all.length ? ` (${all.length})` : ""}
        </button>
        {/* Bulk cleanup only while the whole list is on screen, so what the
            button is about to delete is all there next to it. A filtered list
            is not all of it, and Clear deletes all of it. */}
        {open && all.length > 0 && !q && (
          <button
            className="archived-cleanup"
            title="Delete every archived agent, its record, and any branch it still holds"
            disabled={busy}
            onClick={() => setConfirmCleanup(true)}
          >Clear</button>
        )}
      </div>
      {open && all.length > 0 && (
        <div className="archived-search">
          <input
            className="docs-search-input"
            type="search"
            placeholder="Search titles, records and conversations"
            aria-label="Search the archive"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query) {
                e.stopPropagation();
                setQuery("");
              }
            }}
          />
        </div>
      )}
      {open &&
        shown.map((r) => {
          const hit = q ? hits?.byId.get(r.id) : undefined;
          const readable = !!(r.archived?.hasRecord || r.archived?.hasConversation);
          return (
          <div key={r.id} className={pending ? "archived-stale" : undefined}>
            <div className="archived-row">
              {/* The row used to carry a "branch"/"record" chip here. On a
                  sidebar this narrow it cost more of the title than it was
                  worth, and every row after a merge said the same word —
                  "record" — so it distinguished nothing. What it was for now
                  lives in the Restore button's own tooltip, which is where the
                  distinction is acted on.

                  The name itself opens the record, for the same reason: the ≡
                  button that used to sit in the actions spent width on an
                  affordance the row already had. A run with neither a record
                  nor a rescued conversation has nothing to open, so it stays
                  plain text. */}
              {readable ? (
                <button
                  className="archived-name archived-name-open"
                  title="Read the record and the conversation"
                  disabled={busy}
                  onClick={() => setReading({ run: r })}
                >
                  {r.agent}: {r.prompt || r.branch}
                </button>
              ) : (
                <span className="archived-name" title={`${r.agent}: ${r.prompt || r.branch}`}>
                  {r.agent}: {r.prompt || r.branch}
                </span>
              )}
              <div className="archived-acts">
                <button
                  className="icon-btn"
                  title={restoreTitle(r)}
                  disabled={busy || !restorable(r)}
                  onClick={() => restore(r)}
                >↺</button>
                <button
                  className="icon-btn danger"
                  title="Delete permanently"
                  disabled={busy}
                  onClick={() => setConfirmDelete(r)}
                ><TrashIcon /></button>
              </div>
            </div>
            {/* Why a row matched, where its title does not say. Opens the
                dialog on the tab the match is in; a summary match is in
                neither, so the dialog picks its own, which for a run with no
                record is the conversation rather than an empty record pane. */}
            {hit?.snippet && (
              <button
                className="archived-snippet"
                disabled={busy || !readable}
                title={`Matched in the ${hit.field}`}
                onClick={() =>
                  setReading({ run: r, tab: hit.field === "summary" ? undefined : hit.field })
                }
              >
                <span className="archived-snippet-where">{WHERE[hit.field]}</span>{" "}
                {hit.snippet.before}
                <mark className="archived-hit">{hit.snippet.hit}</mark>
                {hit.snippet.after}
              </button>
            )}
          </div>
          );
        })}
      {open && items === null && <div className="archived-empty">Loading…</div>}
      {open && items?.length === 0 && <div className="archived-empty">Nothing archived.</div>}
      {open && all.length > 0 && pending && shown.length === 0 && (
        <div className="archived-empty">Searching…</div>
      )}
      {open && all.length > 0 && q && !pending && shown.length === 0 && (
        <div className="archived-empty">No archived agent matches “{q}”.</div>
      )}
      {reading && (
        <RunRecordDialog
          run={reading.run}
          initialTab={reading.tab}
          busy={busy}
          onClose={() => setReading(null)}
          onRestore={async () => {
            if (await restore(reading.run)) setReading(null);
          }}
        />
      )}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete archived agent?"
          body={
            confirmDelete.archived?.branchKept
              ? `Permanently delete this run, its record, and its ${confirmDelete.branch} branch, which is the only copy of the work on it.`
              : "Permanently delete this run and its record. It has no branch left to lose, so nothing else goes with it."
          }
          confirmLabel="Delete"
          danger={confirmDelete.archived?.branchKept ?? true}
          busy={busy}
          progress={progress}
          progressLabel="Deleting…"
          onConfirm={() => remove(confirmDelete)}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
      {confirmCleanup && (
        <ConfirmDialog
          title="Clear the archive?"
          body={
            <div className="removal-body">
              <p>
                {`Permanently delete ${all.length} archived agent${
                  all.length === 1 ? "" : "s"
                } and ${all.length === 1 ? "its record" : "their records"}.`}
              </p>
              {holding > 0 && (
                <p className="removal-warn">
                  {`${holding} of them still ${holding === 1 ? "holds a branch" : "hold branches"} that ${
                    holding === 1 ? "is" : "are"
                  } the only copy of unmerged work. That goes too.`}
                </p>
              )}
            </div>
          }
          confirmLabel="Clear"
          danger={holding > 0}
          busy={busy}
          progress={progress}
          progressLabel="Clearing…"
          onConfirm={cleanUp}
          onCancel={() => setConfirmCleanup(false)}
        />
      )}
    </div>
  );
}
