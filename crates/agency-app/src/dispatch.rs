//! The run tools' decisions (AGE-209): who may dispatch, whose runs an agent
//! may wait on and read, and what a child's state means to the agent waiting
//! on it. Pure, like `activity.rs` beside it; `AppState::run_tool` does the
//! reading and the starting.
//!
//! The rules here are the issue's hard constraints, made checkable:
//!
//! - **No escalation by proxy.** A child runs the agent profile and model of
//!   the session that dispatched it, read from the registry exactly as that
//!   session was launched ([`caller_profile`]), so its permission gate is the
//!   one the user chose for the caller. No argument names another profile:
//!   comparing two profiles' permissions is not something free-form CLI flags
//!   allow (dropping a restrictive `--permission-mode plan` is fewer arguments
//!   and a wider gate), so the only comparison that cannot be got wrong is
//!   "the same one".
//! - **The parent cannot answer for the user.** There is no tool to type into
//!   a child. A child stopped on a permission prompt is `blocked` on the
//!   user's board with its own badge and notification, and `run_wait` tells
//!   the parent exactly that; it cannot approve anything.
//! - **Never more than the parent had.** A dispatched run is never offered
//!   the run tools itself, so a tree is one level deep and its width is
//!   [`MAX_LIVE_CHILDREN`].
//! - **Default-deny on reach.** An agent waits on and reads only the runs it
//!   dispatched. Every other run in the project is another agent's work, and
//!   another user decision.

use crate::activity::{ActivityInfo, ActivityState};
use agency_core::registry::RunSession;
use agency_core::term::SessionStatus;

/// How many runs one agent may have dispatched and not yet seen archived or
/// discarded. Each is a live agent with the user's credentials and its own
/// port block, and every one of them is a card the user has to review, so the
/// cap is about the user's attention as much as the machine.
pub const MAX_LIVE_CHILDREN: usize = 4;

/// Whether `parent` may dispatch another run now. `is_child` is whether
/// `parent` was itself dispatched; `live_children` how many of its children
/// are still on the board. `Err` is the refusal, addressed to the agent.
pub fn admit(is_child: bool, live_children: usize) -> Result<(), String> {
    if is_child {
        return Err("This run was itself started by run_dispatch, and a dispatched run cannot \
                    dispatch more: do the work in this run instead."
            .to_string());
    }
    if live_children >= MAX_LIVE_CHILDREN {
        return Err(format!(
            "This run already has {live_children} dispatched runs on the board, which is as many \
             as one run may have at once. Wait for one with run_wait; once the user has merged, \
             archived or discarded it, its place is free."
        ));
    }
    Ok(())
}

/// The agent profile and model a run dispatched from session `caller` of run
/// `run_id` starts on: the caller's own, as that session was launched. `agent`
/// and `model` are the run's, `tabs` its extra sessions.
///
/// A run's tabs can each run a different profile, and the caller check admits
/// any of them. The child used to start on the run's own profile whichever
/// tab asked, so a tab the user had opened on a narrower one (plan mode beside
/// a lead that skips permission prompts) started children under the wider
/// gate: the escalation by proxy this module exists to rule out. A tab's model
/// follows `spawn_run_session`'s rule, the run's only when the tab runs the
/// run's own agent, since a model name means nothing to another CLI.
pub fn caller_profile<'a>(
    run_id: &str,
    agent: &'a str,
    model: Option<&'a str>,
    tabs: &'a [RunSession],
    caller: &str,
) -> Result<(&'a str, Option<&'a str>), String> {
    if caller == run_id {
        return Ok((agent, model));
    }
    match tabs.iter().find(|t| t.id == caller) {
        Some(tab) if tab.agent == crate::state::SHELL_AGENT => Err("This request came from a \
            terminal tab, not an agent, and only an agent dispatches runs."
            .to_string()),
        Some(tab) => Ok((tab.agent.as_str(), model.filter(|_| tab.agent == agent))),
        None => Err("The tab this request came from has just been closed, so there is no \
                     agent profile to start the run under."
            .to_string()),
    }
}

/// Whether the agent of run `asker` may wait on or read `run_id`, whose
/// recorded parent is `parent_of_target`.
pub fn may_reach(asker: &str, run_id: &str, parent_of_target: Option<&str>) -> Result<(), String> {
    if parent_of_target == Some(asker) {
        return Ok(());
    }
    Err(format!(
        "{run_id} is not a run this run dispatched. run_wait and run_read reach only the runs \
         this run started with run_dispatch."
    ))
}

/// Why a `run_wait` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The agent said its turn is over.
    Done,
    /// Stopped on a permission prompt or a question only the user can answer.
    Blocked,
    /// Quiet, and the agent does not report its state, so this is the pane's
    /// guess: a finished turn or a prompt look the same from here.
    Quiet,
    /// Nothing in flight: the session was cleared, or a guess went stale.
    Idle,
    /// The agent's process ended.
    Exited(i32),
    /// The run has no session any more.
    Gone,
    /// The user archived the run, which they do on merge too.
    Archived,
    /// The user discarded the run: its row, worktree and branch are gone.
    Discarded,
    /// The timeout passed first.
    StillWorking,
}

/// What one look at the child says, or `None` to keep waiting. `status` is the
/// daemon's word on the child's lead session, `activity` the board's.
///
/// `None` activity is a run the notifier has not looked at yet, which right
/// after a dispatch is every run: that is "not yet", never "idle".
pub fn verdict(status: &SessionStatus, activity: Option<ActivityInfo>) -> Option<Outcome> {
    match status {
        SessionStatus::Exited { code } => return Some(Outcome::Exited(*code)),
        SessionStatus::Gone => return Some(Outcome::Gone),
        SessionStatus::Running => {}
    }
    let info = activity?;
    match info.state {
        ActivityState::Working => None,
        ActivityState::Blocked => Some(Outcome::Blocked),
        ActivityState::Done if info.reported => Some(Outcome::Done),
        ActivityState::Done => Some(Outcome::Quiet),
        ActivityState::Idle => Some(Outcome::Idle),
    }
}

/// Whether the answer to `outcome` should carry the bottom of the child's
/// screen: the outcomes where what is on it is the next thing to know.
pub fn shows_screen(outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Blocked | Outcome::Quiet | Outcome::Exited(_))
}

/// The answer to a `run_wait`. `screen` is the child's last lines when
/// [`shows_screen`] asked for them.
pub fn wait_text(run_id: &str, outcome: Outcome, waited_secs: u64, screen: Option<&str>) -> String {
    let mut out = match outcome {
        Outcome::Done => format!(
            "{run_id} is done: its agent finished its turn. Read what it did with run_read \
             (transcript, or diff for the changes). It stays on the user's board for them to \
             review and merge."
        ),
        Outcome::Blocked => format!(
            "{run_id} is blocked: its agent has stopped on a permission prompt or a question \
             that only the user can answer, and Agency has flagged it on their board. You \
             cannot answer it. Tell the user it is waiting on them, or wait on it again \
             later."
        ),
        Outcome::Quiet => format!(
            "{run_id} has gone quiet. Its agent does not report its own state, so this is a \
             guess from its terminal: it may have finished, or it may be waiting on a prompt. \
             Its screen is below."
        ),
        Outcome::Idle => format!(
            "{run_id} is idle: its agent has nothing in flight. Read its transcript to see \
             where it stopped."
        ),
        Outcome::Exited(code) => format!(
            "{run_id} has exited: its agent's process ended with code {code}. Its worktree and \
             branch are still there, and so is anything it committed."
        ),
        Outcome::Gone => format!(
            "{run_id} has no session any more: its agent is not running. Its worktree and \
             branch are still there."
        ),
        Outcome::Archived => format!(
            "{run_id} has been archived by the user, which is also what a merge does. There is \
             nothing more to wait for."
        ),
        Outcome::Discarded => format!(
            "{run_id} has been discarded by the user: its worktree and branch are gone, and \
             there is nothing more to wait for or read."
        ),
        Outcome::StillWorking => format!(
            "{run_id} is still working after {waited_secs}s. Call run_wait again to keep \
             waiting."
        ),
    };
    if let Some(screen) = screen.map(str::trim_end).filter(|s| !s.trim().is_empty()) {
        out.push_str("\n\nIts screen now:\n");
        out.push_str(screen);
    }
    out
}

/// The answer to a `run_dispatch` that started a run.
pub fn dispatched_text(run_id: &str, title: &str, branch: &str, worktree: &str) -> String {
    format!(
        "Started {run_id} (\"{title}\") on branch {branch}, in its own worktree at {worktree}. \
         It is on the user's board, running your own agent with the same permission settings. \
         Call run_wait with run_id \"{run_id}\" to block until it finishes or needs the user, \
         and run_read to see what it did; its files are at that path too."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(state: ActivityState, reported: bool) -> Option<ActivityInfo> {
        Some(ActivityInfo { state, since: 0, reported })
    }

    #[test]
    fn a_dispatched_run_cannot_dispatch() {
        assert!(admit(false, 0).is_ok());
        let no = admit(true, 0).unwrap_err();
        assert!(no.contains("cannot dispatch"), "{no}");
    }

    #[test]
    fn a_parent_has_at_most_the_cap_on_the_board() {
        assert!(admit(false, MAX_LIVE_CHILDREN - 1).is_ok());
        let no = admit(false, MAX_LIVE_CHILDREN).unwrap_err();
        assert!(no.contains("run_wait"), "the refusal says how to free a place: {no}");
    }

    fn tab(id: &str, agent: &str) -> RunSession {
        RunSession { id: id.into(), run_id: "r".into(), agent: agent.into(), created_at: 0 }
    }

    #[test]
    fn a_child_runs_the_callers_own_profile_not_the_runs() {
        let tabs = [tab("r--2", "claude-plan"), tab("r--3", "claude"), tab("r--4", "codex")];
        let of = |caller| caller_profile("r", "claude", Some("opus"), &tabs, caller);
        assert_eq!(of("r"), Ok(("claude", Some("opus"))), "the run's own session");
        // A narrower profile in a tab beside a wider lead stays narrow.
        assert_eq!(of("r--2"), Ok(("claude-plan", None)));
        assert_eq!(of("r--3"), Ok(("claude", Some("opus"))), "the run's agent, its model too");
        assert_eq!(of("r--4"), Ok(("codex", None)), "\"opus\" means nothing to another CLI");
    }

    #[test]
    fn a_terminal_tab_or_a_closed_one_dispatches_nothing() {
        let tabs = [tab("r--2", crate::state::SHELL_AGENT)];
        assert!(caller_profile("r", "claude", None, &tabs, "r--2")
            .unwrap_err()
            .contains("terminal"));
        assert!(caller_profile("r", "claude", None, &tabs, "r--9").is_err());
    }

    #[test]
    fn only_a_runs_own_children_are_in_reach() {
        assert!(may_reach("p", "c", Some("p")).is_ok());
        assert!(may_reach("p", "c", Some("other")).is_err(), "a sibling's child");
        assert!(may_reach("p", "c", None).is_err(), "a run the user started");
        assert!(may_reach("p", "p", None).is_err(), "itself");
    }

    #[test]
    fn working_and_unobserved_keep_the_wait_going() {
        let running = SessionStatus::Running;
        assert_eq!(verdict(&running, None), None, "not looked at yet is not idle");
        assert_eq!(verdict(&running, info(ActivityState::Working, true)), None);
        assert_eq!(verdict(&running, info(ActivityState::Working, false)), None);
    }

    #[test]
    fn a_reported_done_is_done_and_a_guessed_one_says_it_is_a_guess() {
        let running = SessionStatus::Running;
        assert_eq!(verdict(&running, info(ActivityState::Done, true)), Some(Outcome::Done));
        assert_eq!(verdict(&running, info(ActivityState::Done, false)), Some(Outcome::Quiet));
        assert_eq!(verdict(&running, info(ActivityState::Blocked, true)), Some(Outcome::Blocked));
        assert_eq!(verdict(&running, info(ActivityState::Idle, false)), Some(Outcome::Idle));
    }

    #[test]
    fn a_dead_session_ends_the_wait_whatever_the_board_last_said() {
        let working = info(ActivityState::Working, true);
        assert_eq!(verdict(&SessionStatus::Exited { code: 2 }, working), Some(Outcome::Exited(2)));
        assert_eq!(verdict(&SessionStatus::Gone, working), Some(Outcome::Gone));
    }

    #[test]
    fn a_blocked_answer_says_the_parent_cannot_answer_and_shows_the_screen() {
        assert!(shows_screen(Outcome::Blocked));
        assert!(!shows_screen(Outcome::Done));
        let text = wait_text("r1", Outcome::Blocked, 3, Some("Allow `rm -rf build`?\n  1. Yes\n"));
        assert!(text.contains("You cannot answer it"), "{text}");
        assert!(text.ends_with("1. Yes"), "{text}");
        let text = wait_text("r1", Outcome::Done, 3, Some("   \n"));
        assert!(!text.contains("screen now"), "a blank screen adds nothing: {text}");
    }

    #[test]
    fn a_timeout_says_how_long_and_to_call_again() {
        let text = wait_text("r1", Outcome::StillWorking, 300, None);
        assert!(text.contains("300s") && text.contains("run_wait again"), "{text}");
    }
}
