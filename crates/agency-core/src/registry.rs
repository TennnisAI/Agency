use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::profile::AgentProfile;

/// Theme accent names the UI resolves to CSS vars (`var(--<name>)`). Order is
/// the assignment preference for new projects. Public so the color a user picks
/// can be validated against it: the name is interpolated into a CSS var, so
/// only these may ever reach the database.
pub const PROJECT_COLORS: [&str; 9] =
    ["blue", "mauve", "green", "peach", "teal", "pink", "yellow", "lav", "red"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub default_agent: Option<String>,
    pub default_provider: Option<String>,
    /// Theme accent name (e.g. "blue") the UI maps to a CSS var. Assigned at
    /// add time: the least-used palette color, so projects stay distinct.
    pub color: Option<String>,
    /// 3-letter issue key ("AGE") issues are numbered under (AGE-14). Stored,
    /// not derived, so renaming the project never re-keys its issues.
    pub issue_key: Option<String>,
    /// `None` = a normal repo project; `"workspace"` = the single pinned
    /// workspace (journaling/notes home) with relaxed git requirements.
    pub kind: Option<String>,
}

/// The `Project.kind` value marking the pinned workspace.
pub const PROJECT_KIND_WORKSPACE: &str = "workspace";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub base: String,
    pub branch: String,
    pub created_at: i64,
    pub port_base: Option<u16>,
    pub archived_at: Option<i64>,
    pub title: Option<String>,
    pub kind: String,
    /// Branch this run's work merges into. `None` = auto-detect (main/master) at merge time.
    pub merge_target: Option<String>,
    /// Groups runs spawned together on the same prompt (multi-attempt racing).
    pub race_id: Option<String>,
    /// Present = this run is a loop: the agent is re-invoked headless until
    /// the check command passes or the attempt cap is spent. Written once at
    /// creation, never mutated.
    pub loop_config: Option<crate::loops::LoopConfig>,
    /// Loop progress, persisted on every transition so an app restart resumes
    /// the loop. Always None for non-loop runs.
    pub loop_state: Option<crate::loops::LoopState>,
    /// The local issue this run was dispatched from. Many runs may share one
    /// issue (races, retries), so the link lives on the run.
    pub issue_id: Option<String>,
    /// Whether this run owns an isolated worktree at
    /// `<repo>/.agency/worktrees/<id>` on its own `agent/<id>` branch.
    ///
    /// `false` means the run works directly in the project's main checkout, on
    /// whatever branch is checked out there — the "just let me work on main"
    /// flow. Such a run has nothing of its own to tear down or merge, so
    /// discard/archive never touch git and merge/PR are refused. Terminals have
    /// always behaved this way and carry `false` too.
    pub worktree: bool,
}

/// A local issue: the tracker is per-project and agent-native — dispatching
/// an issue spawns a run, merging the run's branch closes the issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub project_id: String,
    /// Per-project number; displayed as `<project issue_key>-<seq>` (AGE-14).
    /// Never reused — deleting an issue leaves a gap.
    pub seq: i64,
    pub title: String,
    pub body: String,
    pub status: IssueStatus,
    /// 0 none · 1 low · 2 medium · 3 high · 4 urgent.
    pub priority: u8,
    /// Civil dates, `YYYY-MM-DD` strings — lexicographic order is date order.
    pub due: Option<String>,
    pub scheduled: Option<String>,
    /// Manual board order within a status group, ascending. Finite.
    pub rank: Option<f64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Done,
    Cancelled,
}

impl IssueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            IssueStatus::Backlog => "backlog",
            IssueStatus::Todo => "todo",
            IssueStatus::InProgress => "in_progress",
            IssueStatus::InReview => "in_review",
            IssueStatus::Done => "done",
            IssueStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Result<IssueStatus> {
        Ok(match s {
            "backlog" => IssueStatus::Backlog,
            "todo" => IssueStatus::Todo,
            "in_progress" => IssueStatus::InProgress,
            "in_review" => IssueStatus::InReview,
            "done" => IssueStatus::Done,
            "cancelled" => IssueStatus::Cancelled,
            other => anyhow::bail!("unknown issue status: {other}"),
        })
    }

    /// Position in the forward workflow. Cancelled is terminal and outside
    /// the progression: automation neither advances from nor to it.
    fn rank(self) -> Option<u8> {
        match self {
            IssueStatus::Backlog => Some(0),
            IssueStatus::Todo => Some(1),
            IssueStatus::InProgress => Some(2),
            IssueStatus::InReview => Some(3),
            IssueStatus::Done => Some(4),
            IssueStatus::Cancelled => None,
        }
    }

    /// Whether automation may move `self` → `to`: strictly forward, never
    /// into or out of cancelled. The one guard behind every automatic status
    /// move (merge → done, PR → in_review, dispatch → in_progress).
    pub fn advances_to(self, to: IssueStatus) -> bool {
        matches!((self.rank(), to.rank()), (Some(from), Some(to)) if from < to)
    }
}

/// Partial update for `update_issue` — `None` fields are left untouched.
/// The nullable fields (`due`/`scheduled`/`rank`) are double-`Option`s so a
/// patch can distinguish "leave it" (absent) from "clear it" (explicit null).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<IssueStatus>,
    pub priority: Option<u8>,
    #[serde(default, deserialize_with = "double_option")]
    pub due: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub scheduled: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub rank: Option<Option<f64>>,
}

/// Deserialize a present-but-maybe-null field as `Some(inner)`; combined with
/// `#[serde(default)]`, an absent field stays `None`.
fn double_option<'de, T, D>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// An extra agent session inside an existing run's worktree. The run's
/// primary session is implicit (daemon session named after the run id);
/// rows here are the additional tabs, keyed `<run_id>--<n>` so the whole
/// terminal plumbing can address them like ordinary run sessions. Stored so
/// tabs survive an app restart and dead sessions relaunch with the right
/// agent profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSession {
    pub id: String,
    pub run_id: String,
    pub agent: String,
    pub created_at: i64,
}

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

pub struct Registry {
    conn: Connection,
}

impl Registry {
    pub fn open(db_path: &Path) -> Result<Registry> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening db at {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                default_agent TEXT,
                default_provider TEXT
            );
            CREATE TABLE IF NOT EXISTS profiles (
                name TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                args TEXT NOT NULL,
                env TEXT NOT NULL,
                resume_args TEXT
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                prompt TEXT NOT NULL,
                base TEXT NOT NULL,
                branch TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                port_base INTEGER,
                archived_at INTEGER,
                title TEXT,
                kind TEXT NOT NULL DEFAULT 'agent'
            );
            CREATE TABLE IF NOT EXISTS run_sessions (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
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
            CREATE TABLE IF NOT EXISTS issues (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'todo',
                priority INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE (project_id, seq)
            );
            CREATE TABLE IF NOT EXISTS issue_seqs (
                project_id TEXT PRIMARY KEY,
                next INTEGER NOT NULL
            );",
        )?;
        // Migrate older DBs whose `runs` table predates `port_base`.
        if !column_exists(&conn, "runs", "port_base")? {
            conn.execute("ALTER TABLE runs ADD COLUMN port_base INTEGER", [])?;
        }
        if !column_exists(&conn, "runs", "archived_at")? {
            conn.execute("ALTER TABLE runs ADD COLUMN archived_at INTEGER", [])?;
        }
        if !column_exists(&conn, "runs", "title")? {
            conn.execute("ALTER TABLE runs ADD COLUMN title TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "kind")? {
            conn.execute("ALTER TABLE runs ADD COLUMN kind TEXT NOT NULL DEFAULT 'agent'", [])?;
        }
        if !column_exists(&conn, "runs", "merge_target")? {
            conn.execute("ALTER TABLE runs ADD COLUMN merge_target TEXT", [])?;
        }
        if !column_exists(&conn, "projects", "color")? {
            conn.execute("ALTER TABLE projects ADD COLUMN color TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "race_id")? {
            conn.execute("ALTER TABLE runs ADD COLUMN race_id TEXT", [])?;
        }
        if !column_exists(&conn, "profiles", "resume_args")? {
            conn.execute("ALTER TABLE profiles ADD COLUMN resume_args TEXT", [])?;
        }
        if !column_exists(&conn, "profiles", "loop_args")? {
            conn.execute("ALTER TABLE profiles ADD COLUMN loop_args TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "loop_config")? {
            conn.execute("ALTER TABLE runs ADD COLUMN loop_config TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "loop_state")? {
            conn.execute("ALTER TABLE runs ADD COLUMN loop_state TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "issue_id")? {
            conn.execute("ALTER TABLE runs ADD COLUMN issue_id TEXT", [])?;
        }
        if !column_exists(&conn, "runs", "worktree")? {
            // Every pre-existing agent run has a worktree; terminals never did.
            conn.execute("ALTER TABLE runs ADD COLUMN worktree INTEGER NOT NULL DEFAULT 1", [])?;
            conn.execute("UPDATE runs SET worktree = 0 WHERE kind = 'terminal'", [])?;
        }
        if !column_exists(&conn, "projects", "issue_key")? {
            conn.execute("ALTER TABLE projects ADD COLUMN issue_key TEXT", [])?;
        }
        if !column_exists(&conn, "projects", "closed")? {
            conn.execute(
                "ALTER TABLE projects ADD COLUMN closed INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !column_exists(&conn, "projects", "kind")? {
            conn.execute("ALTER TABLE projects ADD COLUMN kind TEXT", [])?;
        }
        if !column_exists(&conn, "projects", "issues_migrated")? {
            conn.execute(
                "ALTER TABLE projects ADD COLUMN issues_migrated INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !column_exists(&conn, "issues", "due")? {
            conn.execute("ALTER TABLE issues ADD COLUMN due TEXT", [])?;
        }
        if !column_exists(&conn, "issues", "scheduled")? {
            conn.execute("ALTER TABLE issues ADD COLUMN scheduled TEXT", [])?;
        }
        if !column_exists(&conn, "issues", "rank")? {
            conn.execute("ALTER TABLE issues ADD COLUMN rank REAL", [])?;
        }
        let reg = Registry { conn };
        reg.backfill_project_colors()?;
        reg.backfill_issue_keys()?;
        Ok(reg)
    }

    /// Give color-less projects (rows predating the color column) distinct
    /// palette colors, in name order, using the same least-used rule as
    /// `add_project`. Idempotent: no-op once every project has a color.
    fn backfill_project_colors(&self) -> Result<()> {
        let ids: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id FROM projects WHERE color IS NULL ORDER BY name")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for id in ids {
            let color = self.pick_project_color()?;
            self.conn.execute(
                "UPDATE projects SET color = ?2 WHERE id = ?1",
                rusqlite::params![id, color],
            )?;
        }
        Ok(())
    }

    /// Give key-less projects (rows predating the issue_key column) stable
    /// issue keys, in name order, with the same collision rule as
    /// `add_project`. Idempotent: no-op once every project has a key.
    fn backfill_issue_keys(&self) -> Result<()> {
        let rows: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                // The workspace is included: Phase 5 brought it into the
                // tracker, so a key-less workspace row gets one here.
                .prepare(
                    "SELECT id, name FROM projects
                     WHERE issue_key IS NULL
                     ORDER BY name",
                )?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for (id, name) in rows {
            let key = derive_issue_key(&name, &self.used_issue_keys()?);
            self.conn.execute(
                "UPDATE projects SET issue_key = ?2 WHERE id = ?1",
                rusqlite::params![id, key],
            )?;
        }
        Ok(())
    }

    fn used_issue_keys(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT issue_key FROM projects WHERE issue_key IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        // Re-adding a closed project's path revives it instead of inserting a
        // duplicate — its runs and worktrees were kept on close, so the project
        // comes back exactly as it was left.
        if let Some(p) = self.find_closed_project(repo_path)? {
            self.set_project_closed(&p.id, false)?;
            return Ok(p);
        }
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            repo_path: repo_path.to_path_buf(),
            default_agent: None,
            default_provider: None,
            color: Some(self.pick_project_color()?),
            issue_key: Some(derive_issue_key(name, &self.used_issue_keys()?)),
            kind: None,
        };
        self.insert_project(&project)?;
        Ok(project)
    }

    fn insert_project(&self, project: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, name, repo_path, default_agent, default_provider, color, issue_key, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                project.id,
                project.name,
                project.repo_path.to_string_lossy(),
                project.default_agent,
                project.default_provider,
                project.color,
                project.issue_key,
                project.kind,
            ],
        )?;
        Ok(())
    }

    /// The single pinned workspace project, if it has been created — closed or
    /// not (there is exactly one; hiding it is not a flow).
    pub fn get_workspace(&self) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider, color, issue_key, kind
             FROM projects WHERE kind = 'workspace'",
        )?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_project(row)?)),
            None => Ok(None),
        }
    }

    /// Create the pinned workspace project, or return the existing one
    /// (un-closing it if needed). The workspace is in the tracker like any
    /// project: it gets an issue key so personal/planning tasks have one too.
    pub fn ensure_workspace(&self, name: &str, repo_path: &Path) -> Result<Project> {
        if let Some(mut ws) = self.get_workspace()? {
            self.set_project_closed(&ws.id, false)?;
            // A workspace re-created at a new location adopts it.
            if ws.repo_path != repo_path {
                self.set_project_repo_path(&ws.id, repo_path)?;
                ws.repo_path = repo_path.to_path_buf();
            }
            return Ok(ws);
        }
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            repo_path: repo_path.to_path_buf(),
            default_agent: None,
            default_provider: None,
            color: Some(self.pick_project_color()?),
            issue_key: Some(derive_issue_key(name, &self.used_issue_keys()?)),
            kind: Some(PROJECT_KIND_WORKSPACE.to_string()),
        };
        self.insert_project(&project)?;
        Ok(project)
    }

    /// Point a project at a new folder (workspace move).
    pub fn set_project_repo_path(&self, id: &str, repo_path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET repo_path = ?2 WHERE id = ?1",
            rusqlite::params![id, repo_path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// The least-used palette color (palette order breaks ties), so every new
    /// project gets a color no other project has until the palette runs out.
    fn pick_project_color(&self) -> Result<String> {
        let mut stmt = self.conn.prepare("SELECT color FROM projects WHERE color IS NOT NULL")?;
        let used: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        let pick = PROJECT_COLORS
            .iter()
            .min_by_key(|c| used.iter().filter(|u| u == *c).count())
            .unwrap_or(&PROJECT_COLORS[0]);
        Ok(pick.to_string())
    }

    /// Recolor a project by hand, overriding the color picked at add time.
    /// Only palette names are accepted — the value ends up inside a CSS
    /// `var(--<name>)` in the UI, and an unknown name would silently render
    /// nothing rather than a color.
    pub fn set_project_color(&self, id: &str, color: &str) -> Result<()> {
        if !PROJECT_COLORS.contains(&color) {
            anyhow::bail!("unknown project color: {color}");
        }
        self.conn.execute(
            "UPDATE projects SET color = ?2 WHERE id = ?1",
            rusqlite::params![id, color],
        )?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider, color, issue_key, kind
             FROM projects WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_project(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            // NOCASE so the sidebar reads alphabetically the way a person does:
            // plain `ORDER BY name` is byte order, which files every uppercase
            // name ahead of every lowercase one.
            "SELECT id, name, repo_path, default_agent, default_provider, color, issue_key, kind
             FROM projects WHERE closed = 0 ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_project(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Hide (or un-hide) a project without touching its runs or worktrees.
    /// Closed projects drop out of `list_projects` but keep every record, so
    /// `add_project` on the same path can revive them.
    pub fn set_project_closed(&self, id: &str, closed: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET closed = ?2 WHERE id = ?1",
            rusqlite::params![id, closed as i64],
        )?;
        Ok(())
    }

    fn find_closed_project(&self, repo_path: &Path) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider, color, issue_key, kind
             FROM projects WHERE repo_path = ?1 AND closed = 1",
        )?;
        let mut rows = stmt.query([repo_path.to_string_lossy()])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_project(row)?)),
            None => Ok(None),
        }
    }

    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn upsert_profile(&self, p: &AgentProfile) -> Result<()> {
        let args = serde_json::to_string(&p.args)?;
        let env = serde_json::to_string(&p.env)?;
        let resume = match &p.resume_args {
            Some(r) => Some(serde_json::to_string(r)?),
            None => None,
        };
        let loop_args = match &p.loop_args {
            Some(l) => Some(serde_json::to_string(l)?),
            None => None,
        };
        self.conn.execute(
            "INSERT INTO profiles (name, command, args, env, resume_args, loop_args) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET command = ?2, args = ?3, env = ?4, resume_args = ?5, loop_args = ?6",
            rusqlite::params![p.name, p.command, args, env, resume, loop_args],
        )?;
        Ok(())
    }

    /// Set a profile's resume recipe ONLY if it is currently NULL (so a user's
    /// customization is never clobbered). No-op if the profile does not exist.
    pub fn ensure_profile_resume_args(&self, name: &str, resume_args: &Option<Vec<String>>) -> Result<()> {
        let resume = match resume_args {
            Some(r) => Some(serde_json::to_string(r)?),
            None => None,
        };
        self.conn.execute(
            "UPDATE profiles SET resume_args = ?2 WHERE name = ?1 AND resume_args IS NULL",
            rusqlite::params![name, resume],
        )?;
        Ok(())
    }

    /// Set a profile's loop (headless one-shot) recipe ONLY if it is currently
    /// NULL — same never-clobber rule as `ensure_profile_resume_args`.
    pub fn ensure_profile_loop_args(&self, name: &str, loop_args: &Option<Vec<String>>) -> Result<()> {
        let l = match loop_args {
            Some(r) => Some(serde_json::to_string(r)?),
            None => None,
        };
        self.conn.execute(
            "UPDATE profiles SET loop_args = ?2 WHERE name = ?1 AND loop_args IS NULL",
            rusqlite::params![name, l],
        )?;
        Ok(())
    }

    pub fn get_profile(&self, name: &str) -> Result<Option<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env, resume_args, loop_args FROM profiles WHERE name = ?1")?;
        let mut rows = stmt.query([name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_profile(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env, resume_args, loop_args FROM profiles ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok(row_to_profile(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE name = ?1", [name])?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn insert_run(&self, run: &Run) -> Result<()> {
        let loop_config = match &run.loop_config {
            Some(c) => Some(serde_json::to_string(c)?),
            None => None,
        };
        let loop_state = match &run.loop_state {
            Some(s) => Some(serde_json::to_string(s)?),
            None => None,
        };
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id, loop_config, loop_state, issue_id, worktree)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at, run.title, run.kind,
                run.merge_target, run.race_id, loop_config, loop_state, run.issue_id, run.worktree as i64
            ],
        )?;
        Ok(())
    }

    /// Persist a loop's progress. Called on every loop transition so a
    /// restarted app resumes instead of orphaning the loop.
    pub fn set_loop_state(&self, id: &str, state: &crate::loops::LoopState) -> Result<()> {
        let json = serde_json::to_string(state)?;
        self.conn.execute(
            "UPDATE runs SET loop_state = ?2 WHERE id = ?1",
            rusqlite::params![id, json],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id, loop_config, loop_state, issue_id, worktree FROM runs WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_run(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id, loop_config, loop_state, issue_id, worktree
             FROM runs WHERE project_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id, loop_config, loop_state, issue_id, worktree
             FROM runs WHERE project_id = ?1 AND archived_at IS NOT NULL ORDER BY archived_at DESC",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn set_run_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET title = ?2 WHERE id = ?1",
            rusqlite::params![id, title],
        )?;
        Ok(())
    }

    /// Record the run's prompt after the fact. Runs are created promptless
    /// (the user types straight into the agent terminal), so the first line
    /// they type is captured and stored here as the run's prompt. First
    /// capture wins: an already-set prompt is never overwritten.
    pub fn set_run_prompt_if_empty(&self, id: &str, prompt: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET prompt = ?2 WHERE id = ?1 AND prompt = ''",
            rusqlite::params![id, prompt],
        )?;
        Ok(())
    }

    /// Remember the agent type last used in this project so new-task shortcuts
    /// can default to it instead of a hardcoded agent.
    pub fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET default_agent = ?2 WHERE id = ?1",
            rusqlite::params![id, agent],
        )?;
        Ok(())
    }

    pub fn set_archived(&self, id: &str, archived_at: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET archived_at = ?2 WHERE id = ?1",
            rusqlite::params![id, archived_at],
        )?;
        Ok(())
    }

    pub fn delete_run(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM runs WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_port_base(&self, id: &str, port_base: Option<u16>) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET port_base = ?2 WHERE id = ?1",
            rusqlite::params![id, port_base.map(|p| p as i64)],
        )?;
        Ok(())
    }

    pub fn list_port_bases(&self) -> Result<Vec<u16>> {
        let mut stmt = self
            .conn
            .prepare("SELECT port_base FROM runs WHERE port_base IS NOT NULL AND archived_at IS NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r? as u16);
        }
        Ok(out)
    }

    pub fn insert_run_session(&self, s: &RunSession) -> Result<()> {
        self.conn.execute(
            "INSERT INTO run_sessions (id, run_id, agent, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![s.id, s.run_id, s.agent, s.created_at],
        )?;
        Ok(())
    }

    pub fn get_run_session(&self, id: &str) -> Result<Option<RunSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, agent, created_at FROM run_sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_run_session(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_run_sessions(&self, run_id: &str) -> Result<Vec<RunSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, agent, created_at FROM run_sessions
             WHERE run_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([run_id], |row| Ok(row_to_run_session(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_run_session(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM run_sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Remove every extra session of a run — the cascade for discard/archive.
    pub fn delete_run_sessions(&self, run_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM run_sessions WHERE run_id = ?1", [run_id])?;
        Ok(())
    }

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

    // ── issues ─────────────────────────────────────────────────────────────
    //
    // Since Phase 5 the rows here are an index over `.agency/issues/*.md` —
    // the files are canonical, `issuefs::reconcile` makes rows follow them.
    // Only `issue_seqs` (the never-reuse high-water mark) is owned by the DB.

    /// Hand out the next issue number for a project. Numbers come from a
    /// per-project counter (not MAX(seq)+1) so a deleted issue's number is
    /// never reused. `next` holds the number to hand out *after* this one, so
    /// the row is seeded at 2.
    pub fn alloc_issue_seq(&self, project_id: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO issue_seqs (project_id, next) VALUES (?1, 2)
             ON CONFLICT(project_id) DO UPDATE SET next = next + 1",
            [project_id],
        )?;
        Ok(self.conn.query_row(
            "SELECT next - 1 FROM issue_seqs WHERE project_id = ?1",
            [project_id],
            |row| row.get(0),
        )?)
    }

    /// Raise the high-water mark so `seq` is never handed out again — an agent
    /// that filed `AGE-15.md` by hand consumed 15, whatever the counter said.
    /// Monotonic: never lowers `next`.
    pub fn ensure_issue_seq_at_least(&self, project_id: &str, seq: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO issue_seqs (project_id, next) VALUES (?1, ?2 + 1)
             ON CONFLICT(project_id) DO UPDATE SET next = MAX(next, ?2 + 1)",
            rusqlite::params![project_id, seq],
        )?;
        Ok(())
    }

    /// Write an index row from a file's parsed state, keyed `(project_id,
    /// seq)`. An existing row keeps its `id` — uuids are minted at import and
    /// stay stable for the life of the file — and `issue.id` is used only when
    /// the row is new. Returns the stored row.
    pub fn upsert_issue_row(&self, issue: &Issue) -> Result<Issue> {
        self.conn.execute(
            "INSERT INTO issues (id, project_id, seq, title, body, status, priority, created_at, updated_at, due, scheduled, rank)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(project_id, seq) DO UPDATE SET
                title = ?4, body = ?5, status = ?6, priority = ?7,
                created_at = ?8, updated_at = ?9, due = ?10, scheduled = ?11, rank = ?12",
            rusqlite::params![
                issue.id,
                issue.project_id,
                issue.seq,
                issue.title,
                issue.body,
                issue.status.as_str(),
                issue.priority as i64,
                issue.created_at,
                issue.updated_at,
                issue.due,
                issue.scheduled,
                issue.rank
            ],
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, seq, title, body, status, priority, created_at, updated_at, due, scheduled, rank
             FROM issues WHERE project_id = ?1 AND seq = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![issue.project_id, issue.seq])?;
        match rows.next()? {
            Some(row) => row_to_issue(row),
            None => anyhow::bail!("issue row vanished after upsert"),
        }
    }

    /// Whether this project's issues have been exported to `.agency/issues/`
    /// (the one-shot Phase-5 migration).
    pub fn project_issues_migrated(&self, project_id: &str) -> Result<bool> {
        let v: i64 = self.conn.query_row(
            "SELECT issues_migrated FROM projects WHERE id = ?1",
            [project_id],
            |row| row.get(0),
        )?;
        Ok(v != 0)
    }

    pub fn mark_issues_migrated(&self, project_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET issues_migrated = 1 WHERE id = ?1",
            [project_id],
        )?;
        Ok(())
    }

    pub fn create_issue(
        &self,
        project_id: &str,
        title: &str,
        body: &str,
        status: IssueStatus,
        now: i64,
    ) -> Result<Issue> {
        let id = Uuid::new_v4().to_string();
        let seq = self.alloc_issue_seq(project_id)?;
        self.conn.execute(
            "INSERT INTO issues (id, project_id, seq, title, body, status, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            rusqlite::params![id, project_id, seq, title, body, status.as_str(), now],
        )?;
        Ok(self.get_issue(&id)?.expect("issue just inserted"))
    }

    pub fn get_issue(&self, id: &str) -> Result<Option<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, seq, title, body, status, priority, created_at, updated_at, due, scheduled, rank
             FROM issues WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_issue(row)?)),
            None => Ok(None),
        }
    }

    /// Every issue of the project, all statuses — the UI groups and collapses.
    pub fn list_issues(&self, project_id: &str) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, seq, title, body, status, priority, created_at, updated_at, due, scheduled, rank
             FROM issues WHERE project_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_issue(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM issues WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Cascade for `delete_project`.
    pub fn delete_project_issues(&self, project_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM issues WHERE project_id = ?1", [project_id])?;
        self.conn.execute("DELETE FROM issue_seqs WHERE project_id = ?1", [project_id])?;
        Ok(())
    }

    /// Automation path: move the issue *forward* to `status`, but never
    /// demote a manual advance and never touch a cancelled issue. Returns
    /// whether the status actually changed.
    pub fn advance_issue_status(&self, id: &str, status: IssueStatus, now: i64) -> Result<bool> {
        let Some(current) = self.get_issue(id)? else { return Ok(false) };
        if !current.status.advances_to(status) {
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE issues SET status = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, status.as_str(), now],
        )?;
        Ok(true)
    }

    /// Automation path for abandonment: an issue whose last run was
    /// discarded/archived unmerged falls back to `todo` — but only from the
    /// two agent-driven states, so manual `done`/`cancelled` stay put.
    pub fn rollback_issue_to_todo(&self, id: &str, now: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE issues SET status = 'todo', updated_at = ?2
             WHERE id = ?1 AND status IN ('in_progress', 'in_review')",
            rusqlite::params![id, now],
        )?;
        Ok(changed > 0)
    }

    /// Non-archived runs dispatched from this issue (the issue's "linked
    /// runs" list; also drives the last-run-abandoned rollback check).
    pub fn runs_for_issue(&self, issue_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id, loop_config, loop_state, issue_id, worktree
             FROM runs WHERE issue_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([issue_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Drop every run and the rows hanging off runs, leaving projects, agent
    /// profiles, settings and issues intact. Only used when seeding a fresh DB
    /// from another one (see [`copy_without_runs`]) — a run is bound to a live
    /// daemon session, a worktree and a branch that belong to the app that
    /// created it, so a copy must not carry them.
    pub fn clear_runs(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM review_comments; DELETE FROM run_sessions; DELETE FROM runs;",
        )?;
        Ok(())
    }
}

/// Seed the database at `dst` from the one at `src`, minus every run.
///
/// This is how a `tauri dev` build starts life with the installed app's
/// projects, agent profiles and settings instead of an empty window. Runs are
/// deliberately left behind: each one owns a daemon session, a worktree and a
/// branch that the installed app is still driving, so a second app that knew
/// about them could discard or merge work out from under it — and every one of
/// them would show as dead in a build that does not host their sessions.
///
/// `VACUUM INTO` rather than a file copy: it takes a transactionally
/// consistent snapshot even while the other app has the DB open. It refuses to
/// write an existing file, so `dst` must not exist yet.
pub fn copy_without_runs(src: &Path, dst: &Path) -> Result<()> {
    {
        let conn = Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening db at {} to copy from", src.display()))?;
        conn.execute("VACUUM INTO ?1", [dst.to_string_lossy().as_ref()])
            .with_context(|| format!("copying {} to {}", src.display(), dst.display()))?;
    }
    // Through Registry::open so the copy lands fully migrated: `src` may come
    // from an older build whose schema predates columns this one reads.
    Registry::open(dst)?.clear_runs()
}

/// Snapshot `agency.db` to `agency.db.bak` once per app-version change, before
/// `Registry::open` runs its migrations. `PRAGMA user_version` records the app
/// version that last opened the DB, so the copy happens exactly once per
/// upgrade — leaving the last pre-migration state recoverable if a migration
/// in a new build corrupts or drops data.
///
/// Best-effort by design: it must never block startup, so a failed copy is
/// reported to the caller to log rather than propagated as a fatal error.
/// Returns the backup path when one was taken.
///
/// Call this *before* `Registry::open` on the same path.
pub fn backup_before_migrations(db_path: &Path, app_version: &str) -> Result<Option<PathBuf>> {
    let current = crate::version::encode(app_version);
    let existed = db_path.exists();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Read and stamp through short-lived connections so the file is never open
    // while it is being copied.
    let stored: i64 = {
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening db at {} for version check", db_path.display()))?;
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))?
    };

    if stored == current {
        return Ok(None);
    }

    // A DB that did not exist a moment ago is a fresh install: nothing worth
    // preserving, just stamp it.
    let backup = if existed {
        let bak = db_path.with_extension("db.bak");
        std::fs::copy(db_path, &bak)
            .with_context(|| format!("backing up {} to {}", db_path.display(), bak.display()))?;
        Some(bak)
    } else {
        None
    };

    let conn = Connection::open(db_path)
        .with_context(|| format!("opening db at {} to stamp version", db_path.display()))?;
    conn.pragma_update(None, "user_version", current)?;
    Ok(backup)
}

/// Derive a project's 3-letter issue key from its name: word initials for
/// multi-word names (padded from the first word), the first three letters
/// otherwise. `used` keys are avoided by substituting the last position with
/// later letters of the name, then A–Z.
pub fn derive_issue_key(name: &str, used: &[String]) -> String {
    let words: Vec<Vec<char>> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().map(|c| c.to_ascii_uppercase()).collect())
        .collect();
    let letters: Vec<char> = words.iter().flatten().copied().collect();

    let mut key: Vec<char> = if words.len() >= 2 {
        words.iter().take(3).map(|w| w[0]).collect()
    } else {
        letters.iter().take(3).copied().collect()
    };
    // Pad short keys. Initials-based keys borrow the rest of the first word
    // ("Todo App" -> TA -> TAO); single-word keys already used those letters,
    // so both fall through to 'X' padding ("Go" -> GOX).
    if key.len() < 3 {
        if words.len() >= 2 {
            if let Some(first) = words.first() {
                for &c in first.iter().skip(1) {
                    if key.len() >= 3 {
                        break;
                    }
                    key.push(c);
                }
            }
        }
        while key.len() < 3 {
            key.push('X');
        }
    }

    let taken = |k: &[char]| used.iter().any(|u| u.chars().eq(k.iter().copied()));
    if !taken(&key) {
        return key.into_iter().collect();
    }
    // Collision: try later letters of the name in the last position, then A–Z.
    for c in letters.into_iter().skip(3).chain('A'..='Z') {
        key[2] = c;
        if !taken(&key) {
            return key.into_iter().collect();
        }
    }
    // 26+ collisions on the same prefix: give up on uniqueness gracefully.
    key.into_iter().collect()
}

fn row_to_issue(row: &rusqlite::Row) -> Result<Issue> {
    let status: String = row.get(5)?;
    Ok(Issue {
        id: row.get(0)?,
        project_id: row.get(1)?,
        seq: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        status: IssueStatus::parse(&status)?,
        priority: row.get::<_, i64>(6)? as u8,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        due: row.get(9)?,
        scheduled: row.get(10)?,
        rank: row.get(11)?,
    })
}

fn row_to_profile(row: &rusqlite::Row) -> Result<AgentProfile> {
    let args: String = row.get(2)?;
    let env: String = row.get(3)?;
    let resume_args: Option<String> = row.get(4)?;
    Ok(AgentProfile {
        name: row.get(0)?,
        command: row.get(1)?,
        args: serde_json::from_str(&args)?,
        env: serde_json::from_str(&env)?,
        resume_args: match resume_args {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        },
        loop_args: match row.get::<_, Option<String>>(5)? {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        },
    })
}

fn row_to_project(row: &rusqlite::Row) -> Result<Project> {
    let repo_path: String = row.get(2)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repo_path: PathBuf::from(repo_path),
        default_agent: row.get(3)?,
        default_provider: row.get(4)?,
        color: row.get(5)?,
        issue_key: row.get(6)?,
        kind: row.get(7)?,
    })
}

fn row_to_run(row: &rusqlite::Row) -> Result<Run> {
    let port_base: Option<i64> = row.get(7)?;
    Ok(Run {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent: row.get(2)?,
        prompt: row.get(3)?,
        base: row.get(4)?,
        branch: row.get(5)?,
        created_at: row.get(6)?,
        port_base: port_base.map(|p| p as u16),
        archived_at: row.get(8)?,
        title: row.get(9)?,
        kind: row.get(10)?,
        merge_target: row.get(11)?,
        race_id: row.get(12)?,
        loop_config: match row.get::<_, Option<String>>(13)? {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        },
        loop_state: match row.get::<_, Option<String>>(14)? {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        },
        issue_id: row.get(15)?,
        worktree: row.get::<_, i64>(16)? != 0,
    })
}

fn row_to_run_session(row: &rusqlite::Row) -> Result<RunSession> {
    Ok(RunSession {
        id: row.get(0)?,
        run_id: row.get(1)?,
        agent: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn backup_skips_fresh_install_but_stamps_the_version() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("agency.db");

        let taken = backup_before_migrations(&db, "0.1.0").unwrap();
        assert!(taken.is_none(), "a fresh install has nothing to back up");
        assert!(!db.with_extension("db.bak").exists());

        // The stamp landed, so the next same-version launch is a no-op.
        let taken = backup_before_migrations(&db, "0.1.0").unwrap();
        assert!(taken.is_none());
    }

    #[test]
    fn backup_runs_once_per_version_change_and_preserves_pre_migration_data() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("agency.db");
        let bak = db.with_extension("db.bak");

        // An existing DB from a build that predates the stamp (user_version 0).
        {
            let reg = Registry::open(&db).unwrap();
            reg.add_project("Agency", dir.path()).unwrap();
        }

        // First launch of the stamping build backs the old DB up.
        let taken = backup_before_migrations(&db, "0.1.0").unwrap();
        assert_eq!(taken.as_deref(), Some(bak.as_path()));
        assert!(bak.exists());

        // Same version again: no second copy.
        std::fs::write(&bak, b"sentinel").unwrap();
        assert!(backup_before_migrations(&db, "0.1.0").unwrap().is_none());
        assert_eq!(std::fs::read(&bak).unwrap(), b"sentinel");

        // A version bump takes a fresh copy, overwriting the stale sentinel.
        {
            let reg = Registry::open(&db).unwrap();
            reg.add_project("Second", dir.path()).unwrap();
        }
        assert!(backup_before_migrations(&db, "0.2.0").unwrap().is_some());

        // The backup is a real, readable DB holding the pre-upgrade rows.
        let restored = Registry::open(&bak).unwrap();
        let names: Vec<String> =
            restored.list_projects().unwrap().into_iter().map(|p| p.name).collect();
        assert!(names.contains(&"Agency".to_string()));
        assert!(names.contains(&"Second".to_string()));
    }

    #[test]
    fn copy_without_runs_keeps_the_workspace_and_drops_the_runs() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("agency.db");
        {
            let reg = Registry::open(&src).unwrap();
            let project = reg.add_project("Agency", dir.path()).unwrap();
            reg.set_setting("theme", "mocha").unwrap();
            let mut run = sample_run("run-1", None);
            run.project_id = project.id.clone();
            reg.insert_run(&run).unwrap();
            reg.insert_run_session(&RunSession {
                id: "run-1--2".into(),
                run_id: "run-1".into(),
                agent: "claude".into(),
                created_at: 1,
            })
            .unwrap();
            reg.insert_review_comment(&ReviewComment {
                id: "c1".into(),
                run_id: "run-1".into(),
                path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 1,
                body: "look here".into(),
                sent: false,
                created_at: 1,
            })
            .unwrap();
        }

        let dst = dir.path().join("dev/agency.db");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        copy_without_runs(&src, &dst).unwrap();

        let copy = Registry::open(&dst).unwrap();
        let projects = copy.list_projects().unwrap();
        assert_eq!(projects.len(), 1, "the seeded db opens on the same projects");
        assert_eq!(copy.get_setting("theme").unwrap().as_deref(), Some("mocha"));
        assert!(
            copy.list_runs(&projects[0].id).unwrap().is_empty(),
            "a run's session, worktree and branch belong to the app that started it"
        );
        assert!(copy.list_run_sessions("run-1").unwrap().is_empty());
        assert!(copy.list_review_comments("run-1").unwrap().is_empty());

        // The source is untouched — this is a copy, not a move.
        let orig = Registry::open(&src).unwrap();
        assert_eq!(orig.list_runs(&projects[0].id).unwrap().len(), 1);
    }

    fn sample_run(id: &str, port: Option<u16>) -> Run {
        Run {
            id: id.to_string(),
            project_id: "proj".to_string(),
            agent: "claude".to_string(),
            prompt: "do a thing".to_string(),
            base: "HEAD".to_string(),
            branch: format!("agent/{id}"),
            created_at: 42,
            port_base: port,
            race_id: None,
            archived_at: None,
            title: None,
            kind: "agent".to_string(),
            merge_target: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: true,
        }
    }

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
    fn run_roundtrips_port_base() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        let got = reg.get_run("x-1").unwrap().unwrap();
        assert_eq!(got.port_base, Some(5200));
    }

    #[test]
    fn list_port_bases_returns_only_assigned() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", None)).unwrap();
        reg.insert_run(&sample_run("x-3", Some(5210))).unwrap();
        let mut bases = reg.list_port_bases().unwrap();
        bases.sort();
        assert_eq!(bases, vec![5200, 5210]);
    }

    #[test]
    fn run_sessions_roundtrip_and_cascade() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", None)).unwrap();
        for (n, at) in [(2, 10), (3, 20)] {
            reg.insert_run_session(&RunSession {
                id: format!("x-1--{n}"),
                run_id: "x-1".to_string(),
                agent: "claude".to_string(),
                created_at: at,
            })
            .unwrap();
        }
        let listed = reg.list_run_sessions("x-1").unwrap();
        assert_eq!(
            listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["x-1--2", "x-1--3"]
        );
        assert_eq!(reg.get_run_session("x-1--2").unwrap().unwrap().agent, "claude");

        reg.delete_run_session("x-1--2").unwrap();
        assert_eq!(reg.list_run_sessions("x-1").unwrap().len(), 1);
        reg.delete_run_sessions("x-1").unwrap();
        assert!(reg.list_run_sessions("x-1").unwrap().is_empty());
    }

    #[test]
    fn migrates_legacy_runs_table_without_port_base() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("legacy.db");
        // Create a runs table WITHOUT port_base, as older builds did.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL,
                    prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
                 VALUES ('old-1','proj','claude','p','HEAD','agent/old-1',1)",
                [],
            )
            .unwrap();
        }
        // Opening through Registry must add the column and preserve the row.
        let reg = Registry::open(&db).unwrap();
        let got = reg.get_run("old-1").unwrap().unwrap();
        assert_eq!(got.port_base, None);
        reg.insert_run(&sample_run("new-1", Some(5200))).unwrap();
        assert_eq!(reg.list_port_bases().unwrap(), vec![5200]);
    }

    #[test]
    fn archiving_hides_from_list_runs_and_shows_in_archived() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", Some(5210))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();

        let active: Vec<String> = reg.list_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(active, vec!["x-2"]);
        let archived: Vec<String> = reg.list_archived_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(archived, vec!["x-1"]);
    }

    #[test]
    fn archived_run_port_is_not_listed_as_used() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", Some(5210))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();
        let mut bases = reg.list_port_bases().unwrap();
        bases.sort();
        assert_eq!(bases, vec![5210]); // 5200 freed by archiving x-1
    }

    #[test]
    fn set_archived_none_restores_to_active() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();
        reg.set_archived("x-1", None).unwrap();
        let active: Vec<String> = reg.list_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(active, vec!["x-1"]);
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
    fn run_roundtrips_title() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        assert_eq!(reg.get_run("x-1").unwrap().unwrap().title, None);
        reg.set_run_title("x-1", "Add hunk staging").unwrap();
        assert_eq!(
            reg.get_run("x-1").unwrap().unwrap().title.as_deref(),
            Some("Add hunk staging"),
        );
    }

    #[test]
    fn migrates_legacy_runs_table_without_title() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("legacy-title.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL,
                    prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
                 VALUES ('old-1','proj','claude','p','HEAD','agent/old-1',1)",
                [],
            )
            .unwrap();
        }
        let reg = Registry::open(&db).unwrap();
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().title, None);
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().kind, "agent");
        reg.set_run_title("old-1", "Recovered").unwrap();
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().title.as_deref(), Some("Recovered"));
    }

    #[test]
    fn run_kind_defaults_to_agent_and_roundtrips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("kind.db")).unwrap();
        reg.insert_run(&sample_run("a-1", None)).unwrap();
        assert_eq!(reg.get_run("a-1").unwrap().unwrap().kind, "agent");

        let mut term = sample_run("t-1", None);
        term.kind = "terminal".to_string();
        reg.insert_run(&term).unwrap();
        assert_eq!(reg.get_run("t-1").unwrap().unwrap().kind, "terminal");
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

    #[test]
    fn profile_resume_args_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.upsert_profile(&AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        })
        .unwrap();
        reg.upsert_profile(&AgentProfile {
            name: "cursor".into(),
            command: "cursor-agent".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
        assert_eq!(
            reg.get_profile("claude").unwrap().unwrap().resume_args,
            Some(vec!["--continue".into()])
        );
        assert_eq!(reg.get_profile("cursor").unwrap().unwrap().resume_args, None);
    }

    #[test]
    fn ensure_profile_resume_args_only_sets_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.upsert_profile(&AgentProfile {
            name: "claude".into(), command: "claude".into(),
            args: vec![], env: vec![], resume_args: None, loop_args: None,
        }).unwrap();
        // Unset -> gets set.
        reg.ensure_profile_resume_args("claude", &Some(vec!["--continue".into()])).unwrap();
        assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, Some(vec!["--continue".into()]));
        // Already set -> not clobbered.
        reg.ensure_profile_resume_args("claude", &Some(vec!["--other".into()])).unwrap();
        assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, Some(vec!["--continue".into()]));
    }

    #[test]
    fn migrates_legacy_profiles_table_without_resume_args() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE profiles (name TEXT PRIMARY KEY, command TEXT NOT NULL, args TEXT NOT NULL, env TEXT NOT NULL);
                 INSERT INTO profiles (name, command, args, env) VALUES ('claude','claude','[]','[]');",
            ).unwrap();
        }
        // Opening must add the column and read the legacy row as resume_args = None.
        let reg = Registry::open(&path).unwrap();
        assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, None);
    }

    #[test]
    fn profile_loop_args_round_trip_and_ensure_only_sets_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.upsert_profile(&AgentProfile {
            name: "claude".into(), command: "claude".into(),
            args: vec![], env: vec![], resume_args: None, loop_args: None,
        }).unwrap();
        assert_eq!(reg.get_profile("claude").unwrap().unwrap().loop_args, None);
        // Unset -> gets seeded.
        reg.ensure_profile_loop_args("claude", &Some(vec!["-p".into(), "{{prompt}}".into()])).unwrap();
        assert_eq!(
            reg.get_profile("claude").unwrap().unwrap().loop_args,
            Some(vec!["-p".into(), "{{prompt}}".into()])
        );
        // Already set -> not clobbered.
        reg.ensure_profile_loop_args("claude", &Some(vec!["--other".into()])).unwrap();
        assert_eq!(
            reg.get_profile("claude").unwrap().unwrap().loop_args,
            Some(vec!["-p".into(), "{{prompt}}".into()])
        );
    }

    #[test]
    fn run_loop_config_and_state_round_trip() {
        use crate::loops::{LoopConfig, LoopState, LoopStatus};
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("loops.db")).unwrap();
        let mut run = sample_run("l-1", None);
        run.loop_config = Some(LoopConfig {
            check_command: "cargo test".into(),
            max_attempts: 10,
            check_timeout_secs: 600,
        });
        run.loop_state = Some(LoopState::new(7));
        reg.insert_run(&run).unwrap();

        let got = reg.get_run("l-1").unwrap().unwrap();
        assert_eq!(got.loop_config.as_ref().unwrap().check_command, "cargo test");
        assert_eq!(got.loop_state.as_ref().unwrap().status, LoopStatus::AwaitingAgent);
        assert_eq!(got.loop_state.as_ref().unwrap().attempt, 1);

        // Progress persists via set_loop_state.
        let mut st = got.loop_state.unwrap();
        st.attempt = 3;
        st.status = LoopStatus::Checking;
        reg.set_loop_state("l-1", &st).unwrap();
        let again = reg.get_run("l-1").unwrap().unwrap().loop_state.unwrap();
        assert_eq!(again.attempt, 3);
        assert_eq!(again.status, LoopStatus::Checking);

        // Non-loop runs stay None.
        reg.insert_run(&sample_run("plain", None)).unwrap();
        let plain = reg.get_run("plain").unwrap().unwrap();
        assert!(plain.loop_config.is_none() && plain.loop_state.is_none());
    }

    #[test]
    fn run_merge_target_round_trips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("merge-target.db")).unwrap();
        let project = reg.add_project("p", std::path::Path::new("/tmp/p")).unwrap();
        let run = Run {
            id: "t1".into(),
            project_id: project.id.clone(),
            agent: "claude".into(),
            prompt: "hi".into(),
            base: "main".into(),
            branch: "agent/t1".into(),
            created_at: 1,
            port_base: None,
            archived_at: None,
            title: None,
            kind: "agent".into(),
            merge_target: Some("develop".into()),
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: true,
        };
        reg.insert_run(&run).unwrap();
        let got = reg.get_run("t1").unwrap().unwrap();
        assert_eq!(got.merge_target.as_deref(), Some("develop"));
    }

    /// A run created without a worktree round-trips as one: the flag is what
    /// every teardown/merge guard keys off, so a lost `false` would let discard
    /// delete the branch the user is working on.
    #[test]
    fn run_worktree_flag_round_trips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("worktree-flag.db")).unwrap();
        let mut in_repo = sample_run("in-repo", None);
        in_repo.worktree = false;
        in_repo.branch = "main".into();
        reg.insert_run(&in_repo).unwrap();
        reg.insert_run(&sample_run("isolated", None)).unwrap();
        assert!(!reg.get_run("in-repo").unwrap().unwrap().worktree);
        assert!(reg.get_run("isolated").unwrap().unwrap().worktree);
    }

    // ── issues ─────────────────────────────────────────────────────────────

    #[test]
    fn issue_crud_roundtrip() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("issues.db")).unwrap();
        let a = reg.create_issue("proj", "Fix login", "steps to repro", IssueStatus::Todo, 100).unwrap();
        assert_eq!((a.seq, a.priority, a.status), (1, 0, IssueStatus::Todo));
        assert_eq!((a.created_at, a.updated_at), (100, 100));
        let b = reg.create_issue("proj", "Add board", "", IssueStatus::Backlog, 101).unwrap();
        assert_eq!(b.seq, 2);

        let listed = reg.list_issues("proj").unwrap();
        assert_eq!(listed.iter().map(|i| i.seq).collect::<Vec<_>>(), vec![1, 2]);

        // The mutation path is upsert_issue_row (files are truth; state.rs
        // computes the new value and the row follows). id survives on update.
        let mut next = a.clone();
        next.title = "Fix login flow".into();
        next.status = IssueStatus::Cancelled;
        next.priority = 4;
        next.due = Some("2026-08-01".into());
        next.scheduled = Some("2026-07-30".into());
        next.rank = Some(1.5);
        next.updated_at = 200;
        let updated = reg.upsert_issue_row(&next).unwrap();
        assert_eq!(updated.id, a.id);
        assert_eq!(updated.title, "Fix login flow");
        assert_eq!(updated.body, "steps to repro"); // untouched
        assert_eq!(updated.status, IssueStatus::Cancelled);
        assert_eq!(updated.priority, 4);
        assert_eq!(updated.due.as_deref(), Some("2026-08-01"));
        assert_eq!(updated.scheduled.as_deref(), Some("2026-07-30"));
        assert_eq!(updated.rank, Some(1.5));
        assert_eq!(updated.updated_at, 200);
        // list/get surface the new fields; clearing them round-trips too.
        assert_eq!(reg.list_issues("proj").unwrap()[0].due.as_deref(), Some("2026-08-01"));
        next.due = None;
        next.scheduled = None;
        next.rank = None;
        let cleared = reg.upsert_issue_row(&next).unwrap();
        assert_eq!((cleared.due, cleared.scheduled, cleared.rank), (None, None, None));

        reg.delete_issue(&a.id).unwrap();
        assert!(reg.get_issue(&a.id).unwrap().is_none());
        assert_eq!(reg.list_issues("proj").unwrap().len(), 1);
    }

    #[test]
    fn issue_seq_is_per_project_and_never_reused() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("issues.db")).unwrap();
        let a1 = reg.create_issue("pa", "a1", "", IssueStatus::Todo, 1).unwrap();
        let a2 = reg.create_issue("pa", "a2", "", IssueStatus::Todo, 2).unwrap();
        let b1 = reg.create_issue("pb", "b1", "", IssueStatus::Todo, 3).unwrap();
        assert_eq!((a1.seq, a2.seq, b1.seq), (1, 2, 1));

        // Deleting the highest-numbered issue must not free its number.
        reg.delete_issue(&a2.id).unwrap();
        let a3 = reg.create_issue("pa", "a3", "", IssueStatus::Todo, 4).unwrap();
        assert_eq!(a3.seq, 3);
    }

    #[test]
    fn advance_issue_status_is_forward_only() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("issues.db")).unwrap();
        let i = reg.create_issue("proj", "t", "", IssueStatus::Todo, 1).unwrap();

        // Forward: applied.
        assert!(reg.advance_issue_status(&i.id, IssueStatus::InProgress, 2).unwrap());
        assert_eq!(reg.get_issue(&i.id).unwrap().unwrap().status, IssueStatus::InProgress);

        // Backward or same: refused (a manual advance is never demoted).
        assert!(!reg.advance_issue_status(&i.id, IssueStatus::Todo, 3).unwrap());
        assert!(!reg.advance_issue_status(&i.id, IssueStatus::InProgress, 3).unwrap());

        // Skipping ahead is fine (merge closes an issue that never saw a PR).
        assert!(reg.advance_issue_status(&i.id, IssueStatus::Done, 4).unwrap());

        // Cancelled is outside the progression: never a source nor a target.
        let c = reg.create_issue("proj", "c", "", IssueStatus::Cancelled, 5).unwrap();
        assert!(!reg.advance_issue_status(&c.id, IssueStatus::InProgress, 6).unwrap());
        assert!(!reg.advance_issue_status(&i.id, IssueStatus::Cancelled, 6).unwrap());

        // Missing issue: quietly a no-op (runs can outlive their issue).
        assert!(!reg.advance_issue_status("nope", IssueStatus::Done, 7).unwrap());
    }

    #[test]
    fn rollback_issue_to_todo_only_from_agent_states() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("issues.db")).unwrap();
        let i = reg.create_issue("proj", "t", "", IssueStatus::InProgress, 1).unwrap();
        assert!(reg.rollback_issue_to_todo(&i.id, 2).unwrap());
        assert_eq!(reg.get_issue(&i.id).unwrap().unwrap().status, IssueStatus::Todo);

        // Already todo / done / cancelled: untouched.
        assert!(!reg.rollback_issue_to_todo(&i.id, 3).unwrap());
        let mut done = reg.get_issue(&i.id).unwrap().unwrap();
        done.status = IssueStatus::Done;
        reg.upsert_issue_row(&done).unwrap();
        assert!(!reg.rollback_issue_to_todo(&i.id, 5).unwrap());
        assert_eq!(reg.get_issue(&i.id).unwrap().unwrap().status, IssueStatus::Done);
    }

    #[test]
    fn runs_for_issue_lists_only_active_linked_runs() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("issues.db")).unwrap();
        let issue = reg.create_issue("proj", "t", "", IssueStatus::Todo, 1).unwrap();
        let mut r1 = sample_run("x-1", None);
        r1.issue_id = Some(issue.id.clone());
        let mut r2 = sample_run("x-2", None);
        r2.issue_id = Some(issue.id.clone());
        reg.insert_run(&r1).unwrap();
        reg.insert_run(&r2).unwrap();
        reg.insert_run(&sample_run("x-3", None)).unwrap(); // unlinked

        assert_eq!(reg.runs_for_issue(&issue.id).unwrap().len(), 2);
        assert_eq!(reg.get_run("x-1").unwrap().unwrap().issue_id.as_deref(), Some(issue.id.as_str()));

        reg.set_archived("x-1", Some(1000)).unwrap();
        let active: Vec<String> = reg.runs_for_issue(&issue.id).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(active, vec!["x-2"]);
    }

    #[test]
    fn derive_issue_key_shapes_and_collisions() {
        let none: Vec<String> = vec![];
        assert_eq!(derive_issue_key("Agency", &none), "AGE");
        assert_eq!(derive_issue_key("Todo App", &none), "TAO"); // T + A, padded from "Todo"
        assert_eq!(derive_issue_key("My Cool Project", &none), "MCP");
        assert_eq!(derive_issue_key("Go", &none), "GOX"); // short name padded with X
        assert_eq!(derive_issue_key("a-b_c d", &none), "ABC"); // separators split words

        // Collision: last slot walks later letters of the name, then A-Z.
        let used = vec!["AGE".to_string()];
        assert_eq!(derive_issue_key("Agency", &used), "AGN");
        let used = vec!["AGE".into(), "AGN".into(), "AGC".into(), "AGY".into()];
        assert_eq!(derive_issue_key("Agency", &used), "AGA");
    }

    #[test]
    fn projects_get_issue_keys_assigned_and_backfilled() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("keys.db");
        {
            let reg = Registry::open(&db).unwrap();
            let p = reg.add_project("Agency", std::path::Path::new("/tmp/a")).unwrap();
            assert_eq!(p.issue_key.as_deref(), Some("AGE"));
            // Same-name project gets a distinct key.
            let q = reg.add_project("Agency", std::path::Path::new("/tmp/b")).unwrap();
            assert_eq!(q.issue_key.as_deref(), Some("AGN"));
        }
        // Legacy rows (pre-issue_key) are backfilled on open.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, repo_path, color, issue_key) VALUES ('old', 'Zebra', '/tmp/z', 'blue', NULL)",
                [],
            )
            .unwrap();
        }
        let reg = Registry::open(&db).unwrap();
        assert_eq!(reg.get_project("old").unwrap().unwrap().issue_key.as_deref(), Some("ZEB"));
    }

    #[test]
    fn list_projects_sorts_case_insensitively() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("order.db")).unwrap();
        for (name, path) in [("zebra", "/tmp/z"), ("Apple", "/tmp/a"), ("banana", "/tmp/b")] {
            reg.add_project(name, std::path::Path::new(path)).unwrap();
        }
        let names: Vec<String> = reg.list_projects().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Apple", "banana", "zebra"]);
    }

    #[test]
    fn set_project_color_accepts_only_palette_names() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("color.db")).unwrap();
        let p = reg.add_project("Agency", std::path::Path::new("/tmp/a")).unwrap();

        reg.set_project_color(&p.id, "teal").unwrap();
        assert_eq!(reg.get_project(&p.id).unwrap().unwrap().color.as_deref(), Some("teal"));

        // A non-palette name would render as an undefined CSS var; rejected,
        // and the stored color is left alone.
        assert!(reg.set_project_color(&p.id, "chartreuse").is_err());
        assert_eq!(reg.get_project(&p.id).unwrap().unwrap().color.as_deref(), Some("teal"));
    }

    #[test]
    fn migrates_legacy_runs_table_without_issue_id() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("legacy-issue.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL,
                    prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
                 VALUES ('old-1','proj','claude','p','HEAD','agent/old-1',1)",
                [],
            )
            .unwrap();
        }
        let reg = Registry::open(&db).unwrap();
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().issue_id, None);
        let mut linked = sample_run("new-1", None);
        linked.issue_id = Some("iss-1".into());
        reg.insert_run(&linked).unwrap();
        assert_eq!(reg.get_run("new-1").unwrap().unwrap().issue_id.as_deref(), Some("iss-1"));
    }

    // ── workspace ──────────────────────────────────────────────────────────

    #[test]
    fn project_kind_defaults_to_none_and_roundtrips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("kind.db")).unwrap();
        let p = reg.add_project("Agency", std::path::Path::new("/tmp/a")).unwrap();
        assert_eq!(p.kind, None);
        assert_eq!(reg.get_project(&p.id).unwrap().unwrap().kind, None);

        let ws = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
        assert_eq!(ws.kind.as_deref(), Some(PROJECT_KIND_WORKSPACE));
        assert_eq!(reg.get_project(&ws.id).unwrap().unwrap().kind.as_deref(), Some("workspace"));
    }

    #[test]
    fn ensure_workspace_is_idempotent_revives_closed_and_adopts_new_path() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("ws.db")).unwrap();
        assert!(reg.get_workspace().unwrap().is_none());

        let ws = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
        // Second call returns the same row, not a duplicate.
        let again = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
        assert_eq!(again.id, ws.id);

        // A closed workspace comes back on ensure.
        reg.set_project_closed(&ws.id, true).unwrap();
        assert!(reg.list_projects().unwrap().iter().all(|p| p.id != ws.id));
        let revived = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
        assert_eq!(revived.id, ws.id);
        assert!(reg.list_projects().unwrap().iter().any(|p| p.id == ws.id));

        // Re-created at a new location: the row adopts it.
        let moved = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws2")).unwrap();
        assert_eq!(moved.id, ws.id);
        assert_eq!(moved.repo_path, std::path::PathBuf::from("/tmp/ws2"));
        assert_eq!(
            reg.get_workspace().unwrap().unwrap().repo_path,
            std::path::PathBuf::from("/tmp/ws2")
        );
    }

    #[test]
    fn workspace_gets_an_issue_key_and_backfill_covers_legacy_rows() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("ws-keys.db");
        {
            let reg = Registry::open(&db).unwrap();
            // Phase 5: the workspace is in the tracker like any project.
            let ws = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
            assert_eq!(ws.issue_key.as_deref(), Some("WOR"));
            // A pre-Phase-5 workspace row (key-less) simulated by clearing it.
            reg.conn
                .execute("UPDATE projects SET issue_key = NULL WHERE id = ?1", [&ws.id])
                .unwrap();
        }
        // Reopening runs the backfill — the workspace is included now.
        let reg = Registry::open(&db).unwrap();
        assert_eq!(reg.get_workspace().unwrap().unwrap().issue_key.as_deref(), Some("WOR"));
        let p = reg.add_project("Zebra", std::path::Path::new("/tmp/z")).unwrap();
        assert_eq!(p.issue_key.as_deref(), Some("ZEB"));
    }

    // ── issues as files: index helpers ─────────────────────────────────────

    #[test]
    fn alloc_issue_seq_counts_and_never_reuses() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("seq.db")).unwrap();
        assert_eq!(reg.alloc_issue_seq("p1").unwrap(), 1);
        assert_eq!(reg.alloc_issue_seq("p1").unwrap(), 2);
        assert_eq!(reg.alloc_issue_seq("p2").unwrap(), 1);
        // Interleaves with create_issue's numbering.
        let i = reg.create_issue("p1", "t", "", IssueStatus::Todo, 1).unwrap();
        assert_eq!(i.seq, 3);
    }

    #[test]
    fn ensure_issue_seq_at_least_is_monotonic() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("seq2.db")).unwrap();
        // Fresh project: an externally-created AGE-15 pushes the counter past it.
        reg.ensure_issue_seq_at_least("p1", 15).unwrap();
        assert_eq!(reg.alloc_issue_seq("p1").unwrap(), 16);
        // Never lowers.
        reg.ensure_issue_seq_at_least("p1", 3).unwrap();
        assert_eq!(reg.alloc_issue_seq("p1").unwrap(), 17);
    }

    #[test]
    fn upsert_issue_row_keeps_id_on_update() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("upsert.db")).unwrap();
        let issue = Issue {
            id: "uuid-1".into(),
            project_id: "p1".into(),
            seq: 4,
            title: "First".into(),
            body: "b".into(),
            status: IssueStatus::Todo,
            priority: 1,
            due: None,
            scheduled: None,
            rank: None,
            created_at: 10,
            updated_at: 10,
        };
        let stored = reg.upsert_issue_row(&issue).unwrap();
        assert_eq!(stored, issue);
        // Same (project, seq) with a different candidate id: fields follow the
        // file, the row's id survives.
        let mut edited = issue.clone();
        edited.id = "uuid-2".into();
        edited.title = "Edited".into();
        edited.status = IssueStatus::Done;
        edited.updated_at = 20;
        let stored = reg.upsert_issue_row(&edited).unwrap();
        assert_eq!(stored.id, "uuid-1");
        assert_eq!(stored.title, "Edited");
        assert_eq!(stored.status, IssueStatus::Done);
        assert_eq!(reg.list_issues("p1").unwrap().len(), 1);
    }

    #[test]
    fn issue_patch_distinguishes_absent_from_null() {
        // Absent nullable fields deserialize to None (untouched)…
        let p: IssuePatch = serde_json::from_str(r#"{"title":"t"}"#).unwrap();
        assert_eq!(p.title.as_deref(), Some("t"));
        assert_eq!((p.due, p.scheduled, p.rank), (None, None, None));
        // …explicit null to Some(None) (clear)…
        let p: IssuePatch = serde_json::from_str(r#"{"due":null,"rank":null}"#).unwrap();
        assert_eq!(p.due, Some(None));
        assert_eq!(p.rank, Some(None));
        assert_eq!(p.scheduled, None);
        // …and a value to Some(Some(v)).
        let p: IssuePatch =
            serde_json::from_str(r#"{"due":"2026-08-01","scheduled":"2026-07-30","rank":1.5}"#)
                .unwrap();
        assert_eq!(p.due, Some(Some("2026-08-01".into())));
        assert_eq!(p.scheduled, Some(Some("2026-07-30".into())));
        assert_eq!(p.rank, Some(Some(1.5)));
    }

    #[test]
    fn issues_migrated_flag_roundtrips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("mig.db")).unwrap();
        let p = reg.add_project("Agency", std::path::Path::new("/tmp/a")).unwrap();
        assert!(!reg.project_issues_migrated(&p.id).unwrap());
        reg.mark_issues_migrated(&p.id).unwrap();
        assert!(reg.project_issues_migrated(&p.id).unwrap());
    }

    #[test]
    fn set_project_repo_path_moves_the_row() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("move.db")).unwrap();
        let ws = reg.ensure_workspace("Workspace", std::path::Path::new("/tmp/ws")).unwrap();
        reg.set_project_repo_path(&ws.id, std::path::Path::new("/tmp/elsewhere")).unwrap();
        assert_eq!(
            reg.get_project(&ws.id).unwrap().unwrap().repo_path,
            std::path::PathBuf::from("/tmp/elsewhere")
        );
    }
}
