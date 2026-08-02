import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FileRoot, Issue, IssuePatch, RunInfo, trashPath } from "../api";
import { LinkEdge } from "../lib/links";
import { runName } from "../agents";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS, fmtDate, isOverdue } from "../lib/issues";
import { Attachment, insertAttachment, parseAttachments, removeAttachment } from "../lib/attachments";
import { Attached, attachBlob, attachPath } from "../lib/issueAttach";
import { dateStamp } from "../lib/dailyNote";
import { toastError } from "../lib/toast";
import { PriorityGlyph, StatusDot } from "./IssueRow";
import IssueAttachments, { forgetAttachment } from "./IssueAttachments";
import DatePicker from "./DatePicker";

function ts(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

// A date as a quiet property pill: reads as text ("◷ Due Aug 1"), the click
// opens the in-app calendar popover (native pickers can't be dismissed
// without choosing a date in this webview). Unset renders a ghost prompt;
// clearing is the ✕ that appears once a date is set.
function DateProp({
  glyph,
  label,
  value,
  overdue,
  onChange,
}: {
  glyph: string;
  label: string;
  value: string | null;
  overdue?: boolean;
  onChange: (v: string | null) => void;
}) {
  const pillRef = useRef<HTMLSpanElement>(null);
  const [picker, setPicker] = useState<{ top: number; left?: number; right?: number } | null>(null);
  const today = dateStamp(new Date());

  // Same edge-flip anchoring as the row menus: the detail pane hugs the
  // window's right edge, so the popover usually opens leftward.
  const openPicker = () => {
    const r = pillRef.current?.getBoundingClientRect();
    if (!r) return;
    const MENU_W = 240; // .date-picker width
    const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
    setPicker(
      fitsRight
        ? { top: r.bottom + 4, left: r.left }
        : { top: r.bottom + 4, right: window.innerWidth - r.right },
    );
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
        {value ? `${label} ${fmtDate(value, today)}` : label}
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
export default function IssueDetail({
  issue,
  label,
  root,
  runs,
  mentions,
  onPatch,
  onDelete,
  onOpenRun,
  onOpenMention,
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
  onPatch: (patch: IssuePatch) => void;
  onDelete: () => void;
  onOpenRun: (runId: string) => void;
  onOpenMention: (edge: LinkEdge) => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(issue.title);
  const [body, setBody] = useState(issue.body);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const asideRef = useRef<HTMLElement>(null);
  const [menu, setMenu] = useState<"status" | "priority" | null>(null);
  const [coords, setCoords] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const priorityRef = useRef<HTMLButtonElement>(null);
  // A file being dragged over this pane, and an attach already in flight.
  const [dragOver, setDragOver] = useState(false);
  const [attaching, setAttaching] = useState(false);
  // Where to put the caret once an inserted attachment has rendered.
  const pendingCaret = useRef<number | null>(null);

  const openMenu = (which: "status" | "priority", ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setCoords({ top: r.bottom + 4, left: r.left });
    setMenu(which);
  };

  // Reset drafts when another issue is selected — but never clobber an edit
  // in progress with poll results for the same issue.
  useEffect(() => {
    setTitle(issue.title);
    setBody(issue.body);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id]);

  // Grow the title textarea to fit its wrapped content (no scroll, no clip).
  useLayoutEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [title]);

  const commitTitle = () => {
    const t = title.trim();
    if (t && t !== issue.title) onPatch({ title: t });
    else setTitle(issue.title);
  };
  const commitBody = () => {
    if (body !== issue.body) onPatch({ body });
  };

  // ── attachments ───────────────────────────────────────────────────────────
  // Files pasted, dropped, or picked here are copied into
  // `.agency/issues/assets/` and referenced from the body with a relative
  // markdown link — which resolves from the issue file's own directory, so the
  // same link renders on GitHub and in any markdown editor, and the bytes
  // merge onto main with the issue text.

  const attachments = useMemo(() => parseAttachments(body), [body]);

  // The freshest body for async handlers to splice into: an attach that
  // resolves after a keystroke must not write back the body it started with.
  const bodyDraft = useRef(body);
  bodyDraft.current = body;

  // Unlike a typed edit, an attachment saves immediately: the bytes are
  // already on disk, and a body left uncommitted (pane closed, app quit)
  // would orphan them.
  const saveBody = (next: string) => {
    bodyDraft.current = next;
    setBody(next);
    if (next !== issue.body) onPatch({ body: next });
  };

  // Guards the drop listener, which is subscribed once per issue and so would
  // otherwise read `attaching` from the render that installed it.
  const attachingRef = useRef(attaching);
  attachingRef.current = attaching;

  // Run a batch of attach jobs and splice the markdown they produce into the
  // body — at the caret for a paste, at the end for a drop or a file pick.
  // Failures are reported per file; the ones that landed still get written.
  const runAttach = async (
    jobs: Array<() => Promise<Attached>>,
    at: { from: number; to: number; body: string } | null,
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
      const cur = bodyDraft.current;
      // Offsets only survive if nothing was typed while the bytes were being
      // written; otherwise they point into text that has moved, so fall back
      // to appending rather than splicing mid-word.
      if (at && at.body === cur) {
        const next = insertAttachment(cur, at.from, at.to, md);
        pendingCaret.current = next.cursor;
        saveBody(next.body);
      } else {
        // Appended attachments start their own paragraph at the end.
        saveBody(cur.trim() ? `${cur.replace(/\n+$/, "")}\n\n${md}\n` : `${md}\n`);
      }
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  // Paste: images only. Anything else on the clipboard is text the textarea
  // should keep handling itself.
  const onPasteBody = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
    if (files.length === 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    const at = { from: el.selectionStart, to: el.selectionEnd, body: bodyDraft.current };
    void runAttach(files.map((f) => () => attachBlob(root, label, f)), at);
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

  // Put the caret back after an inserted attachment re-renders the textarea.
  useLayoutEffect(() => {
    const el = bodyRef.current;
    const pos = pendingCaret.current;
    if (!el || pos == null) return;
    pendingCaret.current = null;
    el.focus();
    el.setSelectionRange(pos, pos);
  }, [body]);

  return (
    <aside ref={asideRef} className={`issue-detail${dragOver ? " drop-target" : ""}`}>
      <div className="issue-detail-head">
        <code className="issue-key">{label}</code>
        <div className="spacer" />
        <button
          className="icon-btn"
          title="Attach a file"
          disabled={attaching}
          onClick={() => { void pickFiles(); }}
        >
          ⊕
        </button>
        <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
      </div>
      <textarea
        ref={titleRef}
        className="issue-detail-title"
        rows={1}
        placeholder="Issue title"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); (e.target as HTMLTextAreaElement).blur(); } }}
      />
      <div className="issue-detail-props">
        <button
          ref={statusRef}
          className="issue-prop-pill"
          title="Change status"
          onClick={() => openMenu("status", statusRef)}
        >
          <StatusDot status={issue.status} />
          {STATUS_LABELS[issue.status]}
        </button>
        <button
          ref={priorityRef}
          className="issue-prop-pill"
          title="Change priority"
          onClick={() => openMenu("priority", priorityRef)}
        >
          <PriorityGlyph priority={issue.priority} />
          {PRIORITY_LABELS[issue.priority]}
        </button>
        <DateProp
          glyph="◷"
          label="Due"
          value={issue.due}
          overdue={isOverdue(issue, dateStamp(new Date()))}
          onChange={(due) => onPatch({ due })}
        />
        <DateProp
          glyph="⧖"
          label="Scheduled"
          value={issue.scheduled}
          onChange={(scheduled) => onPatch({ scheduled })}
        />
      </div>

      {menu && (
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
      )}
      <textarea
        ref={bodyRef}
        className="issue-detail-body"
        placeholder="Add description…  (paste or drop a file to attach)"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onPaste={onPasteBody}
        onBlur={commitBody}
      />
      <IssueAttachments root={root} attachments={attachments} onRemove={(a) => { void removeAt(a); }} />
      {(dragOver || attaching) && (
        <div className="issue-drop-hint">{attaching ? "Attaching…" : `Drop to attach to ${label}`}</div>
      )}
      {runs.length > 0 && (
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
      )}
      {mentions.length > 0 && (
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
      )}
      <div className="issue-detail-foot">
        <span title={`Updated ${ts(issue.updatedAt)}`}>Created {ts(issue.createdAt)}</span>
        <div className="spacer" />
        <button className="ghost" onClick={onDelete}>Delete</button>
      </div>
    </aside>
  );
}
