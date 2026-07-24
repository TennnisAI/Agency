import { Fragment, useEffect, useMemo, useState } from "react";
import { PrFileDiff, ReviewThread } from "../../api";
import { buildRows, type DiffRow, type Span } from "../git/diffModel";
import { highlightLine, langForPath } from "../git/highlight";
import { computeAnchor, type DraftEntry, type Side } from "./anchor";
import PrThread from "./PrThread";

// Above this many rows we skip highlighting (Shiki per-line is too costly on a
// huge file); the word-diff spans still render. Yield to the event loop every
// chunk so a long file never freezes the UI. Mirrors DiffViewer's approach.
const MAX_HIGHLIGHT_ROWS = 3000;
const HIGHLIGHT_CHUNK = 200;

const spansText = (spans: Span[] | null): string => (spans ? spans.map((s) => s.text).join("") : "");

// One file of a PR diff. Reuses the pure `buildRows` from the local diff
// renderer, adds PR-review affordances (line selection → inline draft comment,
// existing threads under their anchored lines, an outdated-thread bucket), and
// async Shiki highlighting keyed by row index.
export default function PrDiffFile({
  file,
  threads,
  drafts,
  onAddDraft,
  onRemoveDraft,
  onReply,
  onToggleResolved,
}: {
  file: PrFileDiff;
  threads: ReviewThread[];
  drafts: DraftEntry[];
  onAddDraft: (d: Omit<DraftEntry, "id">) => void;
  onRemoveDraft: (id: string) => void;
  onReply: (inReplyTo: number, body: string) => Promise<void>;
  onToggleResolved: (threadId: string, resolved: boolean) => Promise<void>;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [sel, setSel] = useState<{ start: number; end: number } | null>(null);
  const [commenting, setCommenting] = useState(false);
  const [draft, setDraft] = useState("");
  const [highlighted, setHighlighted] = useState<Record<string, string>>({});
  const [themeTick, setThemeTick] = useState(0);

  const rows = useMemo(() => buildRows(file), [file]);

  // Anchor a GitHub (line, side) to a rendered row: RIGHT → the new-file line,
  // LEFT → the old-file line. Both threads and drafts carry an explicit side, so
  // a deletion-side comment lands on the right row instead of guessing.
  const rowForLineSide = useMemo(() => {
    return (line: number, side: Side): number =>
      side === "LEFT" ? rows.findIndex((r) => r.oldNo === line) : rows.findIndex((r) => r.newNo === line);
  }, [rows]);

  const { threadsAt, outdated } = useMemo(() => {
    const at = new Map<number, ReviewThread[]>();
    const out: ReviewThread[] = [];
    for (const t of threads) {
      const side: Side = t.diffSide === "LEFT" ? "LEFT" : "RIGHT";
      const idx = t.line != null ? rowForLineSide(t.line, side) : -1;
      if (idx === -1) out.push(t);
      else at.set(idx, [...(at.get(idx) ?? []), t]);
    }
    return { threadsAt: at, outdated: out };
  }, [threads, rowForLineSide]);

  const draftsAt = useMemo(() => {
    const at = new Map<number, DraftEntry[]>();
    for (const d of drafts) {
      const idx = rowForLineSide(d.line, d.side);
      if (idx !== -1) at.set(idx, [...(at.get(idx) ?? []), d]);
    }
    return at;
  }, [drafts, rowForLineSide]);

  // Async, chunked syntax highlighting. Plain text renders regardless via the
  // fallback in `line()`; large files skip highlighting entirely.
  useEffect(() => {
    if (collapsed || file.binary || rows.length > MAX_HIGHLIGHT_ROWS) {
      setHighlighted({});
      return;
    }
    let cancelled = false;
    const lang = langForPath(file.path);
    (async () => {
      const out: Record<string, string> = {};
      let since = 0;
      for (let i = 0; i < rows.length; i++) {
        if (cancelled) return;
        for (const [k, spans] of [["o", rows[i].oldSpans], ["n", rows[i].newSpans]] as const) {
          const text = spansText(spans);
          const key = `${k}:${i}`;
          if (text && out[key] === undefined) {
            try { out[key] = await highlightLine(text, lang); } catch { /* leave plain */ }
          }
        }
        if (++since >= HIGHLIGHT_CHUNK) {
          since = 0;
          setHighlighted({ ...out });
          await new Promise((res) => setTimeout(res, 0));
        }
      }
      if (!cancelled) setHighlighted(out);
    })();
    return () => { cancelled = true; };
  }, [rows, file.path, file.binary, collapsed, themeTick]);

  useEffect(() => {
    const onTheme = () => setThemeTick((t) => t + 1);
    window.addEventListener("themechange", onTheme);
    return () => window.removeEventListener("themechange", onTheme);
  }, []);

  const [selLo, selHi] = sel ? [Math.min(sel.start, sel.end), Math.max(sel.start, sel.end)] : [-1, -1];

  function onRowClick(e: React.MouseEvent, i: number) {
    if (e.shiftKey && sel) setSel({ start: sel.start, end: i });
    else setSel({ start: i, end: i });
    setCommenting(false);
  }

  function saveComment() {
    if (!sel || !draft.trim()) return;
    const anchor = computeAnchor(rows.slice(selLo, selHi + 1));
    if (!anchor) return;
    onAddDraft({ path: file.path, body: draft.trim(), ...anchor });
    setDraft("");
    setCommenting(false);
    setSel(null);
  }

  return (
    <div className="prdiff-file">
      <div className="prdiff-file-head" onClick={() => setCollapsed((c) => !c)}>
        <span className="prdiff-caret">{collapsed ? "▸" : "▾"}</span>
        <span className="prdiff-path">{file.path}</span>
        {file.oldPath && <span className="prdiff-renamed">← {file.oldPath}</span>}
        {file.binary && <span className="prdiff-binary">binary</span>}
      </div>

      {!collapsed && file.binary && <div className="prdiff-emptyline">Binary file — not shown.</div>}

      {!collapsed && !file.binary && (
        <div className="prdiff-body">
          {rows.map((r, i) => {
            const selected = i >= selLo && i <= selHi;
            return (
              <Fragment key={i}>
                {rowVisuals(r, i, selected, highlighted, onRowClick)}
                {selected && i === selHi && !commenting && (
                  <div className="prdiff-selbar">
                    <button className="git-iconbtn" onClick={() => setCommenting(true)}>Comment on selection</button>
                    <button className="git-iconbtn" onClick={() => setSel(null)}>Clear</button>
                  </div>
                )}
                {commenting && i === selHi && (
                  <div className="prdiff-commentbox">
                    <textarea
                      className="settings-input"
                      autoFocus
                      placeholder="Leave a comment on the selected lines…"
                      value={draft}
                      onChange={(e) => setDraft(e.target.value)}
                    />
                    <div className="prdiff-comment-actions">
                      <button className="git-iconbtn" disabled={!draft.trim()} onClick={saveComment}>Add comment</button>
                      <button className="git-iconbtn" onClick={() => { setCommenting(false); setDraft(""); }}>Cancel</button>
                    </div>
                  </div>
                )}
                {(draftsAt.get(i) ?? []).map((d) => (
                  <div key={d.id} className="prdiff-draft">
                    <span className="prdiff-draft-tag">Pending</span>
                    <span className="prdiff-draft-body">{d.body}</span>
                    <button className="icon-btn" title="Remove pending comment" onClick={() => onRemoveDraft(d.id)}>✕</button>
                  </div>
                ))}
                {(threadsAt.get(i) ?? []).map((t) => (
                  <PrThread key={t.id} thread={t} onReply={onReply} onToggleResolved={onToggleResolved} />
                ))}
              </Fragment>
            );
          })}
        </div>
      )}

      {!collapsed && outdated.length > 0 && (
        <div className="prdiff-outdated">
          <div className="prdiff-outdated-head">Outdated / unanchored ({outdated.length})</div>
          {outdated.map((t) => (
            <PrThread key={t.id} thread={t} onReply={onReply} onToggleResolved={onToggleResolved} />
          ))}
        </div>
      )}
    </div>
  );
}

// A diff row becomes one visual line, except a paired "modify" row (kind "del"
// carrying a new side) splits into a deletion line above an addition line. When
// a highlighted HTML fragment exists for the line it's rendered (the line-level
// add/del wash conveys the change); otherwise the word-diff spans render.
function rowVisuals(
  r: DiffRow,
  i: number,
  selected: boolean,
  highlighted: Record<string, string>,
  onClick: (e: React.MouseEvent, i: number) => void,
) {
  const line = (kind: "del" | "add" | "ctx", hl: "o" | "n", oldNo: number | null, newNo: number | null, spans: Span[] | null) => {
    const html = highlighted[`${hl}:${i}`];
    return (
      <div
        key={`${i}:${kind}`}
        className={`prdiff-line ${kind}${selected ? " sel" : ""}`}
        onClick={(e) => onClick(e, i)}
      >
        <span className="prdiff-gutter">{oldNo ?? ""}</span>
        <span className="prdiff-gutter">{newNo ?? ""}</span>
        <span className="prdiff-sign">{kind === "add" ? "+" : kind === "del" ? "−" : " "}</span>
        {html !== undefined ? (
          <span className="prdiff-code" dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <span className="prdiff-code">
            {(spans ?? []).map((s, j) => (
              <span key={j} className={s.changed ? "wd" : ""}>{s.text}</span>
            ))}
          </span>
        )}
      </div>
    );
  };

  const isModify = r.kind === "del" && r.newSpans != null;
  if (isModify) return [line("del", "o", r.oldNo, null, r.oldSpans), line("add", "n", null, r.newNo, r.newSpans)];
  if (r.kind === "del") return [line("del", "o", r.oldNo, null, r.oldSpans)];
  if (r.kind === "add") return [line("add", "n", null, r.newNo, r.newSpans)];
  return [line("ctx", "n", r.oldNo, r.newNo, r.newSpans ?? r.oldSpans)];
}
