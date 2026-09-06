import { useEffect, useMemo, useRef, useState } from "react";
import {
  CloneProgress, FileRoot, Issue, IssuePatch, IssueStatus, IssueSyncMode, Project,
  addIssueComment, createIssue, deleteIssue, deleteIssueComment, getIssueSyncConfig, getWorkspace,
  syncIssues, updateIssue, updateIssueComment,
} from "../api";
import { planReorder } from "../lib/issueRank";
import { Corpus, IssueRef, LinkEdge, buildLinkIndex, mentionsOf, resolveTarget } from "../lib/links";
import { IssueLink, hasLink, issueLinks, linkCandidates, withLink, withoutLink } from "../lib/issueLinks";
import { requestNavigate } from "../lib/navigate";
import { useRuns, SpawnOpts } from "../store/runs";
import { useIssues } from "../hooks/useIssues";
import { useCrossRefs } from "../hooks/useCrossRefs";
import { useDocs } from "../hooks/useDocs";
import {
  DEFAULT_COLLAPSED,
  ISSUE_STATUSES,
  IssueSort,
  PENDING_ISSUE_KEY,
  PENDING_QUICKADD_KEY,
  PRIORITY_LABELS,
  STATUS_LABELS,
  StatusFilter,
  filtersActive,
  issueLabel,
  issueSorter,
  issuesCollapsedKey,
  issuesExpandedKey,
  issuesSelectedKey,
  loadCollapsed,
  loadSelected,
  matchesFilters,
  saveCollapsed,
  saveSelected,
  searchTerms,
  seedingRisk,
  stepSelection,
} from "../lib/issues";
import { autoSchedules, shouldSayKeyMismatch } from "../lib/autoSync";
import { ConflictReport, conflictReports } from "../lib/syncConflicts";
import { notifyProjectsChanged } from "../lib/projectEvents";
import { pickDefaultAgent } from "../lib/defaultAgent";
import { loadFold, saveFold, usePaneWidth } from "../hooks/usePaneWidth";
import { useRepoReadiness, isGitless } from "../hooks/useRepoReadiness";
import IssueRow from "./IssueRow";
import IssueDetail from "./IssueDetail";
import ConfirmDialog from "./ConfirmDialog";
import PillSelect from "./PillSelect";
import ProgressReadout from "./ProgressReadout";
import Resizer from "./Resizer";
import { toastError, toastInfo, toastSuccess } from "../lib/toast";
import { FindRank, registerFindTarget } from "../lib/findBus";

// How narrow the list may get once the detail pane takes over the view.
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 460;

// How often an automatically synced backlog fetches, while its board is on
// screen. Two minutes is short enough that a teammate moving an issue is news
// rather than history, and long enough that a day at the board is a couple of
// hundred passes rather than a couple of thousand.
const AUTO_SYNC_MS = 2 * 60 * 1000;

// The project's issue board: a status-grouped list (Linear's default view),
// quick capture on top, detail pane on the right. Dispatching an issue to an
// agent goes through `onStartIssue`, which runs the same installed/readiness
// checks as the "+ Agent" flow.
export default function IssuesView({
  project,
  onStartIssue,
  onOpenBacklogSettings,
}: {
  project: Project;
  onStartIssue: (issue: Issue, agentId: string, opts?: SpawnOpts) => Promise<void>;
  /** Settings, at the Backlog section: where sharing is turned on and pointed. */
  onOpenBacklogSettings: () => void;
}) {
  const { runs, tab, setTab, setView, setFocusedRun } = useRuns();
  const { issues, loaded, refresh } = useIssues(project.id, tab === "issues");
  // No repository here: an issue can still be dispatched (the agent works in
  // the folder), but racing or looping it needs branches, so those hide.
  const { readiness } = useRepoReadiness(project);
  const gitless = isGitless(readiness);
  const [quick, setQuick] = useState("");
  const [selectedId, setSelectedIdState] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<Issue | null>(null);
  // Backlog sharing. `syncOn` is null until the config lands, so the button
  // doesn't flash in and out on every project switch.
  const [syncOn, setSyncOn] = useState<boolean | null>(null);
  // The remote a pass would use, so the board can name it rather than saying
  // "the shared copy" at someone who is trying to work out where that is.
  const [syncRemote, setSyncRemote] = useState("");
  const [syncing, setSyncing] = useState(false);
  // Whether the pass in flight is one nobody asked for, which is the only thing
  // that decides if the progress readout is drawn.
  const [quietSync, setQuietSync] = useState(false);
  // The pass's current step. A sync fetches, reads one blob per issue per tree,
  // then pushes, so on a real backlog it is seconds; before this the window
  // simply stopped answering for that whole time.
  const [syncProgress, setSyncProgress] = useState<CloneProgress | null>(null);
  // Set when a first sync finds issues on both sides with no history in common;
  // holds the two counts the prompt states.
  const [seed, setSeed] = useState<{ local: number; remote: number } | null>(null);
  // Whether this project syncs on its own as well as on the button.
  const [autoOn, setAutoOn] = useState(false);
  // A pass is in flight. A ref and not `syncing`, because the automatic
  // schedule's timer holds the closure it was created with and would read a
  // `syncing` that is permanently false.
  const syncingRef = useRef(false);
  // A Sync pressed while a quiet pass held the lock. The buttons stay live
  // through a pass nobody asked for, so a press there is a real request rather
  // than a double-click, and dropping it left the one button the user reached
  // for doing nothing at all.
  const pendingManual = useRef<IssueSyncMode | null>(null);
  // What the last deciding sync of this project left behind: the row markers,
  // and the dialog an automatic pass owes the user. Seeded from a store that
  // outlives the board, because the board unmounts on every tab switch and the
  // prompt sends the user to look at the markers. See `lib/syncConflicts.ts`.
  const [conflicts, setConflicts] = useState<ConflictReport>(() =>
    conflictReports.forProject(project.id),
  );
  // Any status group folds; done/cancelled are the ones that start folded.
  const [collapsed, setCollapsed] = useState<Set<IssueStatus>>(() => new Set(DEFAULT_COLLAPSED));
  // Quick-add rests as a + button and expands into an inline input on demand.
  const [quickOpen, setQuickOpen] = useState(false);
  const quickRef = useRef<HTMLInputElement>(null);
  // Search and filters. Same controls as the cross-project home board, and
  // they live only for this visit — a hidden filter left over from last time
  // reads as a broken board.
  const [q, setQ] = useState("");
  const [fStatus, setFStatus] = useState<StatusFilter>("all");
  const [fPriority, setFPriority] = useState(-1);
  const [sort, setSort] = useState<IssueSort>("board");
  const searchRef = useRef<HTMLInputElement>(null);

  // ⌘F / Edit ▸ Find over a board means "narrow it", so it lands in the search
  // field rather than opening a text-scanning bar over a list of rows. The
  // field is the registered host, which self-gates: on an empty tracker there
  // is no filter bar, the ref is null, and ⌘F falls through to the description.
  // The registration is made once, so Find Next reads the current list through
  // a ref, the way the mouse handlers read the current grouping.
  const stepRef = useRef<(back: boolean) => void>(() => {});
  useEffect(() => registerFindTarget({
    host: () => searchRef.current,
    open: () => {
      searchRef.current?.focus();
      searchRef.current?.select();
    },
    step: (back) => stepRef.current(back),
    canReplace: false,
    rank: FindRank.list,
  }), []);

  function clearFilters() {
    setQ("");
    setFStatus("all");
    setFPriority(-1);
    setSort("board");
  }

  // Expanded mode: the detail pane takes over the view and the list compresses
  // to a sidebar. Remembered per project, so coming back to a tracker restores
  // the reading layout it was left in.
  const expandKey = issuesExpandedKey(project.id);
  const [expanded, setExpandedState] = useState(false);
  useEffect(() => {
    setExpandedState(typeof localStorage === "undefined" ? false : loadFold(localStorage, expandKey, false));
  }, [expandKey]);
  const setExpanded = (next: boolean) => {
    setExpandedState(next);
    try {
      if (typeof localStorage !== "undefined") saveFold(localStorage, expandKey, next);
    } catch {
      /* storage unavailable */
    }
  };
  // The compressed list's width is a plain machine-wide pane pref, like every
  // other resizable pane — only the expanded/contracted choice is per project.
  const sidebar = usePaneWidth("issues-sidebar", 288, SIDEBAR_MIN, SIDEBAR_MAX);

  // Folded groups are remembered per project too: which parts of the board
  // are worth seeing is a property of the tracker, not of this visit.
  const collapsedKey = issuesCollapsedKey(project.id);
  useEffect(() => {
    setCollapsed(
      typeof localStorage === "undefined" ? new Set(DEFAULT_COLLAPSED) : loadCollapsed(localStorage, collapsedKey),
    );
  }, [collapsedKey]);
  function toggleGroup(status: IssueStatus) {
    const next = new Set(collapsed);
    if (next.has(status)) next.delete(status); else next.add(status);
    setCollapsed(next);
    try {
      if (typeof localStorage !== "undefined") saveCollapsed(localStorage, collapsedKey, next);
    } catch {
      /* storage unavailable */
    }
  }

  // Which issue is open is remembered per project as well, so a trip to the
  // agents, a diff or a note and back lands on the ticket being read. Only
  // deliberate opens and closes are recorded — a project switch clears the
  // pane through the raw setter below, and must not erase the selection
  // stored for the project being switched to.
  const selectedKey = issuesSelectedKey(project.id);
  const setSelectedId = (id: string | null) => {
    setSelectedIdState(id);
    try {
      if (typeof localStorage !== "undefined") saveSelected(localStorage, selectedKey, id);
    } catch {
      /* storage unavailable */
    }
  };
  // Which project's stored selection has been restored already: without it,
  // a pane the user just closed would spring back open on the next poll.
  const restoredFor = useRef<string | null>(null);
  useEffect(() => {
    // On the commit where the project changes, the list is still the previous
    // project's, and an empty list says nothing about whose it is: restore
    // only against issues that are demonstrably this project's, so a stale
    // list can't burn the one restore this board gets.
    if (issues.length === 0 || issues[0].projectId !== project.id) return;
    if (restoredFor.current === project.id) return;
    restoredFor.current = project.id;
    // A handoff from elsewhere (the home overview, the palette) is fresher
    // intent than the stored selection; the effect below applies it.
    if (sessionStorage.getItem(PENDING_ISSUE_KEY)) return;
    let stored: string | null = null;
    try {
      if (typeof localStorage !== "undefined") stored = loadSelected(localStorage, selectedKey, issues);
    } catch {
      /* storage unavailable */
    }
    if (stored) setSelectedIdState(stored);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issues, project.id, selectedKey]);

  useEffect(() => { setSelectedIdState(null); setQuick(""); setQuickOpen(false); clearFilters(); }, [project.id]);
  useEffect(() => { if (quickOpen) quickRef.current?.focus(); }, [quickOpen]);

  // The palette's "New Issue" lands here: open quick-add on tab activation.
  useEffect(() => {
    if (tab !== "issues") return;
    if (sessionStorage.getItem(PENDING_QUICKADD_KEY)) {
      sessionStorage.removeItem(PENDING_QUICKADD_KEY);
      setQuickOpen(true);
      quickRef.current?.focus();
    }
  }, [tab]);

  // An issue clicked on the home overview arrives via sessionStorage (project
  // selection resets tab state, so props can't carry it); select it once the
  // list contains it.
  useEffect(() => {
    const pending = sessionStorage.getItem(PENDING_ISSUE_KEY);
    if (pending && issues.some((i) => i.id === pending)) {
      sessionStorage.removeItem(PENDING_ISSUE_KEY);
      setSelectedId(pending);
    }
  }, [issues]);

  const terms = useMemo(() => searchTerms(q), [q]);
  const filters = useMemo(
    () => ({ terms, status: fStatus, priority: fPriority }),
    [terms, fStatus, fPriority],
  );
  const filtered = filtersActive(filters);
  // Anything the user has touched up top, including a sort — what "Reset" and
  // the match count answer to.
  const narrowed = filtered || sort !== "board";
  // Manual order only means something over the whole, board-sorted group: a
  // rank computed from a filtered subset would reshuffle the hidden rows.
  const reorderable = !narrowed;
  const groups = useMemo(() => {
    const sorter = issueSorter(sort);
    return ISSUE_STATUSES.map((status) => ({
      status,
      issues: issues
        .filter((i) => i.status === status && matchesFilters(i, issueLabel(project, i), filters))
        .sort(sorter),
    }));
  }, [issues, filters, sort, project]);
  // Mouse handlers commit against the freshest grouping, not their closure's.
  const groupsRef = useRef(groups);
  groupsRef.current = groups;
  // A hit inside a folded group would otherwise be a match the board never
  // shows, so filtering opens every group.
  const groupOpen = (status: IssueStatus) => filtered || !collapsed.has(status);
  const visible = useMemo(
    () => groups.flatMap((g) => (groupOpen(g.status) ? g.issues : [])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [groups, collapsed, filtered],
  );
  const matched = groups.reduce((n, g) => n + g.issues.length, 0);
  // ⌘G / ⇧⌘G walk the filtered board. Focus stays wherever it was, including
  // in the filter field, so the keys read as "show me the next one" without
  // taking the field away mid-search.
  stepRef.current = (back: boolean) => {
    const next = stepSelection(visible, selectedId, back);
    if (next) setSelectedId(next);
  };
  // A selection the keyboard moved has to come into view with it: a ⌘G that
  // lands on a row below the fold looks like the same dead key it used to be.
  // `nearest` leaves a row that is already on screen exactly where it is, so
  // clicking a row never scrolls the board under the click.
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    listRef.current?.querySelector(".issue-row.selected")?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);
  const selected = issues.find((i) => i.id === selectedId) ?? null;
  const runsFor = (issue: Issue) => runs.filter((r) => r.issueId === issue.id);
  // Attachments are written beside the issue files, in the project's main
  // checkout — never a run's worktree.
  const fileRoot = useMemo<FileRoot>(() => ({ kind: "project", id: project.id }), [project.id]);

  // Mentions for the detail pane (one-stop Phase 7): notes and issues linking
  // to the selected issue. Corpora scanned: this project's docs plus the
  // workspace vault (where journal notes live) — other projects' docs are out
  // of scope until someone needs them.
  const [workspace, setWorkspace] = useState<Project | null>(null);
  useEffect(() => {
    let cancelled = false;
    getWorkspace().then((ws) => { if (!cancelled) setWorkspace(ws); }).catch(() => {});
    return () => { cancelled = true; };
  }, []);
  const wsId = workspace && workspace.id !== project.id ? workspace.id : null;
  const active = tab === "issues";
  const ownDocs = useDocs(project.id, active);
  const wsDocs = useDocs(wsId, active);
  const { cross, refresh: refreshCross } = useCrossRefs(active);
  const linkTable = useMemo(() => {
    if (!cross) return null;
    const corpora: Corpus[] = [];
    if (ownDocs.index) corpora.push({ project, index: ownDocs.index });
    if (workspace && wsId && wsDocs.index) corpora.push({ project: workspace, index: wsDocs.index });
    return buildLinkIndex(corpora, cross);
  }, [cross, ownDocs.index, wsDocs.index, project, workspace, wsId]);
  const mentions = useMemo(
    () => (linkTable && selected ? mentionsOf(linkTable, "issue", selected.id) : []),
    [linkTable, selected],
  );

  // The selected issue's `links:` relations, and what is left to link it to.
  // Both read from the cross-project rows, so an issue in another tracker is
  // as linkable as one in this project.
  const selectedLabel = selected ? issueLabel(project, selected) : "";
  const links = useMemo(
    () => (selected ? issueLinks(selected, selectedLabel, cross) : []),
    [selected, selectedLabel, cross],
  );
  const candidates = useMemo(
    () => linkCandidates(cross, project.id, selectedLabel, links),
    [cross, project.id, selectedLabel, links],
  );

  // Linking writes both sides: the relation is undirected, and an issue file
  // read on its own (by an agent, or in any markdown editor) should say what
  // it is linked to without having to grep the rest of the tracker. The other
  // side's row comes from the cross-project poll, so its refresh is what makes
  // the new link show up on the issue it was made from.
  async function addLink(ref: IssueRef) {
    if (!selected) return;
    const other = issueLabel(ref.project, ref.issue);
    try {
      await updateIssue(selected.id, { links: withLink(selected.links, other) });
      await updateIssue(ref.issue.id, { links: withLink(ref.issue.links, selectedLabel) });
      await Promise.all([refresh(), refreshCross()]);
    } catch (e) {
      toastError(e, `Couldn't link ${other}`);
    }
  }

  // Unlinking clears whichever sides hold it — including a link only the other
  // issue's file says (a hand edit, or a write that failed halfway).
  async function removeLink(link: IssueLink) {
    if (!selected) return;
    try {
      if (hasLink(selected.links, link.label)) {
        await updateIssue(selected.id, { links: withoutLink(selected.links, link.label) });
      }
      if (link.ref && hasLink(link.ref.issue.links, selectedLabel)) {
        await updateIssue(link.ref.issue.id, { links: withoutLink(link.ref.issue.links, selectedLabel) });
      }
      await Promise.all([refresh(), refreshCross()]);
    } catch (e) {
      toastError(e, `Couldn't unlink ${link.label}`);
    }
  }

  // Is this project's backlog shared? Decides whether the toolbar offers a
  // sync at all, and re-read per project rather than cached globally.
  // Re-read on every arrival at the tab, not only on a project switch: the
  // setting is changed in Settings, which leaves this component mounted, so
  // keying on the project alone would leave the button missing until a remount.
  useEffect(() => {
    if (tab !== "issues") return;
    let live = true;
    getIssueSyncConfig(project.id)
      .then((c) => {
        if (!live) return;
        setSyncOn(c.sync && c.remotes.length > 0);
        setSyncRemote(c.remote);
        setAutoOn(c.auto);
      })
      .catch(() => { if (live) { setSyncOn(false); setAutoOn(false); } });
    return () => { live = false; };
  }, [project.id, tab]);

  // A different project's conflicts say nothing about this one, so the board
  // shows that project's own outstanding report rather than clearing.
  useEffect(() => { setConflicts(conflictReports.forProject(project.id)); }, [project.id]);

  // This project's schedule record, created on first use and reset only when
  // the `auto` setting itself changes. Read through a call rather than held in
  // a ref so that a manual sync arriving before the schedule's own effect has
  // run still finds the same record.
  const sched = () => autoSchedules.forProject(project.id, autoOn);

  // A sync prompt is on screen. No pass may start while one is: the seeding
  // dialog goes `busy` on `syncing`, so a quiet pass would deaden its buttons,
  // its ✕ and Escape with no progress readout to explain why, and a second
  // conflict report would swap the list under the cursor and retarget its
  // "Open" button. A ref, because the schedule's timer cannot see state.
  const promptOpen = useRef(false);
  promptOpen.current = seed !== null || conflicts.prompt !== null;

  // The automatic schedule: a pass on arrival, then one every couple of minutes
  // for as long as the board is on screen. Nothing runs off-screen, and nothing
  // runs for a project that is not selected: a pass is two network round-trips
  // and a read of every issue in two trees, and spending that on boards nobody
  // is looking at is how a quiet feature becomes the reason the fans spin.
  // There is no `tab` check because the board is only mounted on its own tab.
  //
  // A rescheduling timeout and not an interval, so the window is measured from
  // the last pass rather than from this mount. `IssuesView` unmounts on every
  // tab switch, and an interval that started over each time it came back meant
  // arriving at the board was itself a way to force a pass.
  //
  // The interval is not a setting. Anything short enough to feel live is short
  // enough that the number stops mattering, and a knob here would only be a
  // place to get it wrong.
  useEffect(() => {
    if (!syncOn || !autoOn) return;
    let timer = 0;
    function tick() {
      const s = sched();
      const due = s.lastPassAt + AUTO_SYNC_MS - Date.now();
      if (due > 0) {
        timer = window.setTimeout(tick, due);
        return;
      }
      // `runSync` stamps `lastPassAt` for every pass it actually starts, the
      // manual ones included, so pressing Sync also pushes the window out
      // rather than leaving a scheduled pass due seconds later.
      if (!s.paused && !syncingRef.current && !promptOpen.current) void runSync("merge", true);
      timer = window.setTimeout(tick, AUTO_SYNC_MS);
    }
    tick();
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id, syncOn, autoOn]);

  // One sync pass. `merge` is the steady state; `publish`/`adopt` only ever
  // arrive from the seeding prompt, which is the one question the merge cannot
  // answer for itself.
  //
  // `auto` is the same pass with the reporting inverted. A pass the user asked
  // for says how it went, including "Already up to date", because they pressed
  // a button and are owed an answer. A pass on the schedule says nothing unless
  // there is something only a person can settle: a toast every couple of
  // minutes trains people to ignore the one that matters.
  async function runSync(mode: IssueSyncMode, auto = false) {
    const s = sched();
    if (syncingRef.current) {
      // Queued rather than dropped: the Sync buttons stay live through a quiet
      // pass, so this is someone asking for one, and the pass in flight is
      // seconds from letting go. The pass in flight stops being quiet at the
      // same time, or a press during a slow one would leave the button reading
      // "Sync" and apparently doing nothing until it finished. The readout has
      // a fallback for the progress this pass never reported.
      if (!auto) {
        pendingManual.current = mode;
        setQuietSync(false);
      }
      return;
    }
    syncingRef.current = true;
    if (!auto) {
      // A manual sync is the user saying they are dealing with whatever
      // stopped the schedule, so it starts the whole record over.
      s.paused = false;
      s.fails = 0;
      s.pushFailed = false;
    }
    s.lastPassAt = Date.now();
    setSyncing(true);
    setQuietSync(auto);
    setSyncProgress(null);
    try {
      // No readout for a pass nobody asked for. The progress bar exists to
      // explain a window that has gone quiet for several seconds, and on the
      // schedule it would instead be a bar that appears on its own.
      const res = await syncIssues(project.id, mode, auto ? () => {} : setSyncProgress);
      if (res.kind === "needsSeeding") {
        // Not a modal thrown up by a timer: the choice discards one side's
        // backlog, and it should be made by someone who just asked for a sync,
        // not by someone dismissing a box that appeared while they typed.
        if (auto) {
          s.paused = true;
          toastInfo(
            "Automatic sync is waiting on you: this machine's backlog and the shared copy " +
              "have nothing in common yet. Press Sync to choose which to keep.",
            10000,
          );
        } else {
          setSeed({ local: res.local, remote: res.remote });
        }
        return;
      }
      s.fails = 0;
      const o = res.outcome;
      await refresh();
      // A pass that merged issues this project's key does not cover has put
      // them on disk and left them off the board, and every count below is
      // about to report that as a clean sync. Said before the summary, because
      // it is the reason the summary looks the way it does.
      const km = res.keyMismatch;
      if (km?.adopted) {
        toastInfo(
          `This project now uses the shared backlog's issue key, ${km.theirs}.`,
          8000,
        );
        // The project row changed under every view that holds it; the board
        // labels issues with the key it was handed, and that key is now wrong.
        notifyProjectsChanged();
      }
      // Once per key on the schedule, every time by hand, for the reason the
      // push-failure rule below gives: this pass and every pass after it find
      // the same mismatch until the user changes the key.
      const theirs = km && !km.adopted ? km.theirs : null;
      if (shouldSayKeyMismatch(s, theirs, auto) && km) {
        const one = km.count === 1;
        toastError(
          `${km.count} issue${one ? "" : "s"} in the shared copy ${one ? "is" : "are"} keyed ` +
            `${km.theirs}, and this project uses ${km.ours}, so ${one ? "it is" : "they are"} not ` +
            `on this board. Set this project's issue key to ${km.theirs} in Settings to see ` +
            `${one ? "it" : "them"}.`,
          "Sync",
        );
      }
      // The rules for what a pass does to the markers and the prompt live in
      // the store, which is where they can be tested: a pass that decided
      // nothing leaves both alone, and only an automatic one raises a dialog.
      setConflicts(conflictReports.record(project.id, o.conflicts, { auto }));
      // Say what changed here, not what was uploaded: the push is the boring
      // half, and its failure is the only part of it worth a sentence.
      const bits: string[] = [];
      if (o.written) bits.push(`${o.written} issue${o.written === 1 ? "" : "s"} updated`);
      if (o.deleted) bits.push(`${o.deleted} removed`);
      if (o.assetsFetched) bits.push(`${o.assetsFetched} attachment${o.assetsFetched === 1 ? "" : "s"}`);
      if (o.conflicts.length) {
        bits.push(`${o.conflicts.length} conflict${o.conflicts.length === 1 ? "" : "s"} decided for you`);
      }
      const what = bits.length ? bits.join(", ") : "Already up to date";
      if (auto) {
        // A failed push on the schedule gets said once and then not again: the
        // board looks synced while the shared copy is not, which is the one
        // outcome silence would misrepresent, but a remote that refuses a push
        // refuses every push, and saying so every two minutes for the rest of
        // the day is the toast people learn to dismiss unread. Said again only
        // after a pass has succeeded in between.
        if (!o.pushed && !s.pushFailed) {
          toastError("Sync couldn't update the shared copy. Try again.", "Sync");
        }
        s.pushFailed = !o.pushed;
        return;
      }
      s.pushFailed = !o.pushed;
      if (!o.pushed) toastError(`${what}, but the shared copy was not updated. Try again.`, "Sync");
      else if (o.conflicts.length) toastInfo(what, 8000);
      else toastSuccess(what);
    } catch (e) {
      // On the schedule, the first failure is worth saying and the tenth is
      // not. Three in a row is a remote that is not coming back on its own
      // (no network, no auth, a URL that has moved), so the schedule stops and
      // says so rather than retrying until the app closes.
      if (auto) {
        s.fails += 1;
        if (s.fails === 1) toastError(e, "Couldn't sync the backlog");
        if (s.fails >= 3) {
          s.paused = true;
          toastError("Automatic sync gave up after three failures. Press Sync to try again.", "Sync");
        }
        return;
      }
      toastError(e, "Couldn't sync the backlog");
    } finally {
      syncingRef.current = false;
      setSyncing(false);
      setSyncProgress(null);
      const queued = pendingManual.current;
      pendingManual.current = null;
      if (queued) void runSync(queued);
    }
  }

  // The comment thread. Each call returns the issue as the file now has it,
  // but the board's own list is what the pane renders, so a refresh follows;
  // failures propagate so the section can keep the text that didn't post.
  async function comment(write: () => Promise<unknown>) {
    await write();
    await refresh();
  }

  // A wikilink ⌘-clicked in a description. Notes resolve against this project's
  // docs; issues and runs against every project. Nothing is created from here —
  // an issue is no place to be told "⌘-click to create a note".
  function followLink(target: string) {
    const res = resolveTarget(ownDocs.index, cross, target);
    switch (res.kind) {
      case "note":
        // A "#heading" suffix opens the note; scrolling to the heading is a
        // docs-editor trick that cross-tab navigation doesn't carry.
        requestNavigate({ kind: "note", projectId: project.id, path: res.path });
        return;
      case "issue":
        requestNavigate({ kind: "issue", projectId: res.ref.project.id, issueId: res.ref.issue.id });
        return;
      case "run":
        requestNavigate({ kind: "run", projectId: res.ref.project.id, runId: res.ref.run.id });
        return;
      case "unresolvedIssue":
        toastInfo(`No issue ${res.label} in any project.`);
        return;
      case "unresolvedRun":
        toastInfo("That run doesn't exist anymore.");
        return;
      case "unresolvedNote":
        toastInfo(`No note "${target}" in this project.`);
    }
  }

  function openMention(m: LinkEdge) {
    if (m.fromKind === "note") {
      requestNavigate({ kind: "note", projectId: m.fromProjectId, path: m.fromId });
    } else {
      requestNavigate({ kind: "issue", projectId: m.fromProjectId, issueId: m.fromId });
    }
  }

  async function add(status: "todo" | "backlog") {
    const title = quick.trim();
    if (!title) return;
    setQuick("");
    try {
      const issue = await createIssue(project.id, title, "", status);
      await refresh();
      setSelectedId(issue.id);
    } catch (e) {
      toastError(e, "Couldn't create issue");
    }
  }

  async function patch(issue: Issue, p: IssuePatch) {
    try {
      await updateIssue(issue.id, p);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't update issue");
    }
  }

  async function doDelete(issue: Issue) {
    setConfirmDelete(null);
    if (selectedId === issue.id) setSelectedId(null);
    try {
      await deleteIssue(issue.id);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't delete issue");
    }
  }

  // Manual reorder within a status group. Pointer-based (mousedown →
  // 5px threshold → track → commit on mouseup), NOT HTML5 drag-and-drop:
  // Tauri's native drag-drop layer intercepts drops at the NSView level on
  // macOS, so an in-page HTML5 drag lifts but its drop event never fires.
  // The live position is a plain ref (mouse events outrun React renders);
  // `drag` state mirrors it for the seam indicator. The drop maps to a rank
  // plan (materialize / midpoint / renormalize — see lib/issueRank).
  const [drag, setDrag] = useState<{ status: IssueStatus; from: number; to: number } | null>(null);
  const dragLive = useRef<{ status: IssueStatus; from: number; to: number } | null>(null);
  // A completed drag must not read as a click on the row it ends over.
  const suppressClick = useRef(false);

  async function dropReorder(group: Issue[], from: number, to: number) {
    const plan = planReorder(group.map((i) => ({ id: i.id, rank: i.rank })), from, to);
    if (plan.length === 0) return;
    try {
      await Promise.all(plan.map((u) => updateIssue(u.id, { rank: u.rank })));
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't reorder");
    }
  }

  function onRowMouseDown(status: IssueStatus, from: number, e: React.MouseEvent) {
    if (e.button !== 0) return;
    // Grabs must start on the row itself — not its buttons and menus.
    if ((e.target as HTMLElement).closest("button, input, textarea")) return;
    // Retire the focused editor by hand: preventDefault below suppresses the
    // blur that would otherwise commit an edit in the detail pane, and the
    // click that follows swaps the pane to another issue.
    // (The description is a CodeMirror contenteditable, not a textarea.)
    const focused = document.activeElement as HTMLElement | null;
    if (focused && (focused.tagName === "TEXTAREA" || focused.tagName === "INPUT" || focused.isContentEditable)) {
      focused.blur();
    }
    // Stop WebKit starting a text selection on the key/title — the selection
    // begins at mousedown, long before the drag threshold; the click that
    // selects the row is unaffected.
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY };
    // Set once the 5px threshold is crossed; drives click suppression on
    // release even after a cancel (the pointer is no longer "just clicking").
    let started = false;
    // Escape sets this so the drag can't silently restart on the next
    // mousemove; the still-held button then releases as a no-op.
    let cancelled = false;

    const onMove = (ev: MouseEvent) => {
      if (cancelled) return;
      if (!dragLive.current) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
        started = true;
        dragLive.current = { status, from, to: from };
        window.getSelection()?.removeAllRanges();
      }
      ev.preventDefault();
      const row = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)
        ?.closest<HTMLElement>("[data-issue-idx]");
      if (row && row.dataset.issueStatus === status) {
        dragLive.current = { ...dragLive.current, to: Number(row.dataset.issueIdx) };
      }
      setDrag({ ...dragLive.current });
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      cancelled = true;
      dragLive.current = null;
      setDrag(null);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey, true);
      const d = dragLive.current;
      dragLive.current = null;
      setDrag(null);
      if (started) {
        // Neither a completed nor a cancelled drag may read as a click on
        // the row the pointer ends over.
        suppressClick.current = true;
        window.setTimeout(() => { suppressClick.current = false; }, 0);
      }
      if (!d || cancelled) return;
      const group = groupsRef.current.find((g) => g.status === d.status)?.issues ?? [];
      dropReorder(group, d.from, d.to);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    // Capture phase so an Escape mid-drag can't reach anything else.
    window.addEventListener("keydown", onKey, true);
  }

  async function startDefault(issue: Issue) {
    const agent = await pickDefaultAgent(project.id, project.default_agent);
    await onStartIssue(issue, agent);
  }

  function openRun(runId: string) {
    setFocusedRun(runId);
    setView("focus");
    setTab("agents");
  }

  // Linear-flavored keys: `c` captures, `/` searches, arrows move, Enter
  // opens, 0-4 set priority. Skipped while typing in any field.
  useEffect(() => {
    if (tab !== "issues") return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "c") {
        e.preventDefault();
        setQuickOpen(true);
        quickRef.current?.focus();
      } else if (e.key === "/") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        if (visible.length === 0) return;
        e.preventDefault();
        const idx = visible.findIndex((i) => i.id === selectedId);
        const next = e.key === "ArrowDown"
          ? visible[Math.min(idx + 1, visible.length - 1)]
          : visible[Math.max(idx - 1, 0)];
        setSelectedId(next.id);
      } else if (e.key === "Enter" && selectedId == null && visible.length > 0) {
        e.preventDefault();
        setSelectedId(visible[0].id);
      } else if (/^[0-4]$/.test(e.key) && selected) {
        e.preventDefault();
        patch(selected, { priority: Number(e.key) });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, visible, selectedId, selected]);

  // The list only compresses while there is a detail pane to give the room to.
  const wide = expanded && selected != null;
  // A pass the user can see. The Sync buttons and the progress readout both key
  // off this and not `syncing`, or an automatically synced board would grey its
  // Sync button out and relabel it "Syncing…" on its own every couple of
  // minutes, which is the unrequested change the quiet pass exists to avoid. A
  // press during a quiet pass is queued, not swallowed; see `runSync`.
  const busySync = syncing && !quietSync;
  // The row an automatic conflict prompt offers to open: the first one it names
  // that is still on the board. A merge can report a conflict on an issue this
  // side then deletes, so the lookup can come back empty.
  const conflictTarget = conflicts.prompt
    ? issues.find((i) => issueLabel(project, i) === conflicts.prompt?.[0]?.key) ?? null
    : null;
  // One string for both Sync buttons (the toolbar's and the empty board's), so
  // the two cannot drift. It is the whole answer to "where did my issues go",
  // in the place someone asking that is already looking; the same claim the
  // Backlog section in Settings makes, in fewer words.
  const syncTitle =
    `Sync this backlog with ${syncRemote || "the shared copy"}. Issues travel on a ref of ` +
    "their own, refs/agency/issues, so they never land on a branch or in a diff." +
    // Someone pressing Sync on a backlog that already syncs itself is owed the
    // reason it was probably already up to date.
    (autoOn ? " This one also syncs on its own every couple of minutes." : "");

  return (
    <div className={`issues-wrap${wide ? " expanded" : ""}`}>
      <div className="issues-main" style={wide ? { width: sidebar.width, flex: "0 0 auto" } : undefined}>
        <div className={`issues-quickadd${quickOpen ? " open" : ""}`}>
          <button
            className="quickadd-toggle"
            title={quickOpen ? "Close" : "Add an issue (c)"}
            // Keep the press from blurring the input first — the blur handler
            // would collapse the box and this click would re-open it.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              if (quickOpen) {
                setQuick("");
                setQuickOpen(false);
                quickRef.current?.blur();
              } else {
                setQuickOpen(true);
              }
            }}
          >
            +
          </button>
          <input
            ref={quickRef}
            className="quickadd-input"
            placeholder="Add an issue…  (Enter → Todo, Shift+Enter → Backlog)"
            value={quick}
            tabIndex={quickOpen ? 0 : -1}
            onChange={(e) => setQuick(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") add(e.shiftKey ? "backlog" : "todo");
              if (e.key === "Escape") {
                setQuick("");
                setQuickOpen(false);
                (e.target as HTMLInputElement).blur();
              }
            }}
            onBlur={() => { if (!quick.trim()) setQuickOpen(false); }}
          />
        </div>
        {issues.length > 0 && (
          // Compressed to a sidebar there is no room for the pills, so the bar
          // keeps what a narrow list needs most — the search, and the way back
          // to the whole board if a filter is still on from before the expand.
          <div className={`issues-filterbar issues-toolbar${wide ? " compact" : ""}`}>
            <span className="filter-search">
              <span className="filter-search-glyph">⌕</span>
              <input
                ref={searchRef}
                placeholder="Search issues…"
                title="Search issues (/)"
                value={q}
                onChange={(e) => setQ(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Escape") return;
                  // First Escape empties a query, the next leaves the field.
                  if (q) setQ("");
                  else (e.target as HTMLInputElement).blur();
                }}
              />
              {q && <button className="filter-clear" title="Clear search" onClick={() => setQ("")}>✕</button>}
            </span>
            {narrowed && !wide && (
              <span className="filter-count">{matched} of {issues.length}</span>
            )}
            <div className="spacer" />
            {syncOn && (
              <button
                className="filter-reset"
                title={syncTitle}
                disabled={busySync}
                onClick={() => { runSync("merge"); }}
              >
                {busySync ? "Syncing…" : "Sync"}
              </button>
            )}
            {narrowed && (
              <button className="filter-reset" title="Clear search and filters" onClick={clearFilters}>
                Reset
              </button>
            )}
            {!wide && (
              <>
                <PillSelect<StatusFilter>
                  value={fStatus}
                  defaultValue="all"
                  title="Status"
                  onChange={setFStatus}
                  options={[
                    { value: "all", label: "All statuses" },
                    { value: "open", label: "Open" },
                    ...ISSUE_STATUSES.map((s) => ({ value: s, label: STATUS_LABELS[s] })),
                  ]}
                />
                <PillSelect
                  value={fPriority}
                  defaultValue={-1}
                  title="Priority"
                  onChange={setFPriority}
                  options={[
                    { value: -1, label: "Any priority" },
                    ...PRIORITY_LABELS.map((p, n) => ({ value: n, label: p })),
                  ]}
                />
                <PillSelect<IssueSort>
                  value={sort}
                  defaultValue="board"
                  title="Sort"
                  onChange={setSort}
                  options={[
                    { value: "board", label: "Board order" },
                    { value: "due", label: "Due date" },
                    { value: "updated", label: "Recently updated" },
                  ]}
                />
              </>
            )}
          </div>
        )}
        {busySync && (
          // Outside the filter bar's own row: the bar is only drawn once there
          // are issues, and a sync runs from the seeding prompt too.
          <div className={`issues-sync-progress${wide ? " compact" : ""}`}>
            <ProgressReadout
              progress={syncProgress}
              fallback={`Syncing with ${syncRemote || "the remote"}`}
            />
          </div>
        )}
        {loaded && issues.length === 0 ? (
          <div className="board empty issues-empty">
            <button
              className="quickadd-toggle big"
              title="Add an issue (c)"
              onClick={() => setQuickOpen(true)}
            >
              +
            </button>
            <div>
              {syncOn
                ? "Capture your first issues, or sync to bring down the shared backlog."
                : "Capture your first issues, or set up sharing to bring one down from a repository."}
            </div>
            {/* The toolbar's Sync button lives inside `issues.length > 0`, so
                without one here a machine that has just cloned a shared project
                has no way to fetch the backlog at all: nothing local, everything
                on the ref, and no control anywhere. That is exactly the state
                the second-machine half of the feature starts in. Plain "merge"
                is the right mode for it, not adopt: with nothing local there is
                no seeding question to ask, and the merge writes every issue the
                remote has.

                Sharing off is the same dead end one step earlier: the empty
                board was the only thing on screen and nothing on it said where
                sharing is turned on, so the second machine's first question
                ("how do I get my issues here") had no answer in the place it
                was being asked. */}
            <div className="issues-empty-actions">
              {syncOn && (
                <button
                  className="ghost"
                  title={syncTitle}
                  disabled={busySync}
                  onClick={() => { runSync("merge"); }}
                >
                  {busySync ? "Syncing…" : `Sync with ${syncRemote || "the remote"}`}
                </button>
              )}
              <button
                className="ghost"
                title={
                  syncOn
                    ? `This backlog syncs with ${syncRemote}. Change or turn off sharing in Settings.`
                    : "Share this backlog with a repository, so it travels between your machines."
                }
                onClick={onOpenBacklogSettings}
              >
                {syncOn ? "Backlog settings" : "Set up sharing"}
              </button>
            </div>
          </div>
        ) : (
          <div ref={listRef} className={`issues-list${drag ? " reordering" : ""}${wide ? " compact" : ""}`}>
            {issues.length > 0 && matched === 0 && (
              <div className="issues-nomatch">
                <div className="issues-nomatch-title">No issues match</div>
                <button className="ghost" onClick={clearFilters}>Clear search and filters</button>
              </div>
            )}
            {groups.map(({ status, issues: group }) => {
              if (group.length === 0) return null;
              const open = groupOpen(status);
              return (
                <section key={status} className="issue-group">
                  <button
                    className={`issue-group-head${filtered ? " static" : ""}`}
                    aria-expanded={open}
                    title={filtered ? undefined : `${open ? "Collapse" : "Expand"} ${STATUS_LABELS[status]}`}
                    onClick={() => {
                      // Filtering pins every group open, so folding is off.
                      if (filtered) return;
                      toggleGroup(status);
                    }}
                  >
                    {!filtered && <span className="issue-group-chev">{open ? "▾" : "▸"}</span>}
                    <span className="issue-group-name">{STATUS_LABELS[status]}</span>
                    <span className="issue-group-count">{group.length}</span>
                  </button>
                  {open && group.map((issue, idx) => (
                    <IssueRow
                      key={issue.id}
                      issue={issue}
                      label={issueLabel(project, issue)}
                      runs={runsFor(issue)}
                      selected={issue.id === selectedId}
                      onSelect={() => {
                        if (suppressClick.current) return;
                        setSelectedId(issue.id === selectedId ? null : issue.id);
                      }}
                      onStart={() => { startDefault(issue); }}
                      onSpawnAgent={(agentId, opts) => { onStartIssue(issue, agentId, opts); }}
                      onPatch={(p) => patch(issue, p)}
                      onDelete={() => setConfirmDelete(issue)}
                      terms={terms}
                      syncConflicts={conflicts.markers[issueLabel(project, issue)]}
                      gitless={gitless}
                      drag={!reorderable ? undefined : {
                        over:
                          drag && drag.status === status && drag.to === idx && drag.from !== idx
                            ? (drag.to > drag.from ? "below" : "above")
                            : null,
                        source: !!drag && drag.status === status && drag.from === idx,
                        idx,
                        status,
                        onMouseDown: (e) => onRowMouseDown(status, idx, e),
                      }}
                    />
                  ))}
                </section>
              );
            })}
          </div>
        )}
      </div>
      {selected && (
        <>
          {wide && (
            <Resizer
              size={sidebar.width}
              min={SIDEBAR_MIN}
              max={SIDEBAR_MAX}
              onChange={sidebar.setWidth}
              side="left"
            />
          )}
          <IssueDetail
            issue={selected}
            label={issueLabel(project, selected)}
            root={fileRoot}
            runs={runsFor(selected)}
            mentions={mentions}
            links={links}
            linkCandidates={candidates}
            index={ownDocs.index}
            cross={cross}
            expanded={wide}
            onToggleExpand={() => setExpanded(!expanded)}
            onPatch={(p) => patch(selected, p)}
            onStart={() => { startDefault(selected); }}
            onSpawnAgent={(agentId, opts) => { onStartIssue(selected, agentId, opts); }}
            onDelete={() => setConfirmDelete(selected)}
            gitless={gitless}
            onOpenRun={openRun}
            onOpenMention={openMention}
            onOpenIssue={(ref) =>
              requestNavigate({ kind: "issue", projectId: ref.project.id, issueId: ref.issue.id })}
            onAddLink={(ref) => { void addLink(ref); }}
            onRemoveLink={(link) => { void removeLink(link); }}
            onPostComment={(body) => comment(() => addIssueComment(selected.id, body))}
            onEditComment={(at, body) => comment(() => updateIssueComment(selected.id, at, body))}
            onDeleteComment={(at) => comment(() => deleteIssueComment(selected.id, at))}
            onFollowLink={followLink}
            // A tag in a description narrows the board to the issues carrying
            // it — the same "show me these" the docs tab gives its search.
            onTagClick={(tag) => setQ(tag.startsWith("#") ? tag : `#${tag}`)}
            onClose={() => setSelectedId(null)}
          />
        </>
      )}
      {seed && (
        // The one question a merge cannot answer for itself. Before the first
        // sync each machine numbered its issues from its own counter, so the
        // two backlogs have no identities in common and merging them would
        // report every issue as a clash. One side has to seed the other.
        //
        // Neither side is styled as the recommendation whenever both hold
        // issues, because neither is safe; `seedingRisk` is where that rule and
        // the reason for it live.
        <ConfirmDialog
          title="Which backlog is the real one?"
          body={
            <>
              This machine has {seed.local} issue{seed.local === 1 ? "" : "s"} and the shared
              copy has {seed.remote}, with nothing in common yet, so neither can be merged into the
              other. Whichever you keep replaces the other everywhere, and your other machines drop
              the replaced issues on their next sync. The replaced copy stays recoverable from the
              repository.
            </>
          }
          confirmLabel={`Keep this machine's ${seed.local}`}
          danger={seedingRisk(seed).publishDanger}
          altLabel={`Keep the shared ${seed.remote}`}
          altDanger={seedingRisk(seed).adoptDanger}
          busy={syncing}
          onConfirm={() => { setSeed(null); runSync("publish"); }}
          onAlt={() => { setSeed(null); runSync("adopt"); }}
          onCancel={() => setSeed(null)}
        />
      )}
      {conflicts.prompt && conflicts.prompt.length > 0 && (
        // The prompt the automatic schedule owes the user. A hand sync reports
        // its conflicts in a toast, which is right when someone is watching the
        // board they just pressed Sync on. On the schedule nobody is, and the
        // merge has already resolved a field by `updated`: for a body that is
        // text quietly replaced, so a dismissed toast would be the last anyone
        // heard of it. The row markers stay behind either way, and both this
        // and they outlive the board: it unmounts on a tab switch, which used
        // to discard the report and the rows it points at together.
        <ConfirmDialog
          title={`Sync decided ${conflicts.prompt.length} ${
            conflicts.prompt.length === 1 ? "field" : "fields"
          } for you`}
          body={
            <>
              <p>
                Both this machine and the shared copy changed these since the last sync, so the
                later edit won. What it replaced is still in the backlog's own history.
              </p>
              <ul className="issue-conflict-list">
                {conflicts.prompt.slice(0, 6).map((c, n) => (
                  <li key={`${c.key}-${c.field}-${n}`}>
                    <code>{c.key}</code> {c.field}: {c.detail}
                  </li>
                ))}
              </ul>
              {conflicts.prompt.length > 6 && (
                <p>and {conflicts.prompt.length - 6} more, marked on their rows.</p>
              )}
            </>
          }
          cancelLabel="Dismiss"
          confirmLabel={`Open ${conflicts.prompt[0].key}`}
          confirmDisabled={!conflictTarget}
          onConfirm={() => {
            if (conflictTarget) setSelectedId(conflictTarget.id);
            setConflicts(conflictReports.dismiss(project.id));
          }}
          onCancel={() => setConflicts(conflictReports.dismiss(project.id))}
        />
      )}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete issue?"
          body={`Delete "${issueLabel(project, confirmDelete)} ${confirmDelete.title}". Its number is never reused. This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => { doDelete(confirmDelete); }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}
