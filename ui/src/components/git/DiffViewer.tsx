import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileDiff, gitParseDiff, gitCommitDiff, gitStageHunk, gitUnstageHunk,
  gitStageLines, gitUnstageLines, gitRevertLines, addReviewComment,
} from "../../api";
import { buildRows, type DiffRow, type Span } from "./diffModel";
import { highlightLine, langForPath } from "./highlight";

type Mode = "working-unstaged" | "working-staged" | "commit";

// Above this many rows we skip syntax highlighting entirely. A root commit's
// diff is the *whole file* as additions (git show has no parent to diff
// against), so the auto-selected first file can be thousands of lines — Shiki's
// per-line codeToHtml is far too expensive to run across all of them.
const MAX_HIGHLIGHT_ROWS = 3000;
// Yield to the event loop after this many rows so input/paint aren't starved.
// Each highlightLine resolves on the microtask queue once Shiki is loaded, and
// the browser drains all microtasks before painting, so without a macrotask
// break (setTimeout) a long loop freezes the UI even though it is `await`ed.
const HIGHLIGHT_CHUNK = 200;

function spansToText(spans: Span[] | null): string {
  return spans ? spans.map((s) => s.text).join("") : "";
}

export default function DiffViewer({
  taskId, path, mode, hash, onChanged, onCommentAdded, allowComments = true,
}: {
  taskId: string;
  path: string;
  mode: Mode;
  hash?: string;
  onChanged: () => void;
  onCommentAdded?: () => void;
  allowComments?: boolean;
}) {
  const [fd, setFd] = useState<FileDiff | null>(null);
  const [rows, setRows] = useState<DiffRow[]>([]);
  const [highlighted, setHighlighted] = useState<Record<string, string>>({});
  const [themeTick, setThemeTick] = useState(0);
  const [error, setError] = useState("");
  const [sideBySide, setSideBySide] = useState(true);
  const [sel, setSel] = useState<{ hunk: number; lines: Set<number> } | null>(null);
  const [commenting, setCommenting] = useState(false);
  const [draft, setDraft] = useState("");
  const wrapRef = useRef<HTMLDivElement>(null);
  const staged = mode === "working-staged";
  const readonly = mode === "commit";

  const load = useCallback(async () => {
    try {
      const parsed = readonly && hash
        ? parseInto(await gitCommitDiff(taskId, hash, path))
        : await gitParseDiff(taskId, path, staged);
      setFd(parsed);
      setRows(buildRows(parsed));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId, path, staged, readonly, hash]);

  useEffect(() => { load(); }, [load]);

  // collapse to inline when narrow
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setSideBySide(entry.contentRect.width >= 900));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // re-highlight when the app theme changes (dark/light Shiki theme swap)
  useEffect(() => {
    const onTheme = () => setThemeTick((t) => t + 1);
    window.addEventListener("themechange", onTheme);
    return () => window.removeEventListener("themechange", onTheme);
  }, []);

  // syntax highlight (async, best-effort). Skips outright for very large diffs
  // (e.g. a root commit's full-file diff) and yields to the event loop as it
  // goes, so highlighting never freezes the UI. Plain text renders regardless
  // via the fallback path in DiffLineRow.
  useEffect(() => {
    if (rows.length > MAX_HIGHLIGHT_ROWS) { setHighlighted({}); return; }
    let cancelled = false;
    const lang = langForPath(path);
    (async () => {
      const out: Record<string, string> = {};
      let sinceYield = 0;
      for (const r of rows) {
        if (cancelled) return;
        for (const [side, spans] of [["o", r.oldSpans], ["n", r.newSpans]] as const) {
          const text = spansToText(spans);
          const key = `${side}:${r.lineIndex}`;
          if (text && out[key] === undefined) {
            try { out[key] = await highlightLine(text, lang); } catch { /* best-effort: leave key absent, falls back to plain text */ }
          }
        }
        if (++sinceYield >= HIGHLIGHT_CHUNK) {
          sinceYield = 0;
          setHighlighted({ ...out }); // progressive reveal
          await new Promise((res) => setTimeout(res, 0)); // let the UI breathe
        }
      }
      if (!cancelled) setHighlighted(out);
    })();
    return () => { cancelled = true; };
  }, [rows, path, themeTick]);

  async function hunkAction(hunkIndex: number) {
    try {
      if (staged) await gitUnstageHunk(taskId, path, hunkIndex);
      else await gitStageHunk(taskId, path, hunkIndex);
      await load(); onChanged();
    } catch (e) { setError(String(e)); }
  }

  async function applySelection(action: "stage" | "unstage" | "revert") {
    if (!sel || sel.lines.size === 0) return;
    const lines = [...sel.lines];
    try {
      if (action === "stage") await gitStageLines(taskId, path, sel.hunk, lines);
      else if (action === "unstage") await gitUnstageLines(taskId, path, sel.hunk, lines);
      else await gitRevertLines(taskId, path, sel.hunk, lines);
      setSel(null); await load(); onChanged();
    } catch (e) { setError(String(e)); }
  }

  function selectedRange(): { start: number; end: number } | null {
    if (!sel || sel.lines.size === 0) return null;
    const nums = rows
      .filter((r) => r.hunkIndex === sel.hunk && sel.lines.has(r.lineIndex))
      .map((r) => r.newNo ?? r.oldNo)
      .filter((n): n is number => n != null);
    if (nums.length === 0) return null;
    return { start: Math.min(...nums), end: Math.max(...nums) };
  }

  async function saveComment() {
    const range = selectedRange();
    if (!range || !draft.trim()) return;
    try {
      await addReviewComment(taskId, path, range.start, range.end, draft.trim());
      setDraft("");
      setCommenting(false);
      setSel(null);
      onCommentAdded?.();
    } catch (e) { setError(String(e)); }
  }

  function toggleLine(hunk: number, lineIndex: number) {
    if (readonly) return;
    setSel((prev) => {
      const lines = new Set(prev && prev.hunk === hunk ? prev.lines : []);
      lines.has(lineIndex) ? lines.delete(lineIndex) : lines.add(lineIndex);
      return { hunk, lines };
    });
  }

  if (error) return <div className="git-error">{error}</div>;
  if (!fd) return <div className="diff-empty">loading…</div>;
  if (rows.length === 0) return <div className="diff-empty">no textual changes</div>;

  return (
    <div className="diffviewer" ref={wrapRef}>
      <div className="diff-toolbar">
        <span className="diff-path">{path}</span>
        <span className="spacer" style={{ flex: 1 }} />
        {sel && sel.lines.size > 0 && !readonly && (
          <span className="diff-sel-actions">
            {!staged && <button className="git-iconbtn" onClick={() => applySelection("stage")}>Stage selection</button>}
            {staged && <button className="git-iconbtn" onClick={() => applySelection("unstage")}>Unstage selection</button>}
            {!staged && <button className="git-iconbtn" onClick={() => applySelection("revert")}>Revert selection</button>}
            {allowComments && <button className="git-iconbtn" onClick={() => setCommenting(true)}>Comment</button>}
          </span>
        )}
        <button className="git-iconbtn" onClick={() => setSideBySide((s) => !s)}>
          {sideBySide ? "Inline" : "Side by side"}
        </button>
      </div>
      {allowComments && commenting && sel && sel.lines.size > 0 && (
        <div className="diff-comment-box">
          <textarea
            className="settings-input"
            placeholder="Comment for the agent on the selected lines…"
            value={draft}
            autoFocus
            onChange={(e) => setDraft(e.target.value)}
          />
          <div className="diff-comment-actions">
            <button className="git-iconbtn" onClick={saveComment}>Add comment</button>
            <button className="git-iconbtn" onClick={() => { setCommenting(false); setDraft(""); }}>Cancel</button>
          </div>
        </div>
      )}
      <div className={`diff-body ${sideBySide ? "sxs" : "inline"}`}>
        {rows.flatMap((r) => {
          const isInlinePair = !sideBySide && r.oldSpans != null && r.newSpans != null;
          if (isInlinePair) {
            const shared = { sideBySide, highlighted, onToggle: () => toggleLine(r.hunkIndex, r.lineIndex) };
            const isSelected = !!sel && sel.hunk === r.hunkIndex && sel.lines.has(r.lineIndex);
            return [
              <DiffLineRow key={`${r.hunkIndex}:${r.lineIndex}:del`}
                row={{ ...r, newSpans: null, newNo: null, kind: "del" }}
                selected={isSelected} {...shared} />,
              <DiffLineRow key={`${r.hunkIndex}:${r.lineIndex}:add`}
                row={{ ...r, oldSpans: null, oldNo: null, kind: "add" }}
                selected={isSelected} {...shared} />,
            ];
          }
          return [
            <DiffLineRow key={`${r.hunkIndex}:${r.lineIndex}:row`}
              row={r} sideBySide={sideBySide} highlighted={highlighted}
              selected={!!sel && sel.hunk === r.hunkIndex && sel.lines.has(r.lineIndex)}
              onToggle={() => toggleLine(r.hunkIndex, r.lineIndex)} />,
          ];
        })}
      </div>
      {!readonly && (
        <div className="diff-hunks">
          {fd.hunks.map((_h, i) => (
            <button key={i} className="git-iconbtn" onClick={() => hunkAction(i)}>
              {staged ? "Unstage" : "Stage"} hunk {i + 1}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function DiffLineRow({ row, sideBySide, highlighted, selected, onToggle }: {
  row: DiffRow; sideBySide: boolean; highlighted: Record<string, string>;
  selected: boolean; onToggle: () => void;
}) {
  const oldHtml = highlighted[`o:${row.lineIndex}`];
  const newHtml = highlighted[`n:${row.lineIndex}`];
  const cell = (spans: Span[] | null, html: string | undefined, side: "old" | "new") => {
    if (!spans) return <div className={`diff-cell empty`} />;
    return (
      <div className={`diff-cell ${side}`}>
        {html !== undefined
          ? <span dangerouslySetInnerHTML={{ __html: wrapChanged(html, spans) }} />
          : <span>{spans.map((s, i) => <span key={i} className={s.changed ? "wd" : ""}>{s.text}</span>)}</span>}
      </div>
    );
  };
  return (
    <div className={`diff-line ${row.kind} ${selected ? "sel" : ""}`} onClick={onToggle}>
      <span className="diff-gutter">{row.oldNo ?? ""}</span>
      <span className="diff-gutter">{row.newNo ?? ""}</span>
      {sideBySide
        ? <>{cell(row.oldSpans, oldHtml, "old")}{cell(row.newSpans, newHtml, "new")}</>
        : cell(row.newSpans ?? row.oldSpans, newHtml ?? oldHtml, row.newSpans ? "new" : "old")}
    </div>
  );
}

// When highlighted HTML is present we cannot easily inject word-diff spans;
// fall back to a line-level wash (handled by .diff-line.add/.del CSS). The word
// overlay only applies in the non-highlighted span path above (class "wd").
function wrapChanged(html: string, _spans: Span[]): string { return html; }

function parseInto(raw: string): FileDiff {
  // Local mirror of git.parse_diff for commit_diff output.
  const lines = raw.split("\n");
  const header: string[] = [];
  const hunks: FileDiff["hunks"] = [];
  let current: FileDiff["hunks"][number] | null = null;
  let seen = false;
  for (const line of lines) {
    if (line.startsWith("@@")) {
      seen = true;
      if (current) hunks.push(current);
      current = { header: line, lines: [] };
    } else if (current) current.lines.push(line);
    else if (!seen) header.push(line);
  }
  if (current) hunks.push(current);
  return { header: header.length ? header.join("\n") + "\n" : "", hunks };
}
