# Workspace Lifecycle — Plan 5: Review Comment → Agent

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a reviewer attach comments to specific diff lines, then send them to the agent as one precise message — closing the loop from "I see a problem here" to "agent, fix this".

**Architecture:** Comments persist in a new `review_comments` SQLite table keyed by run id. The `DiffViewer` (from the merged source-control redesign) gains a "Comment" action on the current line selection that captures `path` + line range + body. A `ReviewComments` panel lists them and offers "Send to agent", which composes a single-line markdown message and types it into the agent's tmux session via `send-keys`. Sent comments are marked.

**Tech Stack:** Rust (agency-core registry + tmux, agency-app/Tauri), `rusqlite`, `uuid`, React/TS.

**Spec:** `docs/superpowers/specs/2026-06-22-workspace-lifecycle-design.md` (Feature 4 — Review comment → agent). **Builds on Plans 1–4** (merged) and the merged `git-source-control-redesign` `DiffViewer`/`GitPanel`.

## Global Constraints

- Rust edition 2021. No AI/Claude attribution in any commit message (hard rule).
- Comments persist in `review_comments(id, run_id, path, line_start, line_end, body, sent, created_at)`.
- "Send to agent" composes ONE single-line message (avoids premature submit in TUI agents) and types it into the agent's tmux session (`agency-<id>`) via `tmux send-keys -l <text>` then `Enter`; on success the sent comments are marked `sent = 1`.
- Send requires a live agent session. If the session is gone, `send_review_comments` returns an error the UI surfaces.
- The comment "body" prompt is an inline textarea in the diff toolbar — NOT `window.prompt` (unreliable in the Tauri webview).

## Non-goals (explicit, deferred)

- **Re-run-with-feedback when the agent has exited.** Send targets a live session; if the agent exited, the UI tells the user to re-run the agent first, then send. Auto-respawning with the feedback as the prompt is a follow-up.
- **Embedding the diff hunk** in the message. The message carries `path:line` + body; the agent already has the diff in its worktree.

## File Structure

**Backend**
- **Modify** `crates/agency-core/src/registry.rs` — `ReviewComment` struct + `review_comments` table + CRUD.
- **Modify** `crates/agency-core/src/tmux.rs` — `send_text(name, text)`.
- **Modify** `crates/agency-app/src/state.rs` — `compose_feedback` (pure) + `add_review_comment`/`list_review_comments`/`delete_review_comment`/`send_review_comments`.
- **Modify** `crates/agency-app/src/commands.rs` — 4 commands.
- **Modify** `crates/agency-app/src/lib.rs` — register 4 commands.

**Frontend**
- **Modify** `ui/src/api.ts` — `ReviewComment` type + 4 wrappers.
- **Modify** `ui/src/components/git/DiffViewer.tsx` — Comment action + inline compose + line-range capture.
- **Create** `ui/src/components/git/ReviewComments.tsx` — list + Send + delete.
- **Modify** `ui/src/components/git/GitPanel.tsx` — render `ReviewComments`; refresh it when a comment is added.
- **Modify** `ui/src/styles.css` — comment styles.

---

### Task 1: `review_comments` table + registry CRUD

**Files:**
- Modify: `crates/agency-core/src/registry.rs`

**Interfaces:**
- Produces:
  - `ReviewComment { id, run_id, path: String, line_start: u32, line_end: u32, body: String, sent: bool, created_at: i64 }` (serde camelCase)
  - `Registry::insert_review_comment(&self, c: &ReviewComment) -> Result<()>`
  - `Registry::list_review_comments(&self, run_id: &str) -> Result<Vec<ReviewComment>>`
  - `Registry::list_unsent_review_comments(&self, run_id: &str) -> Result<Vec<ReviewComment>>`
  - `Registry::delete_review_comment(&self, id: &str) -> Result<()>`
  - `Registry::mark_review_comments_sent(&self, run_id: &str) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `registry.rs`:

```rust
    fn sample_comment(id: &str, run_id: &str, sent: bool) -> ReviewComment {
        ReviewComment {
            id: id.to_string(),
            run_id: run_id.to_string(),
            path: "src/main.rs".to_string(),
            line_start: 10,
            line_end: 12,
            body: "fix this".to_string(),
            sent,
            created_at: 5,
        }
    }

    #[test]
    fn review_comments_crud_and_filter() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_review_comment(&sample_comment("c1", "run-1", false)).unwrap();
        reg.insert_review_comment(&sample_comment("c2", "run-1", true)).unwrap();
        reg.insert_review_comment(&sample_comment("c3", "run-2", false)).unwrap();

        let all: Vec<String> = reg.list_review_comments("run-1").unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(all, vec!["c1", "c2"]);
        let unsent: Vec<String> = reg.list_unsent_review_comments("run-1").unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(unsent, vec!["c1"]);

        let got = reg.list_review_comments("run-1").unwrap();
        assert_eq!(got[0].line_start, 10);
        assert_eq!(got[0].line_end, 12);
        assert_eq!(got[0].sent, false);
        assert_eq!(got[1].sent, true);
    }

    #[test]
    fn mark_sent_and_delete() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_review_comment(&sample_comment("c1", "run-1", false)).unwrap();
        reg.insert_review_comment(&sample_comment("c2", "run-1", false)).unwrap();
        reg.mark_review_comments_sent("run-1").unwrap();
        assert!(reg.list_unsent_review_comments("run-1").unwrap().is_empty());

        reg.delete_review_comment("c1").unwrap();
        let ids: Vec<String> = reg.list_review_comments("run-1").unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["c2"]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core registry::`
Expected: FAIL to compile — `ReviewComment` and the methods are undefined.

- [ ] **Step 3: Add the struct**

Add near the `Run` struct in `registry.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub run_id: String,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub body: String,
    pub sent: bool,
    pub created_at: i64,
}
```

- [ ] **Step 4: Create the table**

In `Registry::open`, add to the `execute_batch(...)` schema string (after the `runs` table):

```rust
            CREATE TABLE IF NOT EXISTS review_comments (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                body TEXT NOT NULL,
                sent INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
```

- [ ] **Step 5: Add the CRUD methods**

Add to `impl Registry`:

```rust
    pub fn insert_review_comment(&self, c: &ReviewComment) -> Result<()> {
        self.conn.execute(
            "INSERT INTO review_comments (id, run_id, path, line_start, line_end, body, sent, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                c.id, c.run_id, c.path, c.line_start, c.line_end, c.body, c.sent as i64, c.created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_review_comments(&self, run_id: &str) -> Result<Vec<ReviewComment>> {
        self.query_review_comments(
            "SELECT id, run_id, path, line_start, line_end, body, sent, created_at
             FROM review_comments WHERE run_id = ?1 ORDER BY created_at",
            run_id,
        )
    }

    pub fn list_unsent_review_comments(&self, run_id: &str) -> Result<Vec<ReviewComment>> {
        self.query_review_comments(
            "SELECT id, run_id, path, line_start, line_end, body, sent, created_at
             FROM review_comments WHERE run_id = ?1 AND sent = 0 ORDER BY created_at",
            run_id,
        )
    }

    fn query_review_comments(&self, sql: &str, run_id: &str) -> Result<Vec<ReviewComment>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(ReviewComment {
                id: row.get(0)?,
                run_id: row.get(1)?,
                path: row.get(2)?,
                line_start: row.get(3)?,
                line_end: row.get(4)?,
                body: row.get(5)?,
                sent: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn delete_review_comment(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM review_comments WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn mark_review_comments_sent(&self, run_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE review_comments SET sent = 1 WHERE run_id = ?1 AND sent = 0",
            [run_id],
        )?;
        Ok(())
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p agency-core registry::`
Expected: PASS (existing + 2 new).

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/registry.rs
git commit -m "Add review_comments table and registry CRUD"
```

---

### Task 2: Send-text + state methods (compose, add, list, delete, send)

**Files:**
- Modify: `crates/agency-core/src/tmux.rs`
- Modify: `crates/agency-app/src/state.rs`

**Interfaces:**
- Consumes: Task 1 registry CRUD; `uuid` (already a dep of agency-core, re-exported via registry usage — use `uuid::Uuid` in state.rs).
- Produces:
  - `Tmux::send_text(&self, name: &str, text: &str) -> Result<()>`
  - free fn `compose_feedback(comments: &[ReviewComment]) -> String`
  - `AppState::add_review_comment(&self, run_id, path, line_start: u32, line_end: u32, body) -> Result<ReviewComment>`
  - `AppState::list_review_comments(&self, run_id: &str) -> Result<Vec<ReviewComment>>`
  - `AppState::delete_review_comment(&self, id: &str) -> Result<()>`
  - `AppState::send_review_comments(&self, run_id: &str) -> Result<()>`

- [ ] **Step 1: Add `send_text` to Tmux**

In `crates/agency-core/src/tmux.rs`, add a method to `impl Tmux`:

```rust
    /// Type literal `text` into the session followed by Enter. Errors if the
    /// session does not exist.
    pub fn send_text(&self, name: &str, text: &str) -> Result<()> {
        if !self.session_exists(name)? {
            bail!("session {name} is not running");
        }
        self.ok(&["send-keys", "-t", name, "-l", text])?;
        self.ok(&["send-keys", "-t", name, "Enter"])?;
        Ok(())
    }
```

- [ ] **Step 2: Write the failing test for `compose_feedback`**

Add to the `#[cfg(test)] mod tests` in `state.rs`:

```rust
    use super::compose_feedback;
    use agency_core::registry::ReviewComment;

    fn rc(path: &str, a: u32, b: u32, body: &str) -> ReviewComment {
        ReviewComment {
            id: "x".into(), run_id: "r".into(), path: path.into(),
            line_start: a, line_end: b, body: body.into(), sent: false, created_at: 0,
        }
    }

    #[test]
    fn compose_feedback_is_single_line_with_locations() {
        let msg = compose_feedback(&[
            rc("src/a.rs", 10, 12, "rename this"),
            rc("src/b.rs", 5, 5, "remove dead code"),
        ]);
        assert!(!msg.contains('\n'), "must be single-line");
        assert!(msg.contains("src/a.rs:10-12"));
        assert!(msg.contains("src/b.rs:5"));
        assert!(!msg.contains("src/b.rs:5-5"), "equal start/end shows one number");
        assert!(msg.contains("rename this"));
        assert!(msg.contains("remove dead code"));
    }
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p agency-app compose_feedback`
Expected: FAIL to compile — `compose_feedback` undefined.

- [ ] **Step 4: Implement `compose_feedback` and the state methods**

Add the free function near the top of `state.rs` (after the imports):

```rust
/// Compose a single-line review-feedback message for the agent. Single-line so
/// TUI agents don't submit early on embedded newlines.
fn compose_feedback(comments: &[agency_core::registry::ReviewComment]) -> String {
    let parts: Vec<String> = comments
        .iter()
        .map(|c| {
            let loc = if c.line_end != c.line_start {
                format!("{}:{}-{}", c.path, c.line_start, c.line_end)
            } else {
                format!("{}:{}", c.path, c.line_start)
            };
            format!("[{}] {}", loc, c.body)
        })
        .collect();
    format!("Please address these review comments: {}", parts.join(" | "))
}
```

Add to `impl AppState`:

```rust
    pub fn add_review_comment(
        &self,
        run_id: &str,
        path: &str,
        line_start: u32,
        line_end: u32,
        body: &str,
    ) -> Result<agency_core::registry::ReviewComment> {
        let comment = agency_core::registry::ReviewComment {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            path: path.to_string(),
            line_start,
            line_end,
            body: body.to_string(),
            sent: false,
            created_at: now_secs(),
        };
        self.registry.lock().unwrap().insert_review_comment(&comment)?;
        Ok(comment)
    }

    pub fn list_review_comments(&self, run_id: &str) -> Result<Vec<agency_core::registry::ReviewComment>> {
        self.registry.lock().unwrap().list_review_comments(run_id)
    }

    pub fn delete_review_comment(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().delete_review_comment(id)
    }

    /// Type the unsent comments into the agent's live session and mark them sent.
    pub fn send_review_comments(&self, run_id: &str) -> Result<()> {
        let unsent = self.registry.lock().unwrap().list_unsent_review_comments(run_id)?;
        if unsent.is_empty() {
            bail!("no unsent review comments");
        }
        let message = compose_feedback(&unsent);
        self.tmux.send_text(&session_name(run_id), &message)?;
        self.registry.lock().unwrap().mark_review_comments_sent(run_id)?;
        Ok(())
    }
```

`uuid` is already a dependency of `agency-core`; confirm `agency-app/Cargo.toml` has `uuid` — if not, add `uuid = { version = "1", features = ["v4"] }`. (The brief's implementer should check and add it if missing.)

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p agency-app compose_feedback`
Expected: PASS.
Run: `cargo build && cargo test -p agency-core`
Expected: compiles; core suite green.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/tmux.rs crates/agency-app/src/state.rs crates/agency-app/Cargo.toml
git commit -m "Add review-comment state methods and tmux send-text"
```

---

### Task 3: Review-comment commands

**Files:**
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 `AppState` methods.
- Produces: tauri commands `add_review_comment`, `list_review_comments`, `delete_review_comment`, `send_review_comments`.

- [ ] **Step 1: Add the commands**

Append to `crates/agency-app/src/commands.rs` (import the type at the top of the file or reference it fully):

```rust
use agency_core::registry::ReviewComment;

#[tauri::command]
pub fn add_review_comment(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    line_start: u32,
    line_end: u32,
    body: String,
) -> Result<ReviewComment, String> {
    state
        .add_review_comment(&run_id, &path, line_start, line_end, &body)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_review_comments(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<ReviewComment>, String> {
    state.list_review_comments(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_review_comment(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_review_comment(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_review_comments(state: State<'_, AppState>, run_id: String) -> Result<(), String> {
    state.send_review_comments(&run_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register them in `lib.rs`**

Add to the `generate_handler!` list (after `commands::save_notif_settings,`):

```rust
            commands::add_review_comment,
            commands::list_review_comments,
            commands::delete_review_comment,
            commands::send_review_comments,
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "Expose review-comment commands"
```

---

### Task 4: Review-comment UI (diff action + comments panel)

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/components/git/DiffViewer.tsx`
- Create: `ui/src/components/git/ReviewComments.tsx`
- Modify: `ui/src/components/git/GitPanel.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: Task 3 commands.
- Produces: `ReviewComment` type + wrappers; a Comment action in `DiffViewer`; `ReviewComments` panel.

- [ ] **Step 1: Add api wrappers**

In `ui/src/api.ts`:

```ts
export interface ReviewComment {
  id: string;
  runId: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  body: string;
  sent: boolean;
  createdAt: number;
}

export const addReviewComment = (
  runId: string, path: string, lineStart: number, lineEnd: number, body: string,
) => invoke<ReviewComment>("add_review_comment", { runId, path, lineStart, lineEnd, body });
export const listReviewComments = (runId: string) =>
  invoke<ReviewComment[]>("list_review_comments", { runId });
export const deleteReviewComment = (id: string) =>
  invoke<void>("delete_review_comment", { id });
export const sendReviewComments = (runId: string) =>
  invoke<void>("send_review_comments", { runId });
```

- [ ] **Step 2: Add the Comment action to `DiffViewer`**

In `ui/src/components/git/DiffViewer.tsx`:

Add `addReviewComment` to the api import (line 2-5 block):

```ts
import {
  FileDiff, gitParseDiff, gitCommitDiff, gitStageHunk, gitUnstageHunk,
  gitStageLines, gitUnstageLines, gitRevertLines, addReviewComment,
} from "../../api";
```

Add an `onCommentAdded` prop. Change the component signature:

```ts
export default function DiffViewer({
  taskId, path, mode, hash, onChanged, onCommentAdded,
}: {
  taskId: string;
  path: string;
  mode: Mode;
  hash?: string;
  onChanged: () => void;
  onCommentAdded?: () => void;
}) {
```

Add compose state near the other `useState` calls:

```ts
  const [commenting, setCommenting] = useState(false);
  const [draft, setDraft] = useState("");
```

Add a helper that maps the current selection to a file line range, and the add handler (place after `applySelection`):

```ts
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
```

In the `diff-sel-actions` toolbar block, add a Comment button after the revert button:

```tsx
            {!staged && <button className="git-iconbtn" onClick={() => applySelection("revert")}>Revert selection</button>}
            <button className="git-iconbtn" onClick={() => setCommenting(true)}>Comment</button>
```

Add the inline compose box right after the `diff-toolbar` div (before `diff-body`):

```tsx
      {commenting && sel && sel.lines.size > 0 && (
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
```

- [ ] **Step 3: Create `ReviewComments.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";
import { ReviewComment, listReviewComments, deleteReviewComment, sendReviewComments } from "../../api";

export default function ReviewComments({ taskId }: { taskId: string }) {
  const [items, setItems] = useState<ReviewComment[]>([]);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      setItems(await listReviewComments(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { load(); }, [load]);

  if (items.length === 0) return null;
  const unsent = items.filter((c) => !c.sent);

  return (
    <div className="review-comments">
      <div className="review-head">
        <span>Review comments</span>
        <span className="spacer" style={{ flex: 1 }} />
        {unsent.length > 0 && (
          <button
            className="git-iconbtn"
            onClick={async () => {
              try { await sendReviewComments(taskId); await load(); }
              catch (e) { setError(String(e)); }
            }}
          >Send {unsent.length} to agent</button>
        )}
      </div>
      {error && <div className="git-error">{error}</div>}
      {items.map((c) => (
        <div key={c.id} className={`review-row ${c.sent ? "sent" : ""}`}>
          <code className="review-loc">
            {c.path}:{c.lineStart}{c.lineEnd !== c.lineStart ? `-${c.lineEnd}` : ""}
          </code>
          <span className="review-body">{c.body}</span>
          <button
            className="icon-btn"
            title="Delete comment"
            onClick={async () => { await deleteReviewComment(c.id); await load(); }}
          >✕</button>
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Wire `ReviewComments` into `GitPanel`**

In `ui/src/components/git/GitPanel.tsx`:

Add the import:

```ts
import ReviewComments from "./ReviewComments";
```

Add a refresh key state (with the other `useState` calls):

```ts
  const [commentsKey, setCommentsKey] = useState(0);
```

Pass `onCommentAdded` to BOTH `DiffViewer` usages (compact line 70 and full line 91):

```tsx
            <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />
```

Render `ReviewComments` in the full layout's left column (after the `{tab === "changes" ? changesPanel : historyPanel}` line, still inside `git-full-left`):

```tsx
          <ReviewComments key={commentsKey} taskId={taskId} />
```

And in the compact layout (after the tab panel line, inside the `aside`):

```tsx
        <ReviewComments key={commentsKey} taskId={taskId} />
```

- [ ] **Step 5: Add styles**

Append to `ui/src/styles.css`:

```css
.diff-comment-box { display: flex; flex-direction: column; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border, #2a2d36); }
.diff-comment-box textarea { min-height: 52px; }
.diff-comment-actions { display: flex; gap: 6px; }
.review-comments { border-top: 1px solid var(--border, #2a2d36); padding: 6px 8px; }
.review-head { display: flex; align-items: center; gap: 8px; color: var(--muted, #9aa); font-size: 12px; margin-bottom: 4px; }
.review-row { display: flex; align-items: center; gap: 8px; padding: 2px 0; }
.review-row.sent { opacity: 0.5; }
.review-loc { color: var(--muted, #9aa); white-space: nowrap; }
.review-body { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
```

- [ ] **Step 6: Typecheck + build**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm build`
Expected: no type errors; build succeeds.

- [ ] **Step 7: Manual smoke test**

1. Open a run with changes in the Source Control tab. Select one or more lines in a file's diff → click **Comment** → type a note → **Add comment**.
2. Confirm it appears in the **Review comments** panel as `path:line — body`.
3. With the agent session live, click **Send N to agent** → confirm the agent's terminal receives a single-line "Please address these review comments: …" message and the comments grey out (sent).
4. Add another comment; delete it with ✕ → confirm it disappears.
5. Stop the agent, add a comment, click Send → confirm an error surfaces ("session … is not running"); the comment stays unsent.

- [ ] **Step 8: Commit**

```bash
git add ui/src/api.ts ui/src/components/git/DiffViewer.tsx ui/src/components/git/ReviewComments.tsx ui/src/components/git/GitPanel.tsx ui/src/styles.css
git commit -m "Add review-comment UI: diff action and comments panel"
```

---

## Self-Review

**Spec coverage (Feature 4):**
- `review_comments` table (run_id, path, line range, body, sent) → Task 1. ✓
- Select diff lines → add comment, persisted → Tasks 1/2 + Task 4 (DiffViewer). ✓
- Comments list + per-comment delete → Task 4 (`ReviewComments`). ✓
- "Send to agent" composes one message and writes to the agent PTY; sent marked → Task 2 (`compose_feedback` + `send_text` + `send_review_comments`). ✓
- Commands `add`/`list`/`delete`/`send` → Task 3. ✓
- Agent-exited path: send surfaces an error; user re-runs then sends (re-run-with-feedback deferred per non-goals). ✓ (documented limitation)

**Placeholder scan:** none — every step carries concrete code/commands.

**Type consistency:** `ReviewComment` is one shape: Rust `registry::ReviewComment` (serde camelCase: `runId`/`lineStart`/`lineEnd`/`createdAt`) ↔ TS `ReviewComment`. The 4 command names match across `commands.rs` (Task 3), `lib.rs` (Task 3), `api.ts` (Task 4): `add_review_comment`/`list_review_comments`/`delete_review_comment`/`send_review_comments`. `compose_feedback` (Task 2) is consumed by `send_review_comments` (Task 2). `onCommentAdded` (Task 4 DiffViewer) is supplied by GitPanel (Task 4).

**Task dependencies:** 1→2 (registry before state) → 3 (commands) → 4 (frontend). Sequential.

**Deferred:** re-run-with-feedback when the agent is gone; embedding the diff hunk in the message; multi-line message formatting (single-line chosen to avoid TUI premature-submit).
