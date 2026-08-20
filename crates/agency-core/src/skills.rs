//! The per-worktree skills kit.
//!
//! Agency already writes each agent's MCP config into a fresh worktree in that
//! agent's native format ([`crate::mcp`]) and introduces the issue tracker in
//! `AGENTS.md` ([`crate::briefing`]). This is the third file drop from the same
//! hook points: a small kit of skills, namespaced `agency-*`, in the location
//! the agent already reads project-local skills from.
//!
//! Two skills, both aimed at things a dispatched agent is reliably bad at
//! without help:
//!
//! - `agency-date-range` is a deterministic resolver. A model asked for "last
//!   quarter" does the calendar arithmetic in its head and is wrong often
//!   enough to matter; the resolver is a hundred lines of shell that is never
//!   wrong.
//! - `agency-workspace` is a catalog of the workspace the agent was dropped
//!   into: the worktree and its branch, the project checkout, the tracker's
//!   absolute path, the setup/run/check commands, and what happens to the work
//!   when the run ends.
//!
//! Only agents with a known project-local skills convention get an entry in
//! [`skills_target`]; everything else returns `None` and nothing is written.
//! Our directories are upserted one at a time, so a repo's own
//! `.claude/skills/` is never replaced, and a skill directory the repo *tracks*
//! is left completely alone: a diff on a tracked file rides into the project's
//! main branch at merge, which is not a change Agency gets to make.

use anyhow::Result;
use std::path::Path;

/// Skill directory names. Both are namespaced `agency-` so a repo's own skills
/// can never collide with ours, and so one exclude pattern covers the kit.
pub const DATE_SKILL: &str = "agency-date-range";
pub const WORKSPACE_SKILL: &str = "agency-workspace";

/// The file every agent's skill convention agrees on.
pub const SKILL_FILE: &str = "SKILL.md";

/// The resolver script, beside `agency-date-range/SKILL.md`.
pub const RESOLVER_FILE: &str = "resolve.sh";

/// What keeps the kit out of the run's diff. Anchored at the repo root and
/// matching only our namespace, so a repo's own `.claude/skills/` is untouched.
pub const EXCLUDE_PATTERN: &str = "/.claude/skills/agency-*";

/// Where `agent` reads project-local skills from: path segments under the
/// worktree. `None` means the agent has no project-local skills convention
/// Agency knows, and nothing is written for it.
///
/// Claude Code reads `.claude/skills/<name>/SKILL.md` from the directory it was
/// launched in. The other agents Agency ships either have no skills mechanism
/// or have only a user-scope one, and Agency writes inside the workspace only.
fn skills_target(agent: &str) -> Option<&'static [&'static str]> {
    match agent {
        "claude" => Some(&[".claude", "skills"]),
        _ => None,
    }
}

/// Whether Agency can emit a skills kit for this agent.
pub fn agent_supported(agent: &str) -> bool {
    skills_target(agent).is_some()
}

/// One generated file inside a skill directory.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillFile {
    pub name: &'static str,
    pub contents: String,
    /// Set the user-executable bit on unix. Scripts are documented as `sh
    /// <path>` so the bit is a convenience, not a dependency.
    pub executable: bool,
}

/// One skill: a directory and the files in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: &'static str,
    pub files: Vec<SkillFile>,
}

/// The facts the workspace catalog is written from. Everything the emitted
/// skill says is filled in here at dispatch: an agent that has to go read a
/// file to learn where it is has not been told where it is.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    /// The worktree the agent runs in.
    pub worktree: std::path::PathBuf,
    /// The project's own checkout, which holds the tracker and is where the
    /// branch merges back to.
    pub repo_root: std::path::PathBuf,
    /// This project's issue key prefix (`AGE`), so `AGE-14` reads as this
    /// tracker's rather than as a shape from some other one.
    pub issue_key: String,
    /// The branch the worktree is on.
    pub branch: String,
    /// `scripts.setup` from `.agency/agency.toml`: run before the agent starts.
    pub setup_command: Option<String>,
    /// Named `scripts.runs` entries, as (name, command).
    pub run_scripts: Vec<(String, String)>,
    /// A looping run's check command, and its attempt cap.
    pub loop_check: Option<(String, u32)>,
    /// The run's allocated port, exported as `AGENCY_PORT`.
    pub port: Option<u16>,
}

/// The kit for this workspace, to be written into `skills_dir` (which the
/// generated copy names, so the two can't drift). What [`emit_for_agent`]
/// writes and what the tests read: pure, so every line of agent-facing copy is
/// checkable without a worktree.
pub fn kit(ws: &Workspace, skills_dir: &Path) -> Vec<Skill> {
    vec![
        Skill {
            name: DATE_SKILL,
            files: vec![
                SkillFile {
                    name: SKILL_FILE,
                    contents: date_skill_md(&skills_dir.join(DATE_SKILL).join(RESOLVER_FILE)),
                    executable: false,
                },
                SkillFile {
                    name: RESOLVER_FILE,
                    contents: RESOLVER_SH.to_string(),
                    executable: true,
                },
            ],
        },
        Skill {
            name: WORKSPACE_SKILL,
            files: vec![SkillFile {
                name: SKILL_FILE,
                contents: workspace_skill_md(ws),
                executable: false,
            }],
        },
    ]
}

/// Write the kit into `ws.worktree` in `agent`'s convention, and keep it out of
/// git so no agent commits it into the project.
///
/// Returns whether anything was written. Best-effort per skill: a directory the
/// repo tracks is skipped with a reason logged, and the rest of the kit still
/// lands.
pub fn emit_for_agent(agent: &str, ws: &Workspace) -> Result<bool> {
    let Some(segments) = skills_target(agent) else {
        return Ok(false);
    };
    let root = segments.iter().fold(ws.worktree.clone(), |p, s| p.join(s));
    let mut wrote = false;
    for skill in kit(ws, &root) {
        let dir = root.join(skill.name);
        let rel = segments.join("/") + "/" + skill.name;
        if tracked(&ws.worktree, &rel) {
            log::info!(
                "{rel} is tracked in {}: leaving the repo's own skill alone",
                ws.worktree.display()
            );
            continue;
        }
        for file in &skill.files {
            let path = dir.join(file.name);
            crate::issuefs::atomic_write(&path, &file.contents)?;
            if file.executable {
                make_executable(&path);
            }
        }
        wrote = true;
    }
    if !wrote {
        return Ok(false);
    }
    // Worktrees get committed and turned into pull requests, and agents reach
    // for `git add -A` constantly, so a generated file that git offers lands in
    // the project at merge. `.git/info/exclude` and not the worktree's own
    // `.git/worktrees/<name>/info/exclude`: tested 2026-08-17, git reads only
    // the common dir's copy. Excluding costs the user nothing, since `git add
    // -f` still wins and an exclude has no say over a file once it is tracked.
    if let Err(e) = crate::worktree::ensure_exclude_pattern(&ws.repo_root, EXCLUDE_PATTERN) {
        log::warn!("excluding the skills kit in {}: {e}", ws.repo_root.display());
    }
    Ok(true)
}

/// Whether git tracks anything under `rel` in this worktree. A tracked path is
/// the repo's own; see [`crate::briefing`], which skips on the same test.
fn tracked(worktree: &Path, rel: &str) -> bool {
    std::process::Command::new("git")
        .args(["ls-files", "--", rel])
        .current_dir(worktree)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)) {
        log::warn!("marking {} executable: {e}", path.display());
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// `agency-date-range/SKILL.md`. The absolute path is the point: the agent's
/// cwd is not guaranteed to be the worktree root, and a relative path that
/// resolves to nothing reads as "the skill is broken".
fn date_skill_md(resolver: &Path) -> String {
    format!(
        "---\n\
         name: {DATE_SKILL}\n\
         description: Resolve a relative date expression (today, yesterday, this or last week, \
         month, quarter or year, week/month/quarter/year to date, last N days, last N months) \
         into exact start and end dates with a deterministic script. Use whenever a task, an \
         issue or a query mentions a period relative to now, instead of working the calendar \
         out by hand.\n\
         ---\n\
         \n\
         # Date ranges\n\
         \n\
         Calendar arithmetic done from memory is wrong often enough to matter, and it is wrong \
         quietly: month lengths, leap years, quarter boundaries and the year rollover all look \
         right until someone checks. Resolve the range with the script instead, then use the \
         dates it prints.\n\
         \n\
         ```sh\n\
         sh {resolver} <range> [--today YYYY-MM-DD] [--week-starts monday|sunday]\n\
         ```\n\
         \n\
         It prints `key=value` lines and nothing else:\n\
         \n\
         ```\n\
         range=last-month\n\
         start=2026-07-01\n\
         end=2026-07-31\n\
         days=31\n\
         today=2026-08-18\n\
         ```\n\
         \n\
         Both endpoints are inclusive. `last-7-days` ends today and includes it; \
         `last-3-months` is three whole calendar months ending with last month, so it never \
         includes a partial current one. Weeks start Monday unless you pass `--week-starts \
         sunday`.\n\
         \n\
         Run `sh {resolver} --help` for the full list of ranges. Beyond the named ones it also \
         takes a literal `YYYY-MM-DD`, `YYYY-MM` or `YYYY` and expands it to that day, month or \
         year.\n\
         \n\
         `--today` overrides the machine clock, which is what to use when the user names the \
         day to reckon from (\"as of the 1st, what was last quarter\"). Without it the script \
         reads the local date itself, which is the authority worth trusting: a date carried in \
         a prompt can be stale.\n\
         \n\
         Agency generates this skill. Edits are replaced on the next run.\n",
        resolver = resolver.display(),
    )
}

/// `agency-workspace/SKILL.md`: the catalog of the workspace this run was
/// dropped into.
fn workspace_skill_md(ws: &Workspace) -> String {
    let issues = ws.repo_root.join(crate::issuefs::ISSUES_DIR);
    let mut body = String::new();
    body.push_str(&format!(
        "---\n\
         name: {WORKSPACE_SKILL}\n\
         description: What this Agency workspace is: the worktree and branch this run owns, the \
         project checkout it merges back into, the absolute path of the issue tracker, the \
         setup, run and check commands, the AGENCY_* variables, and what happens to the work \
         when the run ends. Use before looking for a tracker, a build or test command, or a \
         place to put a file, and before deciding what to commit.\n\
         ---\n\
         \n\
         # This workspace\n\
         \n\
         Agency, the desktop app that dispatched you, cut this workspace for one run. It is a \
         git worktree, not a clone: a second checkout of the same repository, on a branch of \
         its own.\n\
         \n\
         - Worktree (your working copy): `{worktree}`\n\
         - Branch: `{branch}`\n\
         - Project checkout (where this branch merges back to): `{repo_root}`\n\
         - Issue tracker: `{issues}`\n\
         \n\
         The tracker is markdown, one file per issue, named for its key: this project's are \
         `{issue_key}-<n>`, so `{issue_key}-14.md`. It lives in the project checkout, not \
         here, and it is untracked by git, so there is no copy in this worktree, nothing to \
         commit and nothing to merge. An edit to an issue file takes effect as soon as it is \
         written. Read `{readme}` before filing, commenting or changing a status.\n",
        worktree = ws.worktree.display(),
        branch = ws.branch,
        repo_root = ws.repo_root.display(),
        issues = issues.display(),
        issue_key = ws.issue_key,
        readme = issues.join("README.md").display(),
    ));

    body.push_str("\n## Running the project\n\n");
    match &ws.setup_command {
        Some(cmd) => body.push_str(&format!(
            "This project's setup command, `{cmd}`, already ran in this worktree before you \
             started, so its output is in place.\n\n"
        )),
        None => body.push_str(
            "This project has no setup command configured, so nothing was built or installed \
             here before you started.\n\n",
        ),
    }
    if ws.run_scripts.is_empty() {
        body.push_str(
            "It has no run scripts configured either. Use whatever the repository's own \
             documentation says, from this worktree.\n",
        );
    } else {
        body.push_str(
            "Agency runs these for the user from the workspace's Run controls. You can run \
             them here too, and they are the commands the project is actually started with:\n\n",
        );
        for (name, command) in &ws.run_scripts {
            body.push_str(&format!("- {name}: `{command}`\n"));
        }
    }
    if let Some(port) = ws.port {
        body.push_str(&format!(
            "\nThis workspace's port block starts at {port}, exported as `AGENCY_PORT`. Bind to \
             it rather than the project's default port: every other workspace has its own \
             block, and that is what keeps two agents running the same server at once from \
             fighting over one port.\n"
        ));
    }
    body.push_str(&format!(
        "\n`AGENCY_WORKSPACE_PATH` (`{worktree}`), `AGENCY_ROOT_PATH` (`{repo_root}`) and \
         `AGENCY_WORKSPACE_NAME` are exported too.\n",
        worktree = ws.worktree.display(),
        repo_root = ws.repo_root.display(),
    ));

    if let Some((check, max_attempts)) = &ws.loop_check {
        body.push_str(&format!(
            "\n## This run is a loop\n\
             \n\
             When you exit, Agency runs `{check}` in this worktree. Exit 0 ends the loop as \
             complete; anything else re-invokes you here with the same prompt, up to \
             {max_attempts} attempts in total. Your uncommitted changes are committed to the \
             branch as `wip: loop attempt <n>` between attempts, so work is never lost between \
             them. Run that command yourself before you finish: it is the only thing that \
             decides whether the run is done.\n"
        ));
    }

    body.push_str(&format!(
        "\n## How this run finishes\n\
         \n\
         Commit your work on `{branch}`. What merges is the branch: uncommitted changes are \
         left behind by a merge, and discarding the run removes this worktree along with \
         anything not committed. Archiving is gentler, and commits what is left first.\n\
         \n\
         The user reviews the run in Agency and either merges the branch into the project or \
         opens a pull request from it. A merge closes the issue the run was dispatched from, \
         if there was one, so there is no status to set by hand at the end.\n\
         \n\
         Do not commit anything Agency generated here. `AGENTS.md`, \
         `.claude/skills/agency-*` and the MCP config (`.mcp.json`, or the equivalent for the \
         agent running here) are written into every worktree and excluded from git in the \
         repository's `.git/info/exclude`, which is why `git status` does not show them; \
         `git add -f` would defeat that permanently and put generated files in the project at \
         merge.\n\
         \n\
         Agency generates this skill. Edits are replaced on the next run.\n",
        branch = ws.branch,
    ));
    body
}

/// The resolver, verbatim. POSIX shell so it runs under whatever `sh` is on
/// the machine, and all arithmetic is on day numbers rather than on `date`
/// flags, because GNU and BSD `date` disagree on every arithmetic flag they
/// have. Exercised against real `sh` in the tests below.
const RESOLVER_SH: &str = r#"#!/bin/sh
# Resolve a named date range into exact start and end dates. Written by Agency;
# see SKILL.md beside it.
#
# All arithmetic is on day numbers (days since 1970-01-01) using the civil
# calendar conversions, so leap years, month lengths and year boundaries fall
# out of the algorithm rather than out of a table. Nothing here shells out to
# `date` except to read today, because GNU and BSD `date` disagree on every
# arithmetic flag they have.

set -u

usage() {
    cat <<'EOF'
usage: resolve.sh <range> [--today YYYY-MM-DD] [--week-starts monday|sunday]

Ranges (all inclusive of both endpoints):
  today  yesterday  tomorrow
  this-week  last-week  next-week      week-to-date
  this-month last-month next-month     month-to-date
  this-quarter last-quarter next-quarter  quarter-to-date
  this-year  last-year  next-year      year-to-date
  last-N-days   the N days ending today, today included
  next-N-days   the N days starting today, today included
  last-N-months the N calendar months ending with last month
  YYYY-MM-DD    that single day
  YYYY-MM       that whole calendar month
  YYYY          that whole calendar year

Prints key=value lines: range, start, end, days, today.
EOF
}

# Days since 1970-01-01 for a proleptic Gregorian date. Correct for any year
# >= 0, which covers every date this script can be handed.
to_days() {
    _y=$1
    _m=$2
    _d=$3
    if [ "$_m" -le 2 ]; then
        _y=$((_y - 1))
        _mp=$((_m + 9))
    else
        _mp=$((_m - 3))
    fi
    _era=$((_y / 400))
    _yoe=$((_y - _era * 400))
    _doy=$(((153 * _mp + 2) / 5 + _d - 1))
    _doe=$((_yoe * 365 + _yoe / 4 - _yoe / 100 + _doy))
    echo $((_era * 146097 + _doe - 719468))
}

# The inverse: day number back to YYYY-MM-DD.
from_days() {
    _z=$(($1 + 719468))
    _era=$((_z / 146097))
    _doe=$((_z - _era * 146097))
    _yoe=$(((_doe - _doe / 1460 + _doe / 36524 - _doe / 146096) / 365))
    _y=$((_yoe + _era * 400))
    _doy=$((_doe - (365 * _yoe + _yoe / 4 - _yoe / 100)))
    _mp=$(((5 * _doy + 2) / 153))
    _d=$((_doy - (153 * _mp + 2) / 5 + 1))
    if [ "$_mp" -lt 10 ]; then
        _m=$((_mp + 3))
    else
        _m=$((_mp - 9))
    fi
    if [ "$_m" -le 2 ]; then _y=$((_y + 1)); fi
    printf '%04d-%02d-%02d\n' "$_y" "$_m" "$_d"
}

# "08" is octal in $(( )), and August is not a valid octal number.
undecimal() {
    _s=$1
    while [ ${#_s} -gt 1 ]; do
        case $_s in
        0*) _s=${_s#0} ;;
        *) break ;;
        esac
    done
    echo "$_s"
}

die() {
    echo "resolve.sh: $1" >&2
    exit 2
}

is_num() {
    case $1 in
    '' | *[!0-9]*) return 1 ;;
    *) return 0 ;;
    esac
}

# Day number of the first of the month `n` months from y-m (n may be negative).
month_start() {
    _my=$1
    _mm=$(($2 + $3))
    while [ "$_mm" -lt 1 ]; do
        _mm=$((_mm + 12))
        _my=$((_my - 1))
    done
    while [ "$_mm" -gt 12 ]; do
        _mm=$((_mm - 12))
        _my=$((_my + 1))
    done
    to_days "$_my" "$_mm" 1
}

range=''
today=''
week_starts=monday
while [ $# -gt 0 ]; do
    case $1 in
    --today)
        [ $# -ge 2 ] || die "--today needs a YYYY-MM-DD date"
        today=$2
        shift 2
        ;;
    --week-starts)
        [ $# -ge 2 ] || die "--week-starts needs monday or sunday"
        week_starts=$2
        shift 2
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    -*) die "unknown option: $1" ;;
    *)
        [ -z "$range" ] || die "one range at a time (got '$range' and '$1')"
        range=$1
        shift
        ;;
    esac
done
[ -n "$range" ] || {
    usage >&2
    exit 2
}
[ -n "$today" ] || today=$(date +%Y-%m-%d)

case $today in
[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
*) die "today must be YYYY-MM-DD, got '$today'" ;;
esac
ty=$(undecimal "${today%%-*}")
tm=$(undecimal "$(echo "$today" | cut -d- -f2)")
td=$(undecimal "${today##*-}")
t=$(to_days "$ty" "$tm" "$td")

# 0 = the first day of the week, under whichever convention is in force.
if [ "$week_starts" = sunday ]; then
    dow=$(((t + 4) % 7))
elif [ "$week_starts" = monday ]; then
    dow=$(((t + 3) % 7))
else
    die "--week-starts takes monday or sunday, got '$week_starts'"
fi
week_start=$((t - dow))
quarter_first_month=$(((tm - 1) / 3 * 3 + 1))

case $range in
today)
    s=$t
    e=$t
    ;;
yesterday)
    s=$((t - 1))
    e=$s
    ;;
tomorrow)
    s=$((t + 1))
    e=$s
    ;;
this-week)
    s=$week_start
    e=$((s + 6))
    ;;
last-week)
    s=$((week_start - 7))
    e=$((s + 6))
    ;;
next-week)
    s=$((week_start + 7))
    e=$((s + 6))
    ;;
week-to-date)
    s=$week_start
    e=$t
    ;;
this-month)
    s=$(month_start "$ty" "$tm" 0)
    e=$(($(month_start "$ty" "$tm" 1) - 1))
    ;;
last-month)
    s=$(month_start "$ty" "$tm" -1)
    e=$(($(month_start "$ty" "$tm" 0) - 1))
    ;;
next-month)
    s=$(month_start "$ty" "$tm" 1)
    e=$(($(month_start "$ty" "$tm" 2) - 1))
    ;;
month-to-date)
    s=$(month_start "$ty" "$tm" 0)
    e=$t
    ;;
this-quarter)
    s=$(month_start "$ty" "$quarter_first_month" 0)
    e=$(($(month_start "$ty" "$quarter_first_month" 3) - 1))
    ;;
last-quarter)
    s=$(month_start "$ty" "$quarter_first_month" -3)
    e=$(($(month_start "$ty" "$quarter_first_month" 0) - 1))
    ;;
next-quarter)
    s=$(month_start "$ty" "$quarter_first_month" 3)
    e=$(($(month_start "$ty" "$quarter_first_month" 6) - 1))
    ;;
quarter-to-date)
    s=$(month_start "$ty" "$quarter_first_month" 0)
    e=$t
    ;;
this-year)
    s=$(to_days "$ty" 1 1)
    e=$(($(to_days $((ty + 1)) 1 1) - 1))
    ;;
last-year)
    s=$(to_days $((ty - 1)) 1 1)
    e=$(($(to_days "$ty" 1 1) - 1))
    ;;
next-year)
    s=$(to_days $((ty + 1)) 1 1)
    e=$(($(to_days $((ty + 2)) 1 1) - 1))
    ;;
year-to-date)
    s=$(to_days "$ty" 1 1)
    e=$t
    ;;
last-*-days)
    n=$(echo "$range" | cut -d- -f2)
    is_num "$n" && [ "$n" -ge 1 ] || die "expected last-<n>-days, got '$range'"
    e=$t
    s=$((t - n + 1))
    ;;
next-*-days)
    n=$(echo "$range" | cut -d- -f2)
    is_num "$n" && [ "$n" -ge 1 ] || die "expected next-<n>-days, got '$range'"
    s=$t
    e=$((t + n - 1))
    ;;
last-*-months)
    n=$(echo "$range" | cut -d- -f2)
    is_num "$n" && [ "$n" -ge 1 ] || die "expected last-<n>-months, got '$range'"
    s=$(month_start "$ty" "$tm" $((-n)))
    e=$(($(month_start "$ty" "$tm" 0) - 1))
    ;;
[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9])
    y=$(undecimal "${range%%-*}")
    m=$(undecimal "$(echo "$range" | cut -d- -f2)")
    d=$(undecimal "${range##*-}")
    s=$(to_days "$y" "$m" "$d")
    e=$s
    ;;
[0-9][0-9][0-9][0-9]-[0-9][0-9])
    y=$(undecimal "${range%%-*}")
    m=$(undecimal "${range##*-}")
    s=$(month_start "$y" "$m" 0)
    e=$(($(month_start "$y" "$m" 1) - 1))
    ;;
[0-9][0-9][0-9][0-9])
    y=$(undecimal "$range")
    s=$(to_days "$y" 1 1)
    e=$(($(to_days $((y + 1)) 1 1) - 1))
    ;;
*)
    usage >&2
    die "unknown range: $range"
    ;;
esac

echo "range=$range"
echo "start=$(from_days "$s")"
echo "end=$(from_days "$e")"
echo "days=$((e - s + 1))"
echo "today=$today"
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn workspace() -> Workspace {
        Workspace {
            worktree: "/repo/.agency/worktrees/fix-login-a3k2".into(),
            repo_root: "/repo".into(),
            issue_key: "AGE".into(),
            branch: "agent/fix-login-a3k2".into(),
            setup_command: None,
            run_scripts: Vec::new(),
            loop_check: None,
            port: None,
        }
    }

    fn skills_dir(ws: &Workspace) -> std::path::PathBuf {
        ws.worktree.join(".claude").join("skills")
    }

    fn skill(ws: &Workspace, name: &str) -> String {
        let skills = kit(ws, &skills_dir(ws));
        let s = skills.iter().find(|s| s.name == name).expect("skill in kit");
        s.files.iter().find(|f| f.name == SKILL_FILE).expect("SKILL.md").contents.clone()
    }

    /// Run the emitted resolver under a real `sh`, as the agent would, and
    /// return its `key=value` output.
    fn resolve(args: &[&str]) -> (BTreeMap<String, String>, std::process::Output) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RESOLVER_FILE);
        std::fs::write(&path, RESOLVER_SH).unwrap();
        let out = std::process::Command::new("sh")
            .arg(&path)
            .args(args)
            .output()
            .expect("sh runs the resolver");
        let map = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        (map, out)
    }

    fn range(spec: &str, today: &str) -> (String, String) {
        let (map, out) = resolve(&[spec, "--today", today]);
        assert!(out.status.success(), "{spec} as of {today}: {out:?}");
        (map["start"].clone(), map["end"].clone())
    }

    /// The whole reason the kit ships a script instead of a paragraph of
    /// instructions: month lengths, leap days, quarter edges and the year
    /// rollover, all of which a model gets quietly wrong.
    #[test]
    fn the_resolver_is_right_about_the_calendar() {
        // 2026-08-18 is a Tuesday, so a Monday-start week began the 17th.
        let cases: &[(&str, &str, &str, &str)] = &[
            ("today", "2026-08-18", "2026-08-18", "2026-08-18"),
            ("yesterday", "2026-01-01", "2025-12-31", "2025-12-31"),
            ("this-week", "2026-08-18", "2026-08-17", "2026-08-23"),
            ("last-week", "2026-08-18", "2026-08-10", "2026-08-16"),
            ("week-to-date", "2026-08-18", "2026-08-17", "2026-08-18"),
            // February in a leap year, from the month after it.
            ("last-month", "2024-03-05", "2024-02-01", "2024-02-29"),
            ("this-month", "2024-02-10", "2024-02-01", "2024-02-29"),
            ("month-to-date", "2026-08-09", "2026-08-01", "2026-08-09"),
            // Across the year boundary in both directions.
            ("last-month", "2026-01-15", "2025-12-01", "2025-12-31"),
            ("last-quarter", "2026-02-02", "2025-10-01", "2025-12-31"),
            ("this-quarter", "2026-08-18", "2026-07-01", "2026-09-30"),
            ("quarter-to-date", "2026-08-18", "2026-07-01", "2026-08-18"),
            ("this-year", "2026-08-18", "2026-01-01", "2026-12-31"),
            ("last-year", "2026-08-18", "2025-01-01", "2025-12-31"),
            ("year-to-date", "2026-08-18", "2026-01-01", "2026-08-18"),
            // Inclusive of today, which is the reading "the past 30 days" has.
            ("last-30-days", "2026-08-18", "2026-07-20", "2026-08-18"),
            ("next-3-days", "2026-08-18", "2026-08-18", "2026-08-20"),
            // Whole calendar months ending with last month: never a partial
            // current one, so a monthly report can't double-count.
            ("last-3-months", "2026-08-18", "2026-05-01", "2026-07-31"),
            // Literal dates expand to the day, the month and the year.
            ("2026-08-09", "2026-08-18", "2026-08-09", "2026-08-09"),
            ("2024-02", "2026-08-18", "2024-02-01", "2024-02-29"),
            ("2000", "2026-08-18", "2000-01-01", "2000-12-31"),
        ];
        for (spec, today, start, end) in cases {
            assert_eq!(
                range(spec, today),
                (start.to_string(), end.to_string()),
                "{spec} @ {today}"
            );
        }
    }

    #[test]
    fn the_resolver_counts_the_days_it_returned() {
        let (map, _) = resolve(&["last-3-months", "--today", "2026-08-18"]);
        assert_eq!(map["days"], "92");
        assert_eq!(map["range"], "last-3-months");
        assert_eq!(map["today"], "2026-08-18");
    }

    #[test]
    fn a_sunday_week_is_available_but_not_the_default() {
        assert_eq!(range("this-week", "2026-08-18").0, "2026-08-17");
        let (map, out) =
            resolve(&["this-week", "--today", "2026-08-18", "--week-starts", "sunday"]);
        assert!(out.status.success());
        assert_eq!(map["start"], "2026-08-16");
    }

    /// A range the script doesn't know has to fail loudly. Printing a
    /// plausible-looking date for an unparsed input is the one outcome worse
    /// than the model doing the arithmetic itself.
    #[test]
    fn an_unknown_range_fails_instead_of_guessing() {
        for bad in ["sometime-last-spring", "last-0-days", "last--days"] {
            let (map, out) = resolve(&[bad, "--today", "2026-08-18"]);
            assert_eq!(out.status.code(), Some(2), "{bad} was accepted");
            assert!(map.get("start").is_none(), "{bad} printed a date anyway");
        }
    }

    #[test]
    fn only_agents_with_a_known_convention_get_a_kit() {
        assert!(agent_supported("claude"));
        for other in ["codex", "cursor", "opencode", "copilot", "shell", ""] {
            assert!(!agent_supported(other), "{other} claims a skills convention");
        }
    }

    /// The exclude pattern has to cover the path the kit is actually written
    /// to, or the whole kit shows up in the run's diff.
    #[test]
    fn the_exclude_pattern_covers_what_is_written() {
        let ws = workspace();
        let prefix = EXCLUDE_PATTERN.trim_start_matches('/').trim_end_matches('*');
        for skill in kit(&ws, &skills_dir(&ws)) {
            assert!(
                format!(".claude/skills/{}", skill.name).starts_with(prefix),
                "{} is not covered by {EXCLUDE_PATTERN}",
                skill.name
            );
            assert!(skill.name.starts_with("agency-"), "{} is not namespaced", skill.name);
        }
    }

    /// The date skill is useless if the agent can't find the script: its cwd
    /// is not guaranteed to be the worktree root.
    #[test]
    fn the_date_skill_points_at_the_script_by_absolute_path() {
        let ws = workspace();
        let text = skill(&ws, DATE_SKILL);
        let script = ws.worktree.join(".claude/skills").join(DATE_SKILL).join(RESOLVER_FILE);
        assert!(text.contains(&script.display().to_string()), "{text}");
        assert!(text.starts_with(&format!("---\nname: {DATE_SKILL}\n")), "{text}");
    }

    #[test]
    fn the_catalog_names_the_branch_the_tracker_and_the_checkout() {
        let ws = workspace();
        let text = skill(&ws, WORKSPACE_SKILL);
        assert!(text.contains("agent/fix-login-a3k2"), "{text}");
        // Absolute, for the same reason the tracker briefing's paths are: from
        // inside a worktree, a relative `.agency/issues` is a dead path.
        assert!(text.contains("/repo/.agency/issues"), "{text}");
        assert!(text.contains("/repo/.agency/issues/README.md"), "{text}");
        assert!(text.contains("`AGE-14.md`") || text.contains("AGE-14.md"), "{text}");
        assert!(text.contains("/repo/.agency/worktrees/fix-login-a3k2"), "{text}");
        // The generated files it must not commit, named exactly.
        assert!(text.contains(".claude/skills/agency-*"), "{text}");
    }

    #[test]
    fn the_catalog_says_what_is_configured_and_not_what_is_not() {
        let plain = skill(&workspace(), WORKSPACE_SKILL);
        assert!(plain.contains("no setup command"), "{plain}");
        assert!(plain.contains("no run scripts"), "{plain}");
        assert!(!plain.contains("AGENCY_PORT"), "{plain}");
        assert!(!plain.contains("This run is a loop"), "{plain}");

        let ws = Workspace {
            setup_command: Some("pnpm install".into()),
            run_scripts: vec![("dev".into(), "pnpm dev".into())],
            port: Some(3400),
            ..workspace()
        };
        let full = skill(&ws, WORKSPACE_SKILL);
        assert!(full.contains("`pnpm install`"), "{full}");
        assert!(full.contains("- dev: `pnpm dev`"), "{full}");
        assert!(full.contains("3400") && full.contains("AGENCY_PORT"), "{full}");
    }

    /// A looping run finishes on its check command, so the agent is told what
    /// that command is and that running it is the whole test of being done.
    #[test]
    fn the_catalog_names_the_check_command_of_a_loop() {
        let ws = Workspace { loop_check: Some(("cargo test".into(), 5)), ..workspace() };
        let text = skill(&ws, WORKSPACE_SKILL);
        assert!(text.contains("This run is a loop"), "{text}");
        assert!(text.contains("`cargo test`"), "{text}");
        assert!(text.contains("5 attempts"), "{text}");
    }

    /// House rule, and this copy is read by an agent on every run: no em
    /// dashes in generated user-facing text, and no emoji.
    #[test]
    fn the_generated_copy_follows_the_house_style() {
        let ws = Workspace {
            setup_command: Some("make".into()),
            run_scripts: vec![("dev".into(), "make dev".into())],
            loop_check: Some(("make check".into(), 3)),
            port: Some(3400),
            ..workspace()
        };
        for skill in kit(&ws, &skills_dir(&ws)) {
            for file in &skill.files {
                assert!(
                    !file.contents.contains('\u{2014}'),
                    "em dash in {}/{}",
                    skill.name,
                    file.name
                );
                assert!(file.contents.is_ascii(), "non-ascii in {}/{}", skill.name, file.name);
            }
        }
    }
}
