//! Which data directory a build owns.
//!
//! Everything the app keeps outside a repo lives in one directory Tauri derives
//! from the bundle identifier: the sqlite registry (`agency.db`) and the
//! terminal daemon's socket (`termd.sock`). Derived from the identifier, that
//! path is the same for the installed Agency.app and for anything started by
//! `./dev.sh` — so both used to share one DB and, through one socket, one
//! daemon process. Two bad consequences, both of which cost live agent
//! sessions:
//!
//! - A dev build inherits whichever daemon binary is already running, so
//!   daemon-side changes cannot be tested without killing the daemon the
//!   installed app's sessions live in.
//! - A branch that bumps `PROTOCOL_VERSION` makes the two incompatible, and the
//!   side that starts second shuts the running daemon down (`TermClient::
//!   connect_or_spawn`), ending every session it hosts. Running a dev build
//!   should never be able to kill the agents running in the release app.
//!
//! So a `tauri dev` build gets its own directory next door — own DB, own
//! socket, own daemon — seeded once from the installed app's DB so it opens
//! with the same projects instead of an empty window. The two never meet.
//!
//! The marker is `tauri::is_dev()`, which is true for `tauri dev` (the CLI
//! turns on the `custom-protocol` feature only when bundling) rather than
//! `debug_assertions`, so `tauri dev --release` still counts as dev and a
//! locally bundled app still counts as the real thing.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The sqlite registry, relative to the data dir.
pub const DB_NAME: &str = "agency.db";

/// Appended to the release data dir's name for dev builds.
const DEV_SUFFIX: &str = ".dev";

/// The data dir this build owns: the one Tauri derived for a bundled build, a
/// `<identifier>.dev` sibling for a `tauri dev` build. Pure, so the naming is
/// testable without touching the filesystem.
fn for_build(release_dir: &Path, dev: bool) -> PathBuf {
    if !dev {
        return release_dir.to_path_buf();
    }
    match release_dir.file_name() {
        Some(name) => {
            let mut name = name.to_os_string();
            name.push(DEV_SUFFIX);
            release_dir.with_file_name(name)
        }
        // No trailing component to rename (a bare root — never a real app data
        // dir). Nest instead of returning `release_dir`: sharing it is the very
        // thing this module exists to prevent.
        None => release_dir.join("dev"),
    }
}

/// Resolve the data dir for this build and make sure it exists. A dev dir with
/// no DB yet is seeded from the installed app's, minus its runs (see
/// `registry::copy_without_runs`), so a dev build opens on the real projects
/// while the runs — with their live sessions, worktrees and branches — stay the
/// property of the app that started them.
///
/// Seeding is best-effort: no installed app to copy from, or an unreadable DB,
/// just means the dev build starts empty.
pub fn prepare(release_dir: &Path, dev: bool) -> Result<PathBuf> {
    let dir = for_build(release_dir, dev);
    let needs_seed = dev && !dir.join(DB_NAME).exists();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating data dir {}", dir.display()))?;
    if dev {
        log::info!("dev build: using data dir {} (own db and termd daemon)", dir.display());
    }
    let src = release_dir.join(DB_NAME);
    if needs_seed && src.exists() {
        match agency_core::registry::copy_without_runs(&src, &dir.join(DB_NAME)) {
            Ok(()) => log::info!("seeded dev db from {} (runs not copied)", src.display()),
            Err(e) => log::warn!("could not seed dev db from {}: {e:#}", src.display()),
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_build_gets_a_sibling_dir() {
        let release = Path::new("/Users/x/Library/Application Support/build.agency.app");
        assert_eq!(
            for_build(release, true),
            Path::new("/Users/x/Library/Application Support/build.agency.app.dev"),
            "a dev build must not resolve the installed app's dir: same dir means one \
             termd.sock, and a protocol bump on a branch then kills the release app's sessions"
        );
    }

    #[test]
    fn bundled_build_keeps_the_real_dir() {
        let release = Path::new("/Users/x/Library/Application Support/build.agency.app");
        assert_eq!(for_build(release, false), release);
    }

    #[test]
    fn seeds_a_fresh_dev_dir_without_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let release = tmp.path().join("build.agency.app");
        std::fs::create_dir_all(&release).unwrap();
        let db = release.join(DB_NAME);
        {
            let reg = agency_core::registry::Registry::open(&db).unwrap();
            let project = reg.add_project("proj", Path::new("/tmp/repo")).unwrap();
            reg.insert_run(&agency_core::registry::Run {
                id: "run-1".into(),
                project_id: project.id.clone(),
                agent: "claude".into(),
                prompt: "do a thing".into(),
                base: "main".into(),
                branch: "agent/thing".into(),
                created_at: 1,
                port_base: None,
                archived_at: None,
                title: None,
                kind: "agent".into(),
                merge_target: None,
                race_id: None,
                loop_config: None,
                loop_state: None,
                issue_id: None,
                worktree: true,
                model: None,
                base_commit: None,
                primary_closed_at: None,
                standing: None,
                pin_rank: None,
            })
            .unwrap();
        }

        let dev = prepare(&release, true).unwrap();
        assert_eq!(dev, release.with_file_name("build.agency.app.dev"));

        let reg = agency_core::registry::Registry::open(&dev.join(DB_NAME)).unwrap();
        let projects = reg.list_projects().unwrap();
        assert_eq!(projects.len(), 1, "the dev build should open on the same projects");
        assert!(
            reg.list_runs(&projects[0].id).unwrap().is_empty(),
            "runs belong to the app that started them: their sessions, worktrees and \
             branches are live over there"
        );
    }

    #[test]
    fn existing_dev_db_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let release = tmp.path().join("build.agency.app");
        std::fs::create_dir_all(&release).unwrap();
        {
            let reg = agency_core::registry::Registry::open(&release.join(DB_NAME)).unwrap();
            reg.add_project("proj", Path::new("/tmp/repo")).unwrap();
        }
        // A dev dir that already has its own DB: re-seeding would clobber the
        // projects and settings the dev build accumulated.
        let dev = release.with_file_name("build.agency.app.dev");
        std::fs::create_dir_all(&dev).unwrap();
        agency_core::registry::Registry::open(&dev.join(DB_NAME)).unwrap();

        assert_eq!(prepare(&release, true).unwrap(), dev);
        let reg = agency_core::registry::Registry::open(&dev.join(DB_NAME)).unwrap();
        assert!(reg.list_projects().unwrap().is_empty(), "the dev db was overwritten");
    }
}
