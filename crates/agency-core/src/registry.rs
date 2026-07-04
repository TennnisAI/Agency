use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::profile::AgentProfile;

/// Theme accent names the UI resolves to CSS vars (`var(--<name>)`). Order is
/// the assignment preference for new projects.
const PROJECT_COLORS: [&str; 9] =
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
}

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
            CREATE TABLE IF NOT EXISTS review_comments (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                body TEXT NOT NULL,
                sent INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
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
        let reg = Registry { conn };
        reg.backfill_project_colors()?;
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

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            repo_path: repo_path.to_path_buf(),
            default_agent: None,
            default_provider: None,
            color: Some(self.pick_project_color()?),
        };
        self.conn.execute(
            "INSERT INTO projects (id, name, repo_path, default_agent, default_provider, color)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                project.id,
                project.name,
                project.repo_path.to_string_lossy(),
                project.default_agent,
                project.default_provider,
                project.color,
            ],
        )?;
        Ok(project)
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

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider, color
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
            "SELECT id, name, repo_path, default_agent, default_provider, color
             FROM projects ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_project(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
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
        self.conn.execute(
            "INSERT INTO profiles (name, command, args, env, resume_args) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET command = ?2, args = ?3, env = ?4, resume_args = ?5",
            rusqlite::params![p.name, p.command, args, env, resume],
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

    pub fn get_profile(&self, name: &str) -> Result<Option<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env, resume_args FROM profiles WHERE name = ?1")?;
        let mut rows = stmt.query([name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_profile(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env, resume_args FROM profiles ORDER BY name")?;
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
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at, run.title, run.kind,
                run.merge_target, run.race_id
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id FROM runs WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_run(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id
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
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target, race_id
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
        })
        .unwrap();
        reg.upsert_profile(&AgentProfile {
            name: "cursor".into(),
            command: "cursor-agent".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
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
            args: vec![], env: vec![], resume_args: None,
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
        };
        reg.insert_run(&run).unwrap();
        let got = reg.get_run("t1").unwrap().unwrap();
        assert_eq!(got.merge_target.as_deref(), Some("develop"));
    }
}
