import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FileRoot, Issue, IssuePatch, RunInfo, trashPath } from "../api";
import { CrossRefs, IssueRef, LinkEdge } from "../lib/links";
import { IssueLink } from "../lib/issueLinks";
import { DocsIndex } from "../lib/docsIndex";
import { runName } from "../agents";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS, fmtDate, isOverdue } from "../lib/issues";
import { ISSUES_DIR, Attachment, insertAttachment, parseAttachments, removeAttachment } from "../lib/attachments";
import { Attached, attachBlob, attachPath } from "../lib/issueAttach";
import { dateStamp } from "../lib/dailyNote";
import { MenuCoords, anchorMenu } from "../lib/menuAnchor";
import { toastError } from "../lib/toast";
import { FindRank } from "../lib/findBus";
import { cmFindEngine } from "../lib/cmFind";
import { useFind } from "../hooks/useFind";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { PriorityGlyph, StatusDot } from "./IssueRow";
import IssueAttachments, { forgetAttachment } from "./IssueAttachments";
import MarkdownEditor, { MarkdownEditorHandle } from "./MarkdownEditor";
import AgentAddMenu from "./AgentAddMenu";
import IssueLinkMenu from "./IssueLinkMenu";
import DatePicker from "./DatePicker";
import { ContractIcon, ExpandIcon } from "./icons";

// The property menus, for anchoring: `.agent-menu`'s min-width (no status or
// priority label comes near its 300px max), one button's height, and the
// menu's own padding.
const MENU_W = 220;
const MENU_ROW_H = 33;
const MENU_PAD = 10;

function ts(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

// A date as a quiet property pill: reads as text ("◷ Due Aug 1"), the click
// opens the in-app calendar popover (native pickers can't be dismissed
// without choosing a date in this webview). Unset renders a ghost prompt;
// clearing is the ✕ that appears once a date is set.
//
// `bare` drops the label from the pill's own text, for the expanded layout's
// meta rail where the row already carries it ("Due    ◷ Aug 1").
function DateProp({
  glyph,
  label,
  value,
  overdue,
  bare,
  onChange,
}: {
  glyph: string;
  label: string;
  value: string | null;
  overdue?: boolean;
  bare?: boolean;
  onChange: (v: string | null) => void;
}) {
  const pillRef = useRef<HTMLSpanElement>(null);
  const [picker, setPicker] = useState<MenuCoords | null>(null);
  const today = dateStamp(new Date());

  // Same edge-flip anchoring as the row menus: the detail pane hugs the
  // window's right edge, so the popover usually opens leftward.
  const openPicker = () => {
    const r = pillRef.current?.getBoundingClientRect();
    if (r) setPicker(anchorMenu(r, 240, 260)); // .date-picker's width, and its rendered height
  };

  return (
    <>
      <span
        ref={pillRef}
        className={`issue-prop-pill date-pill${value ? "" : " date-empty"}${overdue ? " overdue" : ""}`}
        role="button"
        tabIndex={0}
        title={value ? `${label} ${value} (click to change)` : `Set ${label.toLowerCase()} date`}
        onClick={openPicker}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openPicker(); } }}
      >
        <span aria-hidden>{glyph}</span>
        {value ? (bare ? fmtDate(value, today) : `${label} ${fmtDate(value, today)}`) : bare ? "Set date" : label}
        {value && (
          <button
            className="date-clear"
            title={`Clear ${label.toLowerCase()} date`}
            onClick={(e) => { e.stopPropagation(); onChange(null); }}
          >
            ✕
          </button>
        )}
      </span>
      {picker && (
        <DatePicker
          value={value}
          coords={picker}
          onPick={(d) => { setPicker(null); if (d !== value) onChange(d); }}
          onClear={() => { setPicker(null); onChange(null); }}
          onClose={() => setPicker(null)}
        />
      )}
    </>
  );
}

// Right-hand detail pane of the Issues view. Title/body commit on blur (and
// Enter for the title); status/priority commit immediately.
//
// Two layouts, same content and same editing model. Contracted, it is the
// 360px rail beside the board: one column, everything stacked. Expanded, it
// takes over the view and splits into a reading column (title + description,
// which grows to fill the height) and a meta rail (properties, agents,
// mentions, attachments) — see `.issue-detail.expanded` in styles.css.
export default function IssueDetail({
  issue,
  label,
  root,
  runs,
  mentions,
  links,
  linkCandidates,
  index,
  cross,
  expanded,
  onPatch,
  onStart,
  onSpawnAgent,
  onDelete,
  onOpenRun,
  onOpenMention,
  onOpenIssue,
  onAddLink,
  onRemoveLink,
  onFollowLink,
  onTagClick,
  onToggleExpand,
  onClose,
}: {
  issue: Issue;
  label: string;
  // The project's main checkout, where `.agency/issues/` (and the attachments
  // beside it) live.
  root: FileRoot;
  runs: RunInfo[];
  // Notes and issues whose text links here ([[AGE-14]]), via lib/links.
  mentions: LinkEdge[];
  // The Links section: issues this one is linked to, either side's `links:`
  // having said so (lib/issueLinks), and what the "+" can still offer.
  links: IssueLink[];
  linkCandidates: IssueRef[];
  // What the description's wikilinks resolve against: this project's notes,
  // and every project's issues and runs.
  index: DocsIndex | null;
  cross: CrossRefs | null;
  expanded: boolean;
  // Awaited before an agent is dispatched, so the run reads the issue the user
  // is looking at rather than the one still on disk.
  onPatch: (patch: IssuePatch) => void | Promise<void>;
  // Same pair as the list rows: ▶ starts the project's default agent, the
  // caret beside it picks another (or races/loops them).
  onStart: () => void;
  onSpawnAgent: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
  onDelete: () => void;
  onOpenRun: (runId: string) => void;
  onOpenMention: (edge: LinkEdge) => void;
  // A linked issue opened from the Links section, and the two writes behind
  // that section — both sides of a link are written, so either file read on
  // its own tells the whole relation.
  onOpenIssue: (ref: IssueRef) => void;
  onAddLink: (ref: IssueRef) => void;
  onRemoveLink: (link: IssueLink) => void;
  // ⌘-click on a wikilink in the description, and a click on a #tag.
  onFollowLink: (target: string, heading: string | null) => void;
  onTagClick: (tag: string) => void;
  onToggleExpand: () => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(issue.title);
  const [body, setBody] = useState(issue.body);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<MarkdownEditorHandle>(null);
  const asideRef = useRef<HTMLElement>(null);
  const [menu, setMenu] = useState<"status" | "priority" | null>(null);
  const [coords, setCoords] = useState<MenuCoords>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const priorityRef = useRef<HTMLButtonElement>(null);
  // A file being dragged over this pane, and an attach already in flight.
  const [dragOver, setDragOver] = useState(false);
  const [attaching, setAttaching] = useState(false);

  // Expanded, these pills sit in the meta rail at the far right of the window,
  // where a left-anchored menu would open past the edge — anchorMenu flips it.
  // The row count decides the height: statuses and priorities differ by one.
  const openMenu = (which: "status" | "priority", ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    const rows = which === "status" ? ISSUE_STATUSES.length : PRIORITY_LABELS.length;
    if (r) setCoords(anchorMenu(r, MENU_W, rows * MENU_ROW_H + MENU_PAD));
    setMenu(which);
  };

  // Rather than leave the menu stranded where the pill used to be. (The date
  // popover does the same from inside DatePicker.)
  useDismissOnResize(menu !== null, () => setMenu(null));

  // What this pane last sent to (or loaded from) the tracker. The `issue` prop
  // lags a write by a save + refresh round-trip, so it can't answer "is this
  // draft already saved?" — this can.
  const saved = useRef({ title: issue.title, body: issue.body });
  // The freshest drafts, for the flush that runs as the pane is torn down: by
  // then the props have already moved on to the next issue.
  const draft = useRef({ title, body });
  draft.current = { title, body };
  // Which issue the drafts above belong to. For the one commit between a
  // selection change and the reset effect they are still the previous issue's,
  // and what the pane renders has to come from the props instead — the
  // description editor is keyed by issue and would otherwise open on the
  // outgoing text.
  const draftsFor = useRef(issue.id);
  const current = draftsFor.current === issue.id;
  const shownTitle = current ? title : issue.title;
  const shownBody = current ? body : issue.body;

  // Reset drafts when another issue is selected — but never clobber an edit
  // in progress with poll results for the same issue. The reset has to stay an
  // effect: the flush below is one too, and its cleanup reads the outgoing
  // issue's drafts before this runs.
  useEffect(() => {
    draftsFor.current = issue.id;
    setTitle(issue.title);
    setBody(issue.body);
    saved.current = { title: issue.title, body: issue.body };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id]);

  // Grow the title textarea to fit its wrapped content (no scroll, no clip).
  // Re-measured on a layout switch too: the same title wraps to fewer lines
  // once the pane is expanded.
  useLayoutEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [shownTitle, expanded]);

  // What is typed but not yet written. Titles are trimmed and may not be
  // emptied; an empty description is a legitimate edit.
  const pending = (d: { title: string; body: string }): IssuePatch | null => {
    const p: IssuePatch = {};
    const t = d.title.trim();
    if (t && t !== saved.current.title) p.title = t;
    if (d.body !== saved.current.body) p.body = d.body;
    return Object.keys(p).length > 0 ? p : null;
  };

  // The write in flight, so a dispatch can wait on it: an agent started from
  // this pane's head bar reads the issue back off disk, and the blur that
  // commits the description lands in the same tick as the click.
  const inflight = useRef<Promise<unknown>>(Promise.resolve());
  const send = (p: IssuePatch) => {
    saved.current = { title: p.title ?? saved.current.title, body: p.body ?? saved.current.body };
    // Failures are reported by the caller's toast; here they only need to stop
    // being unhandled, and to let a dispatch waiting on this write proceed.
    inflight.current = Promise.resolve(onPatch(p)).catch(() => {});
  };

  const commitTitle = () => {
    const t = title.trim();
    if (t && t !== saved.current.title) send({ title: t });
    // Whitespace-only is not a title; anything else just loses its padding.
    else setTitle(t || saved.current.title);
  };
  // The editor hands its own text over: the blur can land before React has
  // processed the keystroke that preceded it.
  const commitBody = (text: string) => {
    if (text !== saved.current.body) send({ body: text });
  };

  // Edits commit on blur, but a click can take the pane away before any blur
  // fires: the list rows call preventDefault on mousedown (to stop WebKit
  // starting a text selection mid-row), and that suppresses the blur outright.
  // So whatever is still uncommitted is written as the pane switches issues or
  // closes. The effect's own closure is what carries the outgoing issue — its
  // `onPatch` still points at the issue being left behind.
  useEffect(() => {
    return () => {
      const p = pending(draft.current);
      if (p) onPatch(p);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id]);

  // Dispatching an agent reads the issue off disk, so every edit has to land
  // first: the one the blur just sent, and anything the blur missed.
  const startAfterSave = async (run: () => void) => {
    const p = pending({ title, body });
    if (p) send(p);
    await inflight.current;
    run();
  };

  // ── attachments ───────────────────────────────────────────────────────────
  // Files pasted, dropped, or picked here are copied into
  // `.agency/issues/assets/` and referenced from the body with a relative
  // markdown link — which resolves from the issue file's own directory, so the
  // same link renders on GitHub and in any markdown editor, and the bytes
  // merge onto main with the issue text.

  const attachments = useMemo(() => parseAttachments(shownBody), [shownBody]);

  // The freshest body for async handlers to splice into: an attach that
  // resolves after a keystroke must not write back the body it started with.
  const bodyDraft = useRef(shownBody);
  bodyDraft.current = shownBody;

  // Unlike a typed edit, an attachment saves immediately: the bytes are
  // already on disk, and a body left uncommitted (pane closed, app quit)
  // would orphan them. `caret` parks the cursor after a splice.
  const saveBody = (next: string, caret?: number) => {
    bodyDraft.current = next;
    setBody(next);
    bodyRef.current?.setText(next, caret);
    if (next !== saved.current.body) send({ body: next });
  };

  // ⌘F in the description: the same bar the note and file editors get, over
  // the same editor. The board's own filter outranks this (see FindRank), so
  // ⌘F only lands here once the description holds focus.
  const findEngine = useMemo(() => cmFindEngine(() => bodyRef.current?.view() ?? null), []);
  const { bar: findBar, onContentChange: onBodyChange } = useFind({
    host: asideRef,
    engine: findEngine,
    canReplace: true,
    rank: FindRank.editor,
    variant: "inline",
    resetKey: issue.id,
  });

  // Guards the drop listener, which is subscribed once per issue and so would
  // otherwise read `attaching` from the render that installed it.
  const attachingRef = useRef(attaching);
  attachingRef.current = attaching;

  // Run a batch of attach jobs and splice the markdown they produce into the
  // body — at the caret for a paste, at the end for a drop or a file pick.
  // Failures are reported per file; the ones that landed still get written.
  const runAttach = async (
    jobs: Array<() => Promise<Attached>>,
    at: { from: number; to: number; text: string } | null,
  ) => {
    if (jobs.length === 0 || attachingRef.current) return;
    attachingRef.current = true;
    setAttaching(true);
    try {
      const done: Attached[] = [];
      for (const job of jobs) {
        try {
          done.push(await job());
        } catch (e) {
          toastError(e, "Couldn't attach file");
        }
      }
      if (done.length === 0) return;
      const md = done.map((d) => d.markdown).join("\n");
      const cur = bodyRef.current?.text() ?? bodyDraft.current;
      // Offsets only survive if nothing was typed while the bytes were being
      // written; otherwise they point into text that has moved, so fall back
      // to appending rather than splicing mid-word.
      if (at && at.text === cur) {
        const next = insertAttachment(cur, at.from, at.to, md);
        saveBody(next.body, next.cursor);
      } else {
        // Appended attachments start their own paragraph at the end.
        saveBody(cur.trim() ? `${cur.replace(/\n+$/, "")}\n\n${md}\n` : `${md}\n`);
      }
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  const pickFiles = async () => {
    try {
      const picked = await openDialog({ multiple: true, title: `Attach to ${label}` });
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
      await runAttach(paths.map((p) => () => attachPath(root, label, p)), null);
    } catch (e) {
      toastError(e, "Couldn't attach file");
    }
  };

  // Dropping from Finder. Tauri intercepts OS drag-drop at the webview
  // boundary, so HTML5 drop events never arrive — we listen to Tauri's own
  // stream and hit-test the pane's rect ourselves. Positions are already CSS
  // pixels on macOS despite the payload's "physical" naming; scaling them by
  // devicePixelRatio makes the hit-test miss on Retina.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const hit = (pos: { x: number; y: number }) => {
      const r = asideRef.current?.getBoundingClientRect();
      if (!r) return false;
      return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
    };
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (disposed) return;
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setDragOver(hit(p.position) && !attachingRef.current);
        } else if (p.type === "drop") {
          setDragOver(false);
          if (!hit(p.position) || p.paths.length === 0) return;
          void runAttach(p.paths.map((path) => () => attachPath(root, label, path)), null);
        } else {
          setDragOver(false);
        }
      })
      .then((u) => { if (disposed) u(); else unlisten = u; });
    return () => { disposed = true; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id, label, root.kind, root.id]);

  // Removing an attachment drops its link and trashes the file — recoverable
  // from the OS trash, and it keeps orphans out of a tracked directory.
  const removeAt = async (a: Attachment) => {
    saveBody(removeAttachment(bodyDraft.current, a));
    forgetAttachment(root, a.repoPath);
    try {
      await trashPath(root, a.repoPath);
    } catch (e) {
      toastError(e, "Removed the link, but couldn't delete the file");
    }
  };

  // ── the pieces, arranged differently by each layout ──────────────────────

  // Dispatch lives in the head bar, which both layouts render — so an issue
  // being read has the same ▶ as an issue in the list, expanded or not.
  const startable = issue.status !== "done" && issue.status !== "cancelled";

  const head = (
    <div className="issue-detail-head">
      <code className="issue-key">{label}</code>
      <div className="spacer" />
      {startable && (
        <span className="issue-detail-start">
          <button
            className="icon-btn"
            title="Start agent on this issue"
            onClick={() => { void startAfterSave(onStart); }}
          >
            ▶
          </button>
          <AgentAddMenu
            variant="icon"
            projectId={issue.projectId}
            issue={issue}
            issueLabel={label}
            onSpawn={(agentId, opts) => { void startAfterSave(() => onSpawnAgent(agentId, opts)); }}
            onTerminal={() => {}}
          />
        </span>
      )}
      <button
        className="icon-btn"
        title="Attach a file"
        disabled={attaching}
        onClick={() => { void pickFiles(); }}
      >
        ⊕
      </button>
      <button
        className="icon-btn"
        title={expanded ? "Contract: back to the full board" : "Expand: give this issue the view"}
        onClick={onToggleExpand}
      >
        {expanded ? <ContractIcon /> : <ExpandIcon />}
      </button>
      <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
    </div>
  );

  const titleField = (
    <textarea
      ref={titleRef}
      className="issue-detail-title"
      rows={1}
      placeholder="Issue title"
      value={shownTitle}
      onChange={(e) => setTitle(e.target.value)}
      onBlur={commitTitle}
      onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); (e.target as HTMLTextAreaElement).blur(); } }}
    />
  );

  const statusPill = (
    <button
      ref={statusRef}
      className="issue-prop-pill"
      title="Change status"
      onClick={() => openMenu("status", statusRef)}
    >
      <StatusDot status={issue.status} />
      {STATUS_LABELS[issue.status]}
    </button>
  );
  const priorityPill = (
    <button
      ref={priorityRef}
      className="issue-prop-pill"
      title="Change priority"
      onClick={() => openMenu("priority", priorityRef)}
    >
      <PriorityGlyph priority={issue.priority} />
      {PRIORITY_LABELS[issue.priority]}
    </button>
  );
  const duePill = (
    <DateProp
      glyph="◷"
      label="Due"
      value={issue.due}
      overdue={isOverdue(issue, dateStamp(new Date()))}
      bare={expanded}
      onChange={(due) => onPatch({ due })}
    />
  );
  const scheduledPill = (
    <DateProp
      glyph="⧖"
      label="Scheduled"
      value={issue.scheduled}
      bare={expanded}
      onChange={(scheduled) => onPatch({ scheduled })}
    />
  );

  // The description renders as markdown the way a note does: live preview,
  // raw syntax revealed on the line the caret is on. Keyed by issue so
  // switching tickets starts a fresh document (and a fresh undo history).
  const bodyField = (
    <>
      {findBar}
      <MarkdownEditor
        key={issue.id}
        ref={bodyRef}
        className="issue-detail-body md-live"
        value={shownBody}
        placeholder="Add description…  (paste or drop a file to attach)"
        root={root}
        dir={ISSUES_DIR}
        path={`${label}.md`}
        index={index}
        cross={cross}
        onChange={(text) => { bodyDraft.current = text; setBody(text); onBodyChange(); }}
        onBlur={commitBody}
        onNavigate={onFollowLink}
        onTagClick={onTagClick}
        onPasteFiles={(files, at) => {
          void runAttach(files.map((f) => () => attachBlob(root, label, f)), at);
        }}
      />
    </>
  );

  const attachmentsList = (
    <IssueAttachments root={root} attachments={attachments} onRemove={(a) => { void removeAt(a); }} />
  );

  const runsList = runs.length > 0 && (
    <div className="issue-detail-runs">
      <h3>Agents on this issue</h3>
      {runs.map((r) => (
        <button key={r.id} className="issue-detail-run" onClick={() => onOpenRun(r.id)}>
          <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
          <span className="issue-detail-run-name">{runName(r)}</span>
          <span className="badge">{r.agent}</span>
        </button>
      ))}
    </div>
  );

  // Linked issues. Always rendered, empty or not: the "+" beside the heading
  // is the only way to make a link without hand-editing frontmatter, and a
  // section that appears only once it has content can't be found.
  const linksList = (
    <div className="issue-detail-runs issue-detail-links">
      <div className="issue-detail-section-head">
        <h3>Links</h3>
        <IssueLinkMenu candidates={linkCandidates} onPick={onAddLink} />
      </div>
      {links.length === 0 && <div className="issue-detail-empty">No linked issues</div>}
      {links.map((l) => (
        <div key={l.label} className="issue-detail-link-row">
          <button
            className={`issue-detail-run issue-detail-link${l.ref ? "" : " unresolved"}`}
            title={
              l.ref
                ? `${l.ref.project.name} · ${l.ref.issue.title}`
                : `No issue ${l.label} in any project (linked anyway)`
            }
            disabled={!l.ref}
            onClick={() => { if (l.ref) onOpenIssue(l.ref); }}
          >
            {l.ref ? <StatusDot status={l.ref.issue.status} /> : <span className="issue-link-ghost" aria-hidden>◌</span>}
            <code className="issue-link-key">{l.label}</code>
            <span className="issue-detail-run-name">{l.ref?.issue.title ?? "Not in this workspace"}</span>
          </button>
          <button
            className="issue-link-remove"
            title={`Unlink ${l.label}`}
            onClick={() => onRemoveLink(l)}
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );

  const mentionsList = mentions.length > 0 && (
    <div className="issue-detail-runs issue-detail-mentions">
      <h3>Mentions</h3>
      {mentions.map((m, i) => (
        <button
          key={`${m.fromKind}:${m.fromProjectId}:${m.fromId}:${m.line}:${i}`}
          className="issue-detail-run issue-detail-mention"
          title={m.snippet}
          onClick={() => onOpenMention(m)}
        >
          <span className="mention-glyph" aria-hidden>{m.fromKind === "issue" ? "▧" : "▥"}</span>
          <span className="issue-detail-run-name">
            {m.fromLabel ? `${m.fromLabel} ` : ""}{m.fromTitle}
          </span>
          <span className="mention-snippet">{m.snippet}</span>
        </button>
      ))}
    </div>
  );

  const foot = (
    <div className="issue-detail-foot">
      <span title={`Updated ${ts(issue.updatedAt)}`}>Created {ts(issue.createdAt)}</span>
      <div className="spacer" />
      <button className="ghost" onClick={onDelete}>Delete</button>
    </div>
  );

  // Both pickers render from here so the fixed-position menu isn't clipped by
  // either layout's scrolling column.
  const propMenu = menu && (
    <>
      <div className="agent-menu-backdrop" onClick={() => setMenu(null)} />
      <div className="agent-menu" style={{ position: "fixed", ...coords }}>
        {menu === "status" &&
          ISSUE_STATUSES.map((s) => (
            <button key={s} onClick={() => { setMenu(null); if (s !== issue.status) onPatch({ status: s }); }}>
              <StatusDot status={s} /> {STATUS_LABELS[s]}{s === issue.status ? " ✓" : ""}
            </button>
          ))}
        {menu === "priority" &&
          PRIORITY_LABELS.map((p, n) => (
            <button key={p} onClick={() => { setMenu(null); if (n !== issue.priority) onPatch({ priority: n }); }}>
              <PriorityGlyph priority={n} /> {p}{n === issue.priority ? " ✓" : ""}
            </button>
          ))}
      </div>
    </>
  );

  const dropHint = (dragOver || attaching) && (
    <div className="issue-drop-hint">{attaching ? "Attaching…" : `Drop to attach to ${label}`}</div>
  );

  const cls = `issue-detail${expanded ? " expanded" : ""}${dragOver ? " drop-target" : ""}`;

  if (expanded) {
    return (
      <aside ref={asideRef} className={cls}>
        {head}
        <div className="issue-detail-cols">
          <div className="issue-detail-read">
            <div className="issue-detail-reading">
              {titleField}
              {bodyField}
              {attachmentsList}
            </div>
          </div>
          <div className="issue-detail-rail">
            <div className="issue-meta">
              <h3>Properties</h3>
              <div className="issue-meta-row"><span className="issue-meta-label">Status</span>{statusPill}</div>
              <div className="issue-meta-row"><span className="issue-meta-label">Priority</span>{priorityPill}</div>
              <div className="issue-meta-row"><span className="issue-meta-label">Due</span>{duePill}</div>
              <div className="issue-meta-row"><span className="issue-meta-label">Scheduled</span>{scheduledPill}</div>
            </div>
            {runsList}
            {linksList}
            {mentionsList}
            {foot}
          </div>
        </div>
        {propMenu}
        {dropHint}
      </aside>
    );
  }

  return (
    <aside ref={asideRef} className={cls}>
      {head}
      {titleField}
      <div className="issue-detail-props">
        {statusPill}
        {priorityPill}
        {duePill}
        {scheduledPill}
      </div>
      {propMenu}
      {bodyField}
      {attachmentsList}
      {dropHint}
      {runsList}
      {linksList}
      {mentionsList}
      {foot}
    </aside>
  );
}
