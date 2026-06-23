# Files Mode, Resizable Source-Control Panes, and Terminal Newline Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a top-level Files mode (lazy file tree + editable CodeMirror viewer) keyed to the focused agent's worktree (falling back to the project's main checkout), make the full Source Control split resizable, and fix the extra blank lines injected when opening an agent terminal.

**Architecture:** File operations live as pure functions in `agency-core` (`files.rs`, tested with tempdirs like `git`), wrapped by thin Tauri commands in `agency-app` that resolve a `FileRoot` selector (run worktree or project repo) to a base directory. The React UI gains a third `Tab` (`"files"`) rendering a `FilesView` (tree + editor + resizer). The two fixes are localized edits to `FocusTerminal.tsx` and `GitPanel.tsx`/CSS.

**Tech Stack:** Rust (Tauri 2, serde, anyhow), React 18 + TypeScript + Vite, CodeMirror 6, existing `Resizer`/`usePaneWidth` primitives.

## Global Constraints

- **No path escape:** every file command must reject `..` traversal and absolute paths; the resolved target must stay within the resolved base directory.
- **Lazy tree only:** never walk the worktree recursively/eagerly — list exactly one directory level per request (the tree is unfiltered and includes `node_modules`).
- **serde casing:** Rust DTOs returned to / received from the UI use `#[serde(rename_all = "camelCase")]`; Tauri command args are snake_case in Rust and camelCase in JS (e.g. `rel_path` ↔ `relPath`).
- **Commit messages:** do NOT add any AI/Claude attribution, `Co-Authored-By`, or "Generated with" trailers (standing user rule).
- **Concurrency note:** `main` is receiving commits from another session during this work. Execute this plan in an isolated git worktree (see Execution Handoff) and rebase before merging. Anchor edits by symbol/pattern, not line number — line numbers in this plan are indicative only.

---

### Task 1: Core file operations (`agency-core::files`)

Pure, filesystem-level functions with traversal protection, unit-tested with tempdirs.

**Files:**
- Create: `crates/agency-core/src/files.rs`
- Modify: `crates/agency-core/src/lib.rs` (add `pub mod files;`)
- Test: `crates/agency-core/tests/files.rs`

**Interfaces:**
- Produces:
  - `struct DirEntry { name: String, is_dir: bool }` (Serialize, camelCase → `{ name, isDir }`)
  - `struct FileContents { text: String, binary: bool, too_large: bool }` (Serialize, camelCase → `{ text, binary, tooLarge }`)
  - `fn list_dir(root: &Path, rel: &str) -> anyhow::Result<Vec<DirEntry>>` — one level, dirs-first then name-sorted
  - `fn read_file(root: &Path, rel: &str) -> anyhow::Result<FileContents>`
  - `fn write_file(root: &Path, rel: &str, contents: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/tests/files.rs`:

```rust
use agency_core::files;
use std::fs;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
    fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 0, 3]).unwrap();
    dir
}

#[test]
fn list_dir_sorts_dirs_first_then_name() {
    let dir = fixture();
    let entries = files::list_dir(dir.path(), "").unwrap();
    let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
    assert_eq!(names, vec![("src", true), ("README.md", false), ("bin.dat", false)]);
}

#[test]
fn list_dir_reads_subdirectory() {
    let dir = fixture();
    let entries = files::list_dir(dir.path(), "src").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "main.rs");
    assert!(!entries[0].is_dir);
}

#[test]
fn read_file_returns_text() {
    let dir = fixture();
    let fc = files::read_file(dir.path(), "src/main.rs").unwrap();
    assert_eq!(fc.text, "fn main() {}\n");
    assert!(!fc.binary && !fc.too_large);
}

#[test]
fn read_file_flags_binary() {
    let dir = fixture();
    let fc = files::read_file(dir.path(), "bin.dat").unwrap();
    assert!(fc.binary);
    assert_eq!(fc.text, "");
}

#[test]
fn write_file_roundtrips() {
    let dir = fixture();
    files::write_file(dir.path(), "README.md", "# changed\n").unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("README.md")).unwrap(), "# changed\n");
}

#[test]
fn rejects_parent_traversal() {
    let dir = fixture();
    assert!(files::read_file(dir.path(), "../secret").is_err());
    assert!(files::list_dir(dir.path(), "../").is_err());
    assert!(files::write_file(dir.path(), "../../x", "no").is_err());
}

#[test]
fn rejects_absolute_path() {
    let dir = fixture();
    assert!(files::read_file(dir.path(), "/etc/passwd").is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core --test files`
Expected: FAIL — `unresolved import agency_core::files` / module does not exist.

- [ ] **Step 3: Implement `files.rs`**

Create `crates/agency-core/src/files.rs`:

```rust
use anyhow::{bail, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// Files larger than this are reported as `too_large` rather than read.
const MAX_FILE_BYTES: u64 = 2_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContents {
    pub text: String,
    pub binary: bool,
    pub too_large: bool,
}

/// Join `rel` onto `root`, rejecting any component that could escape `root`
/// (parent dirs, absolute paths, drive prefixes). Does not require the target
/// to exist, so it is safe for writing new files.
fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => normalized.push(c),
            Component::CurDir => {}
            Component::ParentDir => bail!("path escapes root: {rel}"),
            Component::RootDir | Component::Prefix(_) => bail!("absolute path not allowed: {rel}"),
        }
    }
    Ok(root.join(normalized))
}

/// List a single directory level. Directories sort before files; ties broken by name.
pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<DirEntry>> {
    let dir = resolve_within(root, rel)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        out.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: entry.file_type()?.is_dir(),
        });
    }
    out.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    Ok(out)
}

/// Read a file's contents. Oversized files are flagged `too_large`; files
/// containing a NUL byte are flagged `binary`. In both cases `text` is empty.
pub fn read_file(root: &Path, rel: &str) -> Result<FileContents> {
    let path = resolve_within(root, rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Ok(FileContents { text: String::new(), binary: false, too_large: true });
    }
    let bytes = std::fs::read(&path)?;
    if bytes.contains(&0u8) {
        return Ok(FileContents { text: String::new(), binary: true, too_large: false });
    }
    Ok(FileContents {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        binary: false,
        too_large: false,
    })
}

/// Write `contents` to the file at `rel` within `root`.
pub fn write_file(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    std::fs::write(&path, contents)?;
    Ok(())
}
```

- [ ] **Step 4: Register the module**

In `crates/agency-core/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod files;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p agency-core --test files`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/files.rs crates/agency-core/src/lib.rs crates/agency-core/tests/files.rs
git commit -m "feat(core): file tree + read/write ops with traversal guard"
```

---

### Task 2: Tauri file commands + JS API wrappers

Wrap Task 1 in commands that resolve a `FileRoot` (run worktree or project repo), and expose typed wrappers to the UI.

**Files:**
- Modify: `crates/agency-app/src/commands.rs` (add `FileRoot`, `resolve_root`, 3 commands)
- Modify: `crates/agency-app/src/lib.rs` (register commands in `generate_handler!`)
- Modify: `ui/src/api.ts` (types + wrappers)

**Interfaces:**
- Consumes: `agency_core::files::{list_dir, read_file, write_file, DirEntry, FileContents}` (Task 1); `AppState::worktree_path(&str)` and `AppState::project_repo_path(&str)` (existing).
- Produces (Rust): commands `list_dir`, `read_file`, `write_file`; `enum FileRoot { Run { id }, Project { id } }`.
- Produces (TS):
  - `type FileRoot = { kind: "run"; id: string } | { kind: "project"; id: string }`
  - `interface DirEntry { name: string; isDir: boolean }`
  - `interface FileContents { text: string; binary: boolean; tooLarge: boolean }`
  - `listDir(root, relPath): Promise<DirEntry[]>`
  - `readFile(root, relPath): Promise<FileContents>`
  - `writeFile(root, relPath, contents): Promise<void>`

- [ ] **Step 1: Add the import for `Deserialize`**

In `crates/agency-app/src/commands.rs`, change the serde import line `use serde::Serialize;` to:

```rust
use serde::{Deserialize, Serialize};
```

- [ ] **Step 2: Add `FileRoot`, the resolver, and the three commands**

Append to `crates/agency-app/src/commands.rs`:

```rust
/// Selects which directory the file commands operate on: a run's worktree or a
/// project's main checkout.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileRoot {
    Run { id: String },
    Project { id: String },
}

fn resolve_root(state: &AppState, root: &FileRoot) -> Result<std::path::PathBuf, String> {
    match root {
        FileRoot::Run { id } => state.worktree_path(id).map_err(|e| e.to_string()),
        FileRoot::Project { id } => state.project_repo_path(id).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn list_dir(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<Vec<agency_core::files::DirEntry>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::list_dir(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_file(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<agency_core::files::FileContents, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::read_file(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_file(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
    contents: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::write_file(&base, &rel_path, &contents).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register the commands**

In `crates/agency-app/src/lib.rs`, inside the `tauri::generate_handler![ ... ]` list, add (after the existing `commands::send_review_comments,` entry):

```rust
            commands::list_dir,
            commands::read_file,
            commands::write_file,
```

- [ ] **Step 4: Verify the backend compiles**

Run: `cargo build -p agency-app`
Expected: builds with no errors.

- [ ] **Step 5: Add the TypeScript wrappers**

In `ui/src/api.ts`, add (near the other git wrappers):

```typescript
export type FileRoot = { kind: "run"; id: string } | { kind: "project"; id: string };

export interface DirEntry {
  name: string;
  isDir: boolean;
}

export interface FileContents {
  text: string;
  binary: boolean;
  tooLarge: boolean;
}

export const listDir = (root: FileRoot, relPath: string) =>
  invoke<DirEntry[]>("list_dir", { root, relPath });

export const readFile = (root: FileRoot, relPath: string) =>
  invoke<FileContents>("read_file", { root, relPath });

export const writeFile = (root: FileRoot, relPath: string, contents: string) =>
  invoke<void>("write_file", { root, relPath, contents });
```

- [ ] **Step 6: Verify the frontend typechecks**

Run: `cd ui && npx tsc --noEmit`
Expected: no type errors.

- [ ] **Step 7: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs ui/src/api.ts
git commit -m "feat(app): list_dir/read_file/write_file commands + JS api"
```

---

### Task 3: Fix extra newlines on agent open

Localized fix in `FocusTerminal`. Investigation-led per systematic-debugging; the concrete change is removing the force-appended newline and not double-painting the seeded screen.

**Files:**
- Modify: `ui/src/components/FocusTerminal.tsx` (the `stream.preview(...)` write, ~line 57)

**Interfaces:**
- Consumes: existing `TerminalStream.preview` / `attach`; no signature changes.

- [ ] **Step 1: Reproduce (systematic-debugging — confirm before changing)**

Run the app (`cargo tauri dev` or the project's run skill), create/focus an agent, and observe the terminal on open. Confirm the extra blank line(s) appear and note whether the visible screen content is duplicated (seed + attach redraw) or it's purely a trailing blank line. This determines whether Step 2 alone suffices.

- [ ] **Step 2: Apply the fix**

The current line writes the preview seed and force-appends a newline:

```typescript
stream.preview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed.endsWith("\n") ? seed : seed + "\n"); });
```

Replace it so the seed is written verbatim (no injected newline). tmux's `capture-pane` output already carries the trailing newline structure, and the live `attach` redraw owns the current screen — appending `\n` is what pushes content down:

```typescript
// Seed the pane's recent scrollback verbatim. Do NOT append a newline: the
// capture already ends where the live attach redraw begins, and an extra "\n"
// shows up as a blank line every time the agent is opened.
stream.preview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed); });
```

If Step 1 showed the *visible screen is duplicated* (capture tail + attach redraw paint the same rows twice), additionally gate the seed so it is written only before the attach lands, and clear it when attach takes over — implement by writing the seed, then on the `stream.attach(...).then(...)` callback (before wiring `onData`) issuing `term.reset()` so tmux's redraw paints a clean screen:

```typescript
stream.attach(runId, (bytes) => term.write(bytes)).then(() => {
  if (disposed) return;
  term.reset(); // let tmux's attach redraw own the screen; avoids double-paint
  onData = term.onData((d) => {
    // ...unchanged...
  });
  doFit();
});
```

Apply only the minimal subset that the Step 1 repro proves necessary. If Step 1 showed only a trailing blank line, the first change alone is the fix and `term.reset()` is not added.

- [ ] **Step 3: Verify against the repro**

Re-run the app, focus an agent. Expected: existing terminal content appears once, with no extra blank line above/below it, and scrollback is intact.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/FocusTerminal.tsx
git commit -m "fix(ui): stop injecting a blank line when opening an agent terminal"
```

---

### Task 4: Resizable full Source-Control split

Add a vertical resizer between the file/history list and the diff pane in the `full` GitPanel layout, persisted via `usePaneWidth`.

**Files:**
- Modify: `ui/src/components/git/GitPanel.tsx`
- Modify: `ui/src/styles.css` (`.git-full-left`)

**Interfaces:**
- Consumes: existing `Resizer` (`ui/src/components/Resizer.tsx`, vertical default) and `usePaneWidth` (`ui/src/hooks/usePaneWidth.ts`).

- [ ] **Step 1: Import the resizer primitives**

In `ui/src/components/git/GitPanel.tsx`, add to the imports:

```typescript
import Resizer from "../Resizer";
import { usePaneWidth } from "../../hooks/usePaneWidth";
```

- [ ] **Step 2: Add the width hook**

Inside `GitPanel`, after the existing `useState` declarations (e.g. after `const [commentsKey, setCommentsKey] = useState(0);`):

```typescript
const leftPane = usePaneWidth("git-full-left", 360, 300, 720);
```

- [ ] **Step 3: Wire the resizer into the full layout**

Replace the `<div className="git-full-body"> ... </div>` block in the `full` return with:

```tsx
<div className="git-full-body">
  <div className="git-full-left" style={{ width: leftPane.width }}>
    {sections}
    <ReviewComments key={commentsKey} taskId={taskId} />
  </div>
  <Resizer size={leftPane.width} min={300} max={720} onChange={leftPane.setWidth} />
  <div className="git-full-right">
    {selection?.kind === "file" && <DiffViewer taskId={taskId} path={selection.path} mode={diffMode(selection.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />}
    {selection?.kind === "commit" && <CommitDetail taskId={taskId} item={selection.item} />}
    {!selection && <div className="diff-empty">Select a file or commit.</div>}
  </div>
</div>
```

- [ ] **Step 4: Update the CSS so width is driven by the hook**

In `ui/src/styles.css`, replace the `.git-full-left` rule:

```css
.git-full-left { width: 360px; min-width: 300px; border-right: 1px solid var(--line); display: flex; flex-direction: column; overflow: auto; }
```

with (width now comes from the inline style; keep a floor, prevent fl- shrinking):

```css
.git-full-left { min-width: 300px; flex-shrink: 0; border-right: 1px solid var(--line); display: flex; flex-direction: column; overflow: auto; }
```

- [ ] **Step 5: Verify and manually test**

Run: `cd ui && npx tsc --noEmit` (expected: no errors).
Then run the app, open Source Control with an agent focused, and drag the divider between the list and the diff. Expected: the split resizes and the width persists across an app restart (stored under `pane:git-full-left`).

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/git/GitPanel.tsx ui/src/styles.css
git commit -m "feat(ui): make the full Source Control split resizable"
```

---

### Task 5: Files tab + `FilesView` container

Add the `"files"` tab to the store and `AgentsView`, and a `FilesView` that resolves the active `FileRoot`, shows a header + empty states, and hosts the tree/editor split. The tree and editor are stubbed here and filled in by Tasks 6–7.

**Files:**
- Modify: `ui/src/store/runs.tsx` (extend `Tab`)
- Modify: `ui/src/components/AgentsView.tsx` (tab button + routing)
- Create: `ui/src/components/FilesView.tsx`
- Modify: `ui/src/styles.css` (files-view layout)

**Interfaces:**
- Consumes: `FileRoot` (Task 2), run store `focusedRunId`, `AgentsView`'s `project: Project`.
- Produces: `FilesView({ root, projectName })` where `root: FileRoot | null`, `projectName: string`. Tasks 6–7 add `FileTree` and `FileEditor` children.

- [ ] **Step 1: Extend the `Tab` type**

In `ui/src/store/runs.tsx`, change:

```typescript
type Tab = "agents" | "source";
```

to:

```typescript
type Tab = "agents" | "source" | "files";
```

- [ ] **Step 2: Create `FilesView` (with placeholder children)**

Create `ui/src/components/FilesView.tsx`:

```tsx
import { FileRoot } from "../api";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";

export default function FilesView({ root, projectName }: { root: FileRoot | null; projectName: string }) {
  const treePane = usePaneWidth("files-tree", 280, 180, 560);
  const [selected, setSelected] = useState<string | null>(null);

  if (!root) {
    return <div className="board empty">Open a project to browse its files.</div>;
  }

  const rootLabel = root.kind === "run" ? "Agent worktree" : `${projectName} · main`;

  return (
    <div className="files-view">
      <div className="files-tree" style={{ width: treePane.width }}>
        <div className="files-root-label">{rootLabel}</div>
        {/* FileTree added in Task 6 */}
        <div className="files-tree-body">tree…</div>
      </div>
      <Resizer size={treePane.width} min={180} max={560} onChange={treePane.setWidth} />
      <div className="files-editor">
        {/* FileEditor added in Task 7 */}
        {selected ? <div>editor: {selected}</div> : <div className="diff-empty">Select a file to view.</div>}
      </div>
    </div>
  );
}
```

Add the missing import at the top:

```tsx
import { useState } from "react";
```

- [ ] **Step 3: Add the tab button and routing in `AgentsView`**

In `ui/src/components/AgentsView.tsx`:

(a) Add the import near the other component imports:

```tsx
import FilesView from "./FilesView";
import { FileRoot } from "../api";
```

(b) Add the Files button after the Source Control button inside the first `.seg`:

```tsx
<button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
<button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>▤ Files</button>
```

(c) After the existing `{tab === "source" && ( ... )}` block, add:

```tsx
{tab === "files" && (
  <div className="source-wrap">
    <FilesView
      root={
        focusedRunId
          ? ({ kind: "run", id: focusedRunId } as FileRoot)
          : ({ kind: "project", id: project.id } as FileRoot)
      }
      projectName={project.name}
    />
  </div>
)}
```

(Note: `root` is never null here because a project is always selected when `AgentsView` renders; `FilesView` still handles null defensively.)

- [ ] **Step 4: Add layout CSS**

Append to `ui/src/styles.css`:

```css
.files-view { display: flex; flex: 1; min-height: 0; }
.files-tree { min-width: 180px; flex-shrink: 0; border-right: 1px solid var(--line); display: flex; flex-direction: column; overflow: auto; }
.files-root-label { padding: 6px 10px; font-size: 11px; opacity: 0.7; border-bottom: 1px solid var(--line); }
.files-tree-body { flex: 1; overflow: auto; padding: 4px 0; }
.files-editor { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
```

- [ ] **Step 5: Verify and manually test**

Run: `cd ui && npx tsc --noEmit` (expected: no errors).
Run the app: a **Files** tab appears next to Source Control. Clicking it shows the root label and the tree/editor split, and the divider drags (persisted under `pane:files-tree`).

- [ ] **Step 6: Commit**

```bash
git add ui/src/store/runs.tsx ui/src/components/AgentsView.tsx ui/src/components/FilesView.tsx ui/src/styles.css
git commit -m "feat(ui): add Files tab and FilesView shell"
```

---

### Task 6: Lazy `FileTree`

Recursive, expand-on-demand tree fed by `listDir`. Includes a tested pure path helper.

**Files:**
- Create: `ui/src/lib/filePath.ts`
- Create: `ui/src/components/FileTree.tsx`
- Modify: `ui/src/components/FilesView.tsx` (mount `FileTree`)
- Modify: `ui/src/styles.css` (tree node styles)
- Test: `ui/src/lib/filePath.test.ts`

**Interfaces:**
- Consumes: `listDir`, `DirEntry`, `FileRoot` (Task 2).
- Produces:
  - `joinPath(parent: string, name: string): string`
  - `FileTree({ root, selected, onSelect })` where `onSelect(path: string)` fires for files (not dirs).

- [ ] **Step 1: Write the failing test for the path helper**

Create `ui/src/lib/filePath.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { joinPath } from "./filePath";

describe("joinPath", () => {
  it("joins under the root with no leading slash", () => {
    expect(joinPath("", "src")).toBe("src");
  });
  it("joins nested segments with a single slash", () => {
    expect(joinPath("src", "main.rs")).toBe("src/main.rs");
    expect(joinPath("a/b", "c")).toBe("a/b/c");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npx vitest run src/lib/filePath.test.ts`
Expected: FAIL — cannot find module `./filePath`.

- [ ] **Step 3: Implement the helper**

Create `ui/src/lib/filePath.ts`:

```typescript
/** Join a child name onto a worktree-relative parent path. The root is "". */
export function joinPath(parent: string, name: string): string {
  return parent ? `${parent}/${name}` : name;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ui && npx vitest run src/lib/filePath.test.ts`
Expected: PASS (2 assertions).

- [ ] **Step 5: Implement `FileTree`**

Create `ui/src/components/FileTree.tsx`:

```tsx
import { useEffect, useState } from "react";
import { DirEntry, FileRoot, listDir } from "../api";
import { joinPath } from "../lib/filePath";

function TreeNode({
  root, path, name, isDir, depth, selected, onSelect,
}: {
  root: FileRoot;
  path: string;
  name: string;
  isDir: boolean;
  depth: number;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<DirEntry[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isDir || !open || children) return;
    listDir(root, path).then(setChildren).catch((e) => setError(String(e)));
  }, [isDir, open, children, root, path]);

  const pad = { paddingLeft: 8 + depth * 12 };

  if (!isDir) {
    return (
      <div
        className={`tree-row file ${selected === path ? "on" : ""}`}
        style={pad}
        onClick={() => onSelect(path)}
      >
        <span className="tree-icon">·</span> {name}
      </div>
    );
  }

  return (
    <>
      <div className="tree-row dir" style={pad} onClick={() => setOpen((o) => !o)}>
        <span className="tree-icon">{open ? "▾" : "▸"}</span> {name}
      </div>
      {open && error && <div className="tree-row error" style={pad}>{error}</div>}
      {open && children?.map((c) => (
        <TreeNode
          key={c.name}
          root={root}
          path={joinPath(path, c.name)}
          name={c.name}
          isDir={c.isDir}
          depth={depth + 1}
          selected={selected}
          onSelect={onSelect}
        />
      ))}
    </>
  );
}

export default function FileTree({
  root, selected, onSelect,
}: {
  root: FileRoot;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [error, setError] = useState("");

  // Re-root whenever the FileRoot changes (focused agent ↔ project main).
  const rootKey = `${root.kind}:${root.id}`;
  useEffect(() => {
    setEntries(null);
    setError("");
    listDir(root, "").then(setEntries).catch((e) => setError(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  if (error) return <div className="tree-row error">{error}</div>;
  if (!entries) return <div className="tree-row">loading…</div>;

  return (
    <>
      {entries.map((c) => (
        <TreeNode
          key={c.name}
          root={root}
          path={c.name}
          name={c.name}
          isDir={c.isDir}
          depth={0}
          selected={selected}
          onSelect={onSelect}
        />
      ))}
    </>
  );
}
```

- [ ] **Step 6: Mount `FileTree` in `FilesView`**

In `ui/src/components/FilesView.tsx`, add the import:

```tsx
import FileTree from "./FileTree";
```

Replace the placeholder tree body:

```tsx
<div className="files-tree-body">tree…</div>
```

with:

```tsx
<div className="files-tree-body">
  <FileTree root={root} selected={selected} onSelect={setSelected} />
</div>
```

- [ ] **Step 7: Add tree node CSS**

Append to `ui/src/styles.css`:

```css
.tree-row { display: flex; align-items: center; gap: 4px; padding: 2px 8px; font-size: 12px; white-space: nowrap; cursor: pointer; user-select: none; }
.tree-row:hover { background: var(--hover, rgba(255,255,255,0.05)); }
.tree-row.on { background: var(--sel, rgba(255,255,255,0.12)); }
.tree-row.error { color: var(--err, #e06c75); cursor: default; }
.tree-icon { display: inline-block; width: 1em; opacity: 0.7; }
```

- [ ] **Step 8: Verify and manually test**

Run: `cd ui && npx vitest run src/lib/filePath.test.ts && npx tsc --noEmit` (expected: pass, no type errors).
Run the app, open Files: the worktree's top level lists (dirs first). Expanding a folder lazily loads its children; expanding `node_modules` does not freeze the app (only one level loads). Clicking a file highlights it.

- [ ] **Step 9: Commit**

```bash
git add ui/src/lib/filePath.ts ui/src/lib/filePath.test.ts ui/src/components/FileTree.tsx ui/src/components/FilesView.tsx ui/src/styles.css
git commit -m "feat(ui): lazy-expanding file tree"
```

---

### Task 7: CodeMirror file editor with save

Editable, syntax-highlighted editor with dirty tracking and Cmd+S save; binary/oversized files show a placeholder.

**Files:**
- Modify: `ui/package.json` (add CodeMirror deps)
- Create: `ui/src/components/FileEditor.tsx`
- Create: `ui/src/lib/cmLanguage.ts`
- Modify: `ui/src/components/FilesView.tsx` (mount `FileEditor`)
- Modify: `ui/src/styles.css` (editor chrome)

**Interfaces:**
- Consumes: `readFile`, `writeFile`, `FileContents`, `FileRoot` (Task 2).
- Produces: `FileEditor({ root, path })`; `languageExtension(path): Extension[]`.

- [ ] **Step 1: Install CodeMirror**

Run:

```bash
cd ui && npm install codemirror @codemirror/state @codemirror/view @codemirror/commands \
  @codemirror/theme-one-dark @codemirror/lang-javascript @codemirror/lang-json \
  @codemirror/lang-rust @codemirror/lang-css @codemirror/lang-html \
  @codemirror/lang-markdown @codemirror/lang-python
```

Expected: packages added to `ui/package.json` dependencies; `npm install` exits 0.

- [ ] **Step 2: Implement the language mapper**

Create `ui/src/lib/cmLanguage.ts`:

```typescript
import type { Extension } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { rust } from "@codemirror/lang-rust";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";

/** Pick a CodeMirror language extension from a file path; [] for unknown types. */
export function languageExtension(path: string): Extension[] {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  switch (ext) {
    case "ts": case "tsx": return [javascript({ typescript: true, jsx: ext === "tsx" })];
    case "js": case "jsx": return [javascript({ jsx: ext === "jsx" })];
    case "json": return [json()];
    case "rs": return [rust()];
    case "css": return [css()];
    case "html": return [html()];
    case "md": case "markdown": return [markdown()];
    case "py": return [python()];
    default: return [];
  }
}
```

- [ ] **Step 3: Implement `FileEditor`**

Create `ui/src/components/FileEditor.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { defaultKeymap } from "@codemirror/commands";
import { oneDark } from "@codemirror/theme-one-dark";
import { FileRoot, readFile, writeFile } from "../api";
import { languageExtension } from "../lib/cmLanguage";

export default function FileEditor({ root, path }: { root: FileRoot; path: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [status, setStatus] = useState<"loading" | "binary" | "tooLarge" | "ready" | "error">("loading");
  const [dirty, setDirty] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");

  // Keep a stable save handler that reads the current doc from the live view.
  const save = useRef(async () => {});
  save.current = async () => {
    const view = viewRef.current;
    if (!view) return;
    try {
      await writeFile(root, path, view.state.doc.toString());
      setDirty(false);
    } catch (e) {
      setErrorMsg(String(e));
      setStatus("error");
    }
  };

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setDirty(false);
    setErrorMsg("");
    readFile(root, path).then((fc) => {
      if (cancelled) return;
      if (fc.tooLarge) { setStatus("tooLarge"); return; }
      if (fc.binary) { setStatus("binary"); return; }
      setStatus("ready");
      const host = hostRef.current;
      if (!host) return;
      viewRef.current?.destroy();
      const state = EditorState.create({
        doc: fc.text,
        extensions: [
          basicSetup,
          oneDark,
          ...languageExtension(path),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => { void save.current(); return true; } },
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => { if (u.docChanged) setDirty(true); }),
        ],
      });
      viewRef.current = new EditorView({ state, parent: host });
    }).catch((e) => {
      if (cancelled) return;
      setErrorMsg(String(e));
      setStatus("error");
    });

    return () => {
      cancelled = true;
      viewRef.current?.destroy();
      viewRef.current = null;
    };
  }, [root.kind, root.id, path]);

  return (
    <div className="file-editor-wrap">
      <div className="file-editor-bar">
        <span className="file-editor-path">{path}{dirty ? " ●" : ""}</span>
        <span className="spacer" style={{ flex: 1 }} />
        <button className="git-iconbtn" disabled={!dirty || status !== "ready"} onClick={() => void save.current()}>
          Save
        </button>
      </div>
      {status === "loading" && <div className="diff-empty">loading…</div>}
      {status === "binary" && <div className="diff-empty">Binary file — not shown.</div>}
      {status === "tooLarge" && <div className="diff-empty">File too large to open.</div>}
      {status === "error" && <div className="git-error">{errorMsg}</div>}
      <div ref={hostRef} className="file-editor-host" style={{ display: status === "ready" ? "block" : "none" }} />
    </div>
  );
}
```

- [ ] **Step 4: Mount `FileEditor` in `FilesView`**

In `ui/src/components/FilesView.tsx`, add the import:

```tsx
import FileEditor from "./FileEditor";
```

Replace the placeholder editor block:

```tsx
{selected ? <div>editor: {selected}</div> : <div className="diff-empty">Select a file to view.</div>}
```

with:

```tsx
{selected ? <FileEditor root={root} path={selected} /> : <div className="diff-empty">Select a file to view.</div>}
```

- [ ] **Step 5: Add editor chrome CSS**

Append to `ui/src/styles.css`:

```css
.file-editor-wrap { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.file-editor-bar { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border-bottom: 1px solid var(--line); font-size: 12px; }
.file-editor-path { opacity: 0.85; }
.file-editor-host { flex: 1; min-height: 0; overflow: auto; }
.file-editor-host .cm-editor { height: 100%; }
```

- [ ] **Step 6: Verify and manually test**

Run: `cd ui && npx tsc --noEmit` (expected: no type errors).
Run the app, open Files, click a text file: it opens highlighted and editable. Type → the `●` dirty mark and the Save button enable. Press Cmd+S (or click Save) → the dirty mark clears; reopening the file shows the saved content on disk. Click a binary file (e.g. an image) → "Binary file — not shown." Switching the focused agent re-roots the tree and the editor follows.

- [ ] **Step 7: Commit**

```bash
git add ui/package.json ui/package-lock.json ui/src/lib/cmLanguage.ts ui/src/components/FileEditor.tsx ui/src/components/FilesView.tsx ui/src/styles.css
git commit -m "feat(ui): editable CodeMirror file viewer with save"
```

---

## Final verification

- [ ] Backend tests: `cargo test -p agency-core` and `cargo build -p agency-app` pass.
- [ ] Frontend: `cd ui && npx tsc --noEmit && npx vitest run` pass.
- [ ] Manual smoke (single session of the app):
  - Opening an agent shows no extra blank line (Task 3).
  - The full Source Control split drags and persists (Task 4).
  - Files tab: tree lists & lazy-expands; project-main fallback works when no agent is focused; editing + Cmd+S saves; binary/oversized placeholders show (Tasks 5–7).
- [ ] Rebase onto latest `main` (it advanced during this work) before merging.

## Self-review notes

- **Spec coverage:** newline bug → Task 3; resizable source-control panes → Task 4; Files mode placement/tab → Task 5; unfiltered lazy tree → Task 6; editable CodeMirror + save + binary/too-large placeholder → Task 7; worktree-vs-project-main root selector + path-traversal guard → Tasks 1–2. All spec acceptance items map to a task.
- **Deferred-by-design:** exact binary detection (NUL byte) and size threshold (2 MB) are now concrete in Task 1; external-change-while-open reload beyond reopening the file remains out of scope per the spec.
- **Type consistency:** `FileRoot`, `DirEntry` (`isDir`), `FileContents` (`binary`/`tooLarge`) names match across Rust serde output (camelCase) and the TS interfaces; `joinPath` and `languageExtension` signatures match their call sites.
