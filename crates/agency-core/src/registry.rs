use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::profile::AgentProfile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub default_agent: Option<String>,
    pub default_provider: Option<String>,
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
                env TEXT NOT NULL
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
                created_at INTEGER NOT NULL
            );",
        )?;
        Ok(Registry { conn })
    }

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            repo_path: repo_path.to_path_buf(),
            default_agent: None,
            default_provider: None,
        };
        self.conn.execute(
            "INSERT INTO projects (id, name, repo_path, default_agent, default_provider)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                project.id,
                project.name,
                project.repo_path.to_string_lossy(),
                project.default_agent,
                project.default_provider,
            ],
        )?;
        Ok(project)
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider
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
            "SELECT id, name, repo_path, default_agent, default_provider
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
        self.conn.execute(
            "INSERT INTO profiles (name, command, args, env) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET command = ?2, args = ?3, env = ?4",
            rusqlite::params![p.name, p.command, args, env],
        )?;
        Ok(())
    }

    pub fn get_profile(&self, name: &str) -> Result<Option<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env FROM profiles WHERE name = ?1")?;
        let mut rows = stmt.query([name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_profile(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env FROM profiles ORDER BY name")?;
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
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch, run.created_at
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at FROM runs WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_run(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at
             FROM runs WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_run(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM runs WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_profile(row: &rusqlite::Row) -> Result<AgentProfile> {
    let args: String = row.get(2)?;
    let env: String = row.get(3)?;
    Ok(AgentProfile {
        name: row.get(0)?,
        command: row.get(1)?,
        args: serde_json::from_str(&args)?,
        env: serde_json::from_str(&env)?,
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
    })
}

fn row_to_run(row: &rusqlite::Row) -> Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent: row.get(2)?,
        prompt: row.get(3)?,
        base: row.get(4)?,
        branch: row.get(5)?,
        created_at: row.get(6)?,
    })
}
