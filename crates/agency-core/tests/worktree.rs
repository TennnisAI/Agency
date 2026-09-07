use agency_core::worktree::WorktreeManager;
use std::process::Command;
use tempfile::tempdir;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@t.t"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("README.md"), "hi").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

fn branch_exists(dir: &std::path::Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn remove_keep_branch_keeps_the_branch_then_restore_recreates_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    assert!(wt.path.exists());
    assert!(branch_exists(repo.path(), "agent/task-1"));

    // Archive: worktree gone, branch kept.
    mgr.remove_keep_branch("task-1").unwrap();
    assert!(!wt.path.exists(), "worktree dir removed");
    assert!(branch_exists(repo.path(), "agent/task-1"), "branch kept");

    // Restore: worktree recreated on the same branch.
    let restored = mgr.restore("task-1").unwrap();
    assert!(restored.path.exists(), "worktree recreated");
    assert_eq!(restored.branch, "agent/task-1");
}

#[test]
fn remove_after_keep_branch_deletes_the_branch_without_error() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-2", "HEAD").unwrap();
    mgr.remove_keep_branch("task-2").unwrap();
    // Discarding an archived run: worktree already gone, but the branch must go.
    mgr.remove("task-2").unwrap();
    assert!(!branch_exists(repo.path(), "agent/task-2"), "branch deleted on discard");
}

#[test]
fn create_makes_worktree_and_branch() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    assert_eq!(wt.task_id, "task-1");
    assert_eq!(wt.branch, "agent/task-1");
    assert!(wt.path.ends_with(".agency/worktrees/task-1"));
    assert!(wt.path.join("README.md").exists());
    // The exclude file keeps agency artifacts untracked-invisible.
    let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".agency/"));
}

#[test]
fn list_returns_created_worktrees() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-1", "HEAD").unwrap();
    mgr.create("task-2", "HEAD").unwrap();
    let mut ids: Vec<String> = mgr.list().unwrap().into_iter().map(|w| w.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["task-1".to_string(), "task-2".to_string()]);
}

#[test]
fn remove_deletes_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    mgr.remove("task-1").unwrap();
    assert!(!wt.path.exists());
    assert!(mgr.list().unwrap().is_empty());
}

#[test]
fn commit_all_if_dirty_preserves_work_on_the_branch() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-wip", "HEAD").unwrap();

    // Clean worktree: no commit made.
    assert!(!mgr.commit_all_if_dirty("task-wip", "WIP").unwrap());

    // Dirty worktree (tracked edit + untracked file): committed.
    std::fs::write(wt.path.join("README.md"), "edited").unwrap();
    std::fs::write(wt.path.join("untracked.txt"), "new").unwrap();
    assert!(mgr.commit_all_if_dirty("task-wip", "WIP: archive").unwrap());

    // Worktree is clean afterwards and the commit is on the agent branch.
    assert!(!mgr.commit_all_if_dirty("task-wip", "WIP").unwrap());
    let out = Command::new("git")
        .args(["log", "-1", "--format=%s", "agent/task-wip"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "WIP: archive");

    // Archive+restore round-trip brings the work back.
    mgr.remove_keep_branch("task-wip").unwrap();
    let restored = mgr.restore("task-wip").unwrap();
    assert_eq!(std::fs::read_to_string(restored.path.join("untracked.txt")).unwrap(), "new");
}

#[test]
fn commit_all_if_dirty_is_noop_for_missing_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    assert!(!mgr.commit_all_if_dirty("no-such-task", "WIP").unwrap());
}

#[test]
fn copy_into_copies_untracked_files_and_dirs_skipping_bad_paths() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-copy", "HEAD").unwrap();

    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir_all(repo.path().join("config/certs")).unwrap();
    std::fs::write(repo.path().join("config/certs/dev.pem"), "pem").unwrap();

    let copied = mgr
        .copy_into(
            "task-copy",
            &[
                ".env".to_string(),
                "config".to_string(),
                "missing.txt".to_string(),
                "../escape".to_string(),
                "/abs/path".to_string(),
            ],
        )
        .unwrap();
    assert_eq!(copied, vec![".env".to_string(), "config".to_string()]);
    assert_eq!(std::fs::read_to_string(wt.path.join(".env")).unwrap(), "SECRET=1");
    assert_eq!(std::fs::read_to_string(wt.path.join("config/certs/dev.pem")).unwrap(), "pem");
    assert!(!wt.path.join("missing.txt").exists());
}

#[test]
fn copy_essentials_auto_copies_untracked_root_env_files() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-env", "HEAD").unwrap();

    // Untracked local secrets — the frustrating case: not committed, so a
    // worktree wouldn't otherwise get them.
    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::write(repo.path().join(".env.local"), "LOCAL=2").unwrap();
    // A tracked env file must NOT be re-copied (it already materializes).
    std::fs::write(repo.path().join(".env.example"), "EXAMPLE=tracked").unwrap();
    git(repo.path(), &["add", ".env.example"]);
    git(repo.path(), &["commit", "-q", "-m", "add example"]);

    let detected = mgr.default_env_files();
    assert_eq!(detected, vec![".env".to_string(), ".env.local".to_string()]);

    // With no configured copy list, the env defaults still land in the worktree.
    let copied = mgr.copy_essentials("task-env", &[]).unwrap();
    assert_eq!(copied, vec![".env".to_string(), ".env.local".to_string()]);
    assert_eq!(std::fs::read_to_string(wt.path.join(".env")).unwrap(), "SECRET=1");
    assert_eq!(std::fs::read_to_string(wt.path.join(".env.local")).unwrap(), "LOCAL=2");
}

#[test]
fn copy_essentials_unions_configured_list_with_env_defaults_without_dupes() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-union", "HEAD").unwrap();

    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir_all(repo.path().join("config")).unwrap();
    std::fs::write(repo.path().join("config/dev.pem"), "pem").unwrap();

    // `.env` is listed explicitly AND auto-detected — it must appear once.
    let copied =
        mgr.copy_essentials("task-union", &[".env".to_string(), "config".to_string()]).unwrap();
    assert_eq!(copied, vec![".env".to_string(), "config".to_string()]);
}

#[test]
fn create_on_branch_checks_out_existing_branch() {
    let repo = init_repo();
    git(repo.path(), &["branch", "feat/existing"]);
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create_on_branch("task-pr", "feat/existing").unwrap();
    assert!(wt.path.exists());
    assert_eq!(wt.branch, "feat/existing");
    // No agent/ branch was created for it.
    assert!(!branch_exists(repo.path(), "agent/task-pr"));
}

/// AGE-203: the project folder is dragged somewhere else in Finder. Agency
/// keeps its worktrees inside it, so they move too, and both halves of every
/// worktree link record absolute paths: the worktree's `.git` file points at
/// `<repo>/.git/worktrees/<id>`, and that directory's `gitdir` file points back
/// at the worktree. After the move neither resolves, and every agent's tab
/// reports "not a git repository" until `repair` rewrites them.
#[test]
fn repair_reattaches_worktrees_after_the_project_folder_moves() {
    let parent = tempdir().unwrap();
    let repo = parent.path().join("proj");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@t.t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let mgr = WorktreeManager::new(repo.clone());
    mgr.create("task-1", "HEAD").unwrap();

    let moved = parent.path().join("proj-elsewhere");
    std::fs::rename(&repo, &moved).unwrap();
    let wt = moved.join(".agency").join("worktrees").join("task-1");
    assert!(wt.is_dir(), "the worktree moved with the folder");
    let broken =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&wt).output().unwrap();
    assert!(!broken.status.success(), "the moved worktree's git link is stale");

    let moved_mgr = WorktreeManager::new(moved.clone());
    moved_mgr.repair().unwrap();

    let fixed =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&wt).output().unwrap();
    assert!(
        fixed.status.success(),
        "repair must reattach the worktree: {}",
        String::from_utf8_lossy(&fixed.stderr)
    );
    // And the main repo knows where the worktree went, so archive, merge and
    // restore address the right tree.
    let listed = moved_mgr.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].task_id, "task-1");
    assert_eq!(listed[0].branch, "agent/task-1");

    // Repair on a repo whose worktrees are all present and correct is a no-op,
    // not a failure: the reconnect calls it unconditionally.
    moved_mgr.repair().unwrap();
    assert_eq!(moved_mgr.list().unwrap().len(), 1);
}

/// AGE-203, second half: the reconnect must not take a worktree the *user*
/// made with it. `repair` used to run `git worktree prune` after repairing our
/// own trees, and after the folder moved every other worktree inside it is
/// prunable too: git answered "Removing worktrees/feature: gitdir file points
/// to non-existent location" and deleted the admin entry, at which point the
/// checkout is dead and hand-repair no longer possible ("unable to locate
/// repository"), stranding whatever was uncommitted in it.
#[test]
fn repair_reattaches_a_users_own_worktree_too() {
    let parent = tempdir().unwrap();
    let repo = parent.path().join("proj");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@t.t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let mgr = WorktreeManager::new(repo.clone());
    mgr.create("task-1", "HEAD").unwrap();
    // Theirs: inside the project folder, so it moves too, but nowhere near
    // `.agency/worktrees` and so not one of the paths repair is given.
    git(&repo, &["worktree", "add", "-q", "-b", "feature", "wt/feature"]);
    std::fs::write(repo.join("wt").join("feature").join("notes.txt"), "unsaved").unwrap();

    let moved = parent.path().join("proj-elsewhere");
    std::fs::rename(&repo, &moved).unwrap();
    let moved_mgr = WorktreeManager::new(moved.clone());
    moved_mgr.repair().unwrap();

    // Ours is reattached, theirs is still registered.
    let ours = moved.join(".agency").join("worktrees").join("task-1");
    let fixed =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&ours).output().unwrap();
    assert!(fixed.status.success(), "repair must reattach our own worktree");
    assert!(
        moved.join(".git").join("worktrees").join("feature").is_dir(),
        "the user's worktree must keep its admin entry: pruning it is unrecoverable"
    );

    // And it is mended, not merely spared: repair matches it back from the path
    // git still records, so the user does not have to know that a folder they
    // dragged in Finder left a repair to run by hand. The file that was never
    // committed is still sitting in it.
    let theirs = moved.join("wt").join("feature");
    let mended =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&theirs).output().unwrap();
    assert!(
        mended.status.success(),
        "the user's worktree must be reattached too: {}",
        String::from_utf8_lossy(&mended.stderr)
    );
    assert_eq!(std::fs::read_to_string(theirs.join("notes.txt")).unwrap(), "unsaved");
}

/// The same hazard one step along: `remove` used to run the repo-wide
/// `git worktree prune` to clear the entry of a worktree that had already gone
/// from disk, which took every other stale entry with it. Archiving one agent
/// is an ordinary thing to do just after a reconnect, and the user's own
/// worktrees can still be stale then (an unmounted volume repair cannot reach).
#[test]
fn remove_deregisters_only_its_own_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-1", "HEAD").unwrap();
    git(repo.path(), &["worktree", "add", "-q", "-b", "feature", "wt/feature"]);

    // Ours is gone from disk (so `worktree remove` cannot deregister it), and
    // theirs is somewhere git does not know to look.
    std::fs::remove_dir_all(repo.path().join(".agency").join("worktrees").join("task-1")).unwrap();
    std::fs::rename(repo.path().join("wt").join("feature"), repo.path().join("wt").join("moved"))
        .unwrap();

    mgr.remove("task-1").unwrap();

    assert!(
        !repo.path().join(".git").join("worktrees").join("task-1").is_dir(),
        "our own entry must go: a later create for the same id needs the name back"
    );
    assert!(
        repo.path().join(".git").join("worktrees").join("feature").is_dir(),
        "the user's entry must survive: it is theirs, and pruning it is unrecoverable"
    );
    assert!(!branch_exists(repo.path(), "agent/task-1"));
}

#[test]
fn remove_deregisters_its_own_worktree_recorded_by_a_relative_gitdir() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-1", "HEAD").unwrap();
    let admin = repo.path().join(".git").join("worktrees").join("task-1");

    // What git 2.48's `worktree.useRelativePaths` (and `worktree add
    // --relative-paths`) writes: the tree's path relative to the admin dir,
    // not the absolute one. Written by hand rather than by setting the config,
    // so the case is covered whatever git the machine running the tests has.
    std::fs::write(admin.join("gitdir"), "../../../.agency/worktrees/task-1/.git\n").unwrap();

    // Gone from disk, and locked so that `git worktree remove --force` refuses
    // (it wants a second `-f` for a lock) — which is the only way to reach the
    // deregistering fallback on a git new enough to deregister a missing tree
    // by itself.
    git(repo.path(), &["worktree", "lock", ".agency/worktrees/task-1"]);
    std::fs::remove_dir_all(repo.path().join(".agency").join("worktrees").join("task-1")).unwrap();

    mgr.remove("task-1").unwrap();

    assert!(
        !admin.is_dir(),
        "a relative gitdir still names our own worktree: the entry must go, \
         or a later create for the same id has no name left to use"
    );
}
