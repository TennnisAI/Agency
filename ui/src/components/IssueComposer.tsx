import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { FileRoot } from "../api";
import { CrossRefs } from "../lib/links";
import { DocsIndex } from "../lib/docsIndex";
import { IssueDraft } from "../lib/issueDraft";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS } from "../lib/issues";
import { ISSUES_DIR } from "../lib/attachments";
import { MenuCoords, anchorMenu } from "../lib/menuAnchor";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { useModalKeys } from "../hooks/useModalKeys";
import { PriorityGlyph, StatusDot } from "./IssueRow";
import { DateProp } from "./IssueDetail";
import MarkdownEditor, { MarkdownEditorHandle } from "./MarkdownEditor";

// Same anchoring numbers as the detail pane's property menus.
const MENU_W = 220;
const MENU_ROW_H = 33;
const MENU_PAD = 10;

// The new-issue form (AGE-255): title, description and properties together,
// as one card over the Issues tab. It covers the board and nothing else, so
// the rest of the app stays live; leaving the tab and coming back finds it
// where it was, because every change goes straight into the stored draft
// (lib/issueDraft) rather than into state here. Closing saves the way the
// detail pane does; Save and Discard say it outright.
export default function IssueComposer({
  draft,
  root,
  index,
  cross,
  saving,
  onChange,
  onSave,
  onDiscard,
  onClose,
  onFollowLink,
}: {
  draft: IssueDraft;
  root: FileRoot;
  index: DocsIndex | null;
  cross: CrossRefs | null;
  // A save in flight: the buttons hold still so it cannot be sent twice.
  saving: boolean;
  onChange: (next: IssueDraft) => void;
  onSave: () => void;
  onDiscard: () => void;
  onClose: () => void;
  onFollowLink: (target: string, heading: string | null) => void;
}) {
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<MarkdownEditorHandle>(null);
  const [menu, setMenu] = useState<"status" | "priority" | null>(null);
  const [coords, setCoords] = useState<MenuCoords>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const priorityRef = useRef<HTMLButtonElement>(null);
  const canSave = draft.title.trim() !== "" && !saving;

  // The editor reports keystrokes through a callback it captured once, so the
  // merge has to read the draft as it is now, not as it was at mount.
  const latest = useRef(draft);
  latest.current = draft;
  const set = (p: Partial<IssueDraft>) => onChange({ ...latest.current, ...p });

  // The status and priority menus are not on the modal stack (the date picker
  // is), so Escape over one of them closes the menu, not the whole form.
  useModalKeys(() => (menu ? setMenu(null) : onClose()), !saving);
  useDismissOnResize(menu !== null, () => setMenu(null));

  // Whatever was being written last gets the caret back: the title for a new
  // draft, the end of the description for one that already has a title.
  useEffect(() => {
    if (draft.title.trim() && draft.body) {
      const v = bodyRef.current?.view();
      if (v) {
        v.dispatch({ selection: { anchor: v.state.doc.length } });
        v.focus();
        return;
      }
    }
    const t = titleRef.current;
    if (t) {
      t.focus();
      t.setSelectionRange(t.value.length, t.value.length);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The title wraps rather than scrolls, as in the detail pane.
  useLayoutEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft.title]);

  const openMenu = (which: "status" | "priority", ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    const rows = which === "status" ? ISSUE_STATUSES.length : PRIORITY_LABELS.length;
    if (r) setCoords(anchorMenu(r, MENU_W, rows * MENU_ROW_H + MENU_PAD));
    setMenu(which);
  };

  return (
    <div className="issue-composer-scrim">
      <section
        className="issue-composer"
        role="dialog"
        aria-label="New issue"
        // Capture, so ⌘Enter saves from the description too: CodeMirror's own
        // keymap would otherwise take it as "insert a blank line".
        onKeyDownCapture={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            e.stopPropagation();
            if (canSave) onSave();
          }
        }}
      >
        <div className="issue-composer-head">
          <span className="issue-composer-heading">New issue</span>
          <div className="spacer" />
          <button
            className="icon-btn"
            title={draft.title.trim() ? "Close and save (Esc)" : "Close, keeping the draft (Esc)"}
            disabled={saving}
            onClick={onClose}
          >
            ✕
          </button>
        </div>
        <div className="issue-composer-main">
          <textarea
            ref={titleRef}
            className="issue-detail-title"
            rows={1}
            placeholder="Issue title"
            value={draft.title}
            onChange={(e) => set({ title: e.target.value.replace(/\n/g, " ") })}
            onKeyDown={(e) => {
              // A title is one line; Enter moves on to the description.
              if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                e.preventDefault();
                bodyRef.current?.view()?.focus();
              }
            }}
          />
          <div className="issue-detail-props">
            <button
              ref={statusRef}
              className="issue-prop-pill"
              title="Status"
              onClick={() => openMenu("status", statusRef)}
            >
              <StatusDot status={draft.status} />
              {STATUS_LABELS[draft.status]}
            </button>
            <button
              ref={priorityRef}
              className="issue-prop-pill"
              title="Priority"
              onClick={() => openMenu("priority", priorityRef)}
            >
              <PriorityGlyph priority={draft.priority} />
              {PRIORITY_LABELS[draft.priority]}
            </button>
            <DateProp glyph="◷" label="Due" value={draft.due} onChange={(due) => set({ due })} />
            <DateProp
              glyph="⧖"
              label="Scheduled"
              value={draft.scheduled}
              onChange={(scheduled) => set({ scheduled })}
            />
          </div>
          {/* No paste-to-attach here: an attachment is filed under the issue's
              key, and a draft does not have one until it is saved. */}
          <MarkdownEditor
            ref={bodyRef}
            className="issue-detail-body issue-composer-body md-live"
            value={draft.body}
            placeholder="Add description…  (files can be attached once the issue is saved)"
            root={root}
            dir={ISSUES_DIR}
            path="new-issue.md"
            index={index}
            cross={cross}
            onChange={(body) => set({ body })}
            onNavigate={onFollowLink}
            onTagClick={() => {}}
          />
        </div>
        <div className="issue-composer-foot">
          <button
            className="btn-secondary"
            title="Throw this draft away"
            disabled={saving}
            onClick={onDiscard}
          >
            Discard
          </button>
          <span className="issue-composer-hint">
            The draft waits here while you work elsewhere.
          </span>
          <div className="spacer" />
          <button
            className="btn-primary"
            title={draft.title.trim() ? "Create the issue (⌘Enter)" : "Give the issue a title first"}
            disabled={!canSave}
            onClick={onSave}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </section>
      {menu && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setMenu(null)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
            {menu === "status" &&
              ISSUE_STATUSES.map((s) => (
                <button key={s} onClick={() => { setMenu(null); set({ status: s }); }}>
                  <StatusDot status={s} /> {STATUS_LABELS[s]}{s === draft.status ? " ✓" : ""}
                </button>
              ))}
            {menu === "priority" &&
              PRIORITY_LABELS.map((p, n) => (
                <button key={p} onClick={() => { setMenu(null); set({ priority: n }); }}>
                  <PriorityGlyph priority={n} /> {p}{n === draft.priority ? " ✓" : ""}
                </button>
              ))}
          </div>
        </>
      )}
    </div>
  );
}
