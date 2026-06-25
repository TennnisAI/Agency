use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Staged (index) status code: one of M A D R C ? ! or space.
    pub index: String,
    /// Unstaged (worktree) status code.
    pub worktree: String,
}

/// Run a git command in `worktree`, returning stdout on success.
fn git(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()?;
    if !output.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn status(worktree: &Path) -> Result<Vec<FileChange>> {
    let out = git(worktree, &["status", "--porcelain"])?;
    let mut changes = Vec::new();
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let index = line[0..1].to_string();
        let work = line[1..2].to_string();
        // Path begins at column 3 (after "XY ").
        let mut path = line[3..].to_string();
        // Renames are "orig -> new"; keep the new path.
        if let Some(idx) = path.find(" -> ") {
            path = path[idx + 4..].to_string();
        }
        changes.push(FileChange {
            path,
            index,
            worktree: work,
        });
    }
    Ok(changes)
}

pub fn diff(worktree: &Path, path: &str, staged: bool) -> Result<String> {
    if staged {
        git(worktree, &["diff", "--cached", "--", path])
    } else {
        git(worktree, &["diff", "--", path])
    }
}

pub fn stage(worktree: &Path, path: &str) -> Result<()> {
    git(worktree, &["add", "--", path])?;
    Ok(())
}

pub fn unstage(worktree: &Path, path: &str) -> Result<()> {
    // `restore --staged` requires git >= 2.23; available on all supported setups.
    git(worktree, &["restore", "--staged", "--", path])?;
    Ok(())
}

pub fn commit(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "-m", message])?;
    Ok(())
}

pub fn push(worktree: &Path) -> Result<()> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    git(worktree, &["push", "-u", "origin", &branch])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffStat {
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

pub fn diff_stat(worktree: &Path, base: &str) -> Result<DiffStat> {
    let range = format!("{base}...HEAD");
    let out = git(worktree, &["diff", "--numstat", &range])?;
    let mut stat = DiffStat { added: 0, deleted: 0, files: 0 };
    for line in out.lines() {
        let mut parts = line.split('\t');
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        // Binary files show "-" for counts; treat as 0 but still count the file.
        stat.added += a.parse::<u32>().unwrap_or(0);
        stat.deleted += d.parse::<u32>().unwrap_or(0);
        stat.files += 1;
    }
    Ok(stat)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub header: String,
    pub hunks: Vec<Hunk>,
}

pub fn parse_diff(diff: &str) -> FileDiff {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut seen_hunk = false;

    for line in diff.lines() {
        if line.starts_with("@@") {
            seen_hunk = true;
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(h) = current.as_mut() {
            h.lines.push(line.to_string());
        } else if !seen_hunk {
            header_lines.push(line);
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }

    let header = if header_lines.is_empty() {
        String::new()
    } else {
        let mut s = header_lines.join("\n");
        s.push('\n');
        s
    };
    FileDiff { header, hunks }
}

pub fn git_stdin(worktree: &Path, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("failed to open git stdin"))?
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn build_hunk_patch(file_diff: &FileDiff, hunk_index: usize) -> Result<String> {
    let hunk = file_diff
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    let mut patch = file_diff.header.clone();
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

pub fn stage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, false)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(worktree, &["apply", "--cached", "--unidiff-zero", "-"], &patch)
}

pub fn unstage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, true)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(
        worktree,
        &["apply", "--cached", "--reverse", "--unidiff-zero", "-"],
        &patch,
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: i64,
    pub subject: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub base: Option<String>,
}

/// HEAD ancestry, newest first. Fields are unit-separated (\x1f); parents and
/// refs are space/comma lists. %D yields "HEAD -> main, origin/main, tag: v1".
pub fn log_graph(worktree: &Path, limit: usize) -> Result<Vec<HistoryItem>> {
    let limit_arg = format!("-n{limit}");
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D";
    let out = git(worktree, &["log", &limit_arg, format])?;
    let mut items = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        if f.len() < 7 {
            continue;
        }
        let parents = f[1].split_whitespace().map(str::to_string).collect();
        let refs = f[6]
            .split(',')
            .map(|r| r.trim().trim_start_matches("HEAD -> ").to_string())
            .filter(|r| !r.is_empty())
            .collect();
        items.push(HistoryItem {
            hash: f[0].to_string(),
            parents,
            author: f[2].to_string(),
            email: f[3].to_string(),
            date: f[4].parse().unwrap_or(0),
            subject: f[5].to_string(),
            refs,
        });
    }
    Ok(items)
}

pub fn branch_info(worktree: &Path) -> Result<BranchInfo> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    let upstream = git(worktree, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() {
        if let Ok(counts) = git(worktree, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) {
            let mut p = counts.split_whitespace();
            behind = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            ahead = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    // Base = merge-base with the first reachable default branch.
    let base = ["origin/HEAD", "main", "master"].iter().find_map(|cand| {
        git(worktree, &["merge-base", "HEAD", cand])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    Ok(BranchInfo { branch, upstream, ahead, behind, base })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranches {
    /// The branch currently checked out in the primary worktree (or "HEAD" if detached).
    pub current: String,
    /// All local branch names, with `current` first.
    pub branches: Vec<String>,
}

pub fn list_branches(repo: &Path) -> Result<ProjectBranches> {
    let current = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    let raw = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
    let mut branches: Vec<String> = raw.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    // Put the current branch first so the UI can preselect it.
    if let Some(pos) = branches.iter().position(|b| b == &current) {
        branches.remove(pos);
        branches.insert(0, current.clone());
    }
    Ok(ProjectBranches { current, branches })
}

pub fn stage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["add", "-A"])?;
    Ok(())
}

pub fn unstage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["reset", "-q"])?;
    Ok(())
}

/// Discard one file's changes. Untracked files are deleted; tracked files are
/// restored from the index (mirrors VSCode "Discard Changes" on the Changes group).
pub fn discard(worktree: &Path, path: &str, untracked: bool) -> Result<()> {
    if untracked {
        git(worktree, &["clean", "-f", "--", path])?;
    } else {
        git(worktree, &["restore", "--", path])?;
    }
    Ok(())
}

pub fn discard_all(worktree: &Path) -> Result<()> {
    git(worktree, &["restore", "--", "."])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitFile {
    pub path: String,
    pub status: String,
}

pub fn commit_files(worktree: &Path, hash: &str) -> Result<Vec<CommitFile>> {
    // --format= strips commit metadata; root commits are handled by `show`.
    let out = git(worktree, &["show", "--name-status", "--format=", hash])?;
    let mut files = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("").chars().next().unwrap_or(' ').to_string();
        // Renames have two paths "R100 old new"; keep the last.
        let path = parts.last().unwrap_or("").to_string();
        if !path.is_empty() {
            files.push(CommitFile { path, status });
        }
    }
    Ok(files)
}

pub fn commit_diff(worktree: &Path, hash: &str, path: &str) -> Result<String> {
    git(worktree, &["show", "--format=", hash, "--", path])
}

pub fn commit_amend(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "--amend", "-m", message])?;
    Ok(())
}

/// Build a patch applying only `selected` body lines of one hunk.
/// Unselected '+' lines are dropped; unselected '-' lines become context so
/// they are preserved. The hunk header counts are recomputed. When `reverse`
/// is true (unstaging the cached diff) the roles of +/- are swapped for counting.
pub fn build_partial_patch(
    fd: &FileDiff,
    hunk_index: usize,
    selected: &[usize],
    reverse: bool,
) -> Result<String> {
    let hunk = fd
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    // Parse "@@ -old_start,old_len +new_start,new_len @@".
    let (old_start, new_start) = parse_hunk_starts(&hunk.header)?;
    let mut body: Vec<String> = Vec::new();
    let (mut old_len, mut new_len) = (0u32, 0u32);
    // The patch is applied forward (stage, file == old side) or reversed
    // (unstage/revert, file == new side). An unselected change must be turned
    // into context on whichever side matches the file being patched, and
    // dropped from the other; otherwise that side won't match and the apply
    // is rejected.
    for (i, line) in hunk.lines.iter().enumerate() {
        let kind = line.chars().next().unwrap_or(' ');
        let sel = selected.contains(&i);
        match kind {
            '+' => {
                if sel {
                    body.push(line.clone());
                    new_len += 1;
                } else if reverse {
                    // Already present on the new side (the file we reverse onto):
                    // keep as context so it survives.
                    body.push(format!(" {}", &line[1..]));
                    old_len += 1;
                    new_len += 1;
                }
                // forward + unselected add: drop entirely
            }
            '-' => {
                if sel {
                    body.push(line.clone());
                    old_len += 1;
                } else if !reverse {
                    // Present on the old side (the file we apply onto): keep as
                    // context so it survives.
                    body.push(format!(" {}", &line[1..]));
                    old_len += 1;
                    new_len += 1;
                }
                // reverse + unselected delete: drop entirely
            }
            _ => {
                body.push(line.clone());
                old_len += 1;
                new_len += 1;
            }
        }
    }
    let mut patch = fd.header.clone();
    patch.push_str(&format!(
        "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"
    ));
    for l in body {
        patch.push_str(&l);
        patch.push('\n');
    }
    Ok(patch)
}

fn parse_hunk_starts(header: &str) -> Result<(u32, u32)> {
    // header like "@@ -1,1 +1,3 @@ optional"
    let core = header.trim_start_matches("@@").trim();
    let mut parts = core.split_whitespace();
    let old = parts.next().unwrap_or("");
    let new = parts.next().unwrap_or("");
    let old_start = old.trim_start_matches('-').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    let new_start = new.trim_start_matches('+').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    Ok((old_start, new_start))
}

pub fn stage_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, false)?;
    git_stdin(worktree, &["apply", "--cached", "-"], &patch)
}

pub fn unstage_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, true)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, true)?;
    git_stdin(worktree, &["apply", "--cached", "--reverse", "-"], &patch)
}

pub fn revert_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    // Reverse-applied onto the working tree (the new side), so use reverse framing.
    let patch = build_partial_patch(&fd, hunk_index, selected, true)?;
    git_stdin(worktree, &["apply", "--reverse", "-"], &patch)
}

#[cfg(test)]
mod branch_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn run(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git").args(args).current_dir(dir).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn list_branches_returns_current_first_and_all_locals() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        run(repo, &["init", "-q", "-b", "main"]);
        run(repo, &["config", "user.email", "t@t"]);
        run(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        run(repo, &["add", "."]);
        run(repo, &["commit", "-qm", "init"]);
        run(repo, &["branch", "develop"]);

        let pb = list_branches(repo).unwrap();
        assert_eq!(pb.current, "main");
        assert_eq!(pb.branches[0], "main", "current branch must be first");
        assert!(pb.branches.contains(&"main".to_string()));
        assert!(pb.branches.contains(&"develop".to_string()));
    }
}
