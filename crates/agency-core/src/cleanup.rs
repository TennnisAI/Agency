//! What tearing a run down actually removes, decided from the repository
//! rather than from the verb.
//!
//! Archive and delete used to differ by a guess about the future — archive
//! kept the branch in case you wanted it back, delete threw it away and warned
//! that this could not be undone. Both halves of that were wrong often enough
//! to matter. A branch whose commits are already on the base, or already on a
//! remote, holds nothing that deleting it could lose, so keeping it after a
//! merge is hoarding and warning about it is crying wolf. A branch that is
//! ahead of both holds exactly `commits_ahead` commits that exist nowhere
//! else, and that is worth a red button whichever verb the user reached for.
//!
//! So the plan is computed once, here, from facts git can answer, and both the
//! teardown in `AppState` and the words in the dialog read the same plan. Pure:
//! facts in, decision out, no I/O and no clock.

use serde::{Deserialize, Serialize};

/// What a teardown was asked to do. The user still picks between two verbs;
/// what each one removes is this module's answer, not theirs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Disposal {
    /// Put the run away: the workspace goes, the record stays, and the branch
    /// survives only if it is carrying work nothing else has.
    Archive,
    /// Remove the run outright: workspace, branch and record.
    Delete,
}

/// Where a run's commits live besides its own local branch, as git sees it at
/// the moment the user is deciding. Every field is answerable locally; nothing
/// here needs the network, because a teardown dialog that waits on `gh` is a
/// teardown dialog that hangs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BranchFacts {
    /// This run owns an isolated worktree on its own branch. False for a run
    /// working in the project's checkout and for every terminal: there is no
    /// branch of ours to weigh and nothing of ours on disk to remove.
    pub owns_branch: bool,
    /// Commits on the branch that the base does not have. Meaningless unless
    /// [`commits_known`](Self::commits_known) is set.
    pub commits_ahead: usize,
    /// Whether `base..branch` resolved at all. A base branch that has been
    /// renamed or deleted makes the count come back zero, which would read as
    /// "this branch carries nothing" and get the branch deleted. Nothing is
    /// treated as safe unless it was proved safe, so an unresolvable range
    /// keeps the branch.
    pub commits_known: bool,
    /// The branch tip is contained in the base: the work landed.
    pub merged: bool,
    /// The branch tip is contained in some remote branch, so the commits
    /// survive the local branch going away. True for a pushed branch and for
    /// one whose PR was merged and fetched.
    pub pushed: bool,
    /// The branch no longer resolves — deleted or renamed outside Agency.
    /// Nothing to keep and nothing to delete; the plan says so rather than
    /// promising a branch that is not there.
    pub gone: bool,
    /// The worktree has uncommitted changes. They are the one thing here that
    /// is on no branch at all: archiving commits them (so the branch stops
    /// being a duplicate of anything and has to be kept) and deleting destroys
    /// them (so a delete is dangerous even when the branch itself has landed).
    pub dirty: bool,
    /// The branch this run's work goes to is in the repo. It is what a restore
    /// cuts the run's branch afresh from once the archive has let that branch
    /// go, so it is what makes the ordinary post-merge archive reversible.
    /// Default-deny like the rest: unreadable means not asserted.
    pub base_exists: bool,
}

/// What a teardown will do, and what it costs. Rendered into words by the UI
/// and executed by `AppState`; neither of them decides any of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlan {
    /// The worktree checkout is unlinked and its directory removed.
    pub removes_worktree: bool,
    /// The local branch is deleted.
    pub deletes_branch: bool,
    /// The local branch stays, because it is the only copy of work.
    pub keeps_branch: bool,
    /// The run's record survives, readable from Archived.
    pub keeps_record: bool,
    /// The run can be brought back with a fresh worktree afterwards.
    pub restorable: bool,
    /// Commits that exist nowhere but the branch this is about to delete.
    /// Zero for everything else, including every merged run.
    pub commits_at_risk: usize,
    /// Uncommitted work in the worktree that this teardown destroys. Only ever
    /// true for a delete: archive commits it to the branch first.
    pub loses_uncommitted: bool,
    /// Where the branch's work already is, for the sentence that explains why
    /// deleting it is safe. `None` when it is nowhere else.
    pub safe_because: Option<SafeBecause>,
}

/// Why deleting a branch loses nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SafeBecause {
    /// Every commit is on the base branch.
    Merged,
    /// Every commit is on a remote.
    Pushed,
    /// The branch carries no commits of its own at all.
    Empty,
}

impl CleanupPlan {
    /// Whether this teardown can lose work, and so whether it is the red
    /// button. Deliberately not "is this a delete": a delete after a merge
    /// risks nothing, and dressing it in red is how a warning stops being read.
    pub fn danger(&self) -> bool {
        self.commits_at_risk > 0 || self.loses_uncommitted
    }
}

/// The plan for one run.
///
/// Archive keeps the branch only when the branch is the last copy of something.
/// Delete always takes it, and reports what that costs instead of refusing.
pub fn plan(facts: &BranchFacts, disposal: Disposal) -> CleanupPlan {
    // A run without a branch of its own owns nothing on disk and nothing in
    // git: its changes, if any, are uncommitted in the user's own checkout,
    // where nothing may be thrown away on its behalf. Both verbs reduce to
    // dropping (or keeping) the record.
    if !facts.owns_branch {
        return CleanupPlan {
            removes_worktree: false,
            deletes_branch: false,
            keeps_branch: false,
            keeps_record: disposal == Disposal::Archive,
            restorable: disposal == Disposal::Archive,
            commits_at_risk: 0,
            // The changes are in the user's own checkout, where neither verb
            // touches them, so nothing here is at risk however dirty it is.
            loses_uncommitted: false,
            safe_because: None,
        };
    }
    // Uncommitted work is on no branch and no remote. Archiving commits it
    // (see `archive_run_with_progress`), which makes the branch the only copy
    // of something and so not safe to delete; deleting destroys it outright.
    // Either way nothing about this run is redundant while it is dirty.
    let safe_because = if facts.dirty {
        None
    } else if facts.merged {
        Some(SafeBecause::Merged)
    } else if facts.pushed {
        Some(SafeBecause::Pushed)
    } else if facts.commits_known && facts.commits_ahead == 0 {
        Some(SafeBecause::Empty)
    } else {
        None
    };
    // A branch that is already gone cannot be deleted or kept; saying either
    // would be a promise about a branch that is not there.
    let branch_matters = !facts.gone;
    let redundant = safe_because.is_some();
    let deletes_branch = branch_matters && (disposal == Disposal::Delete || redundant);
    let keeps_branch = branch_matters && !deletes_branch;
    CleanupPlan {
        removes_worktree: true,
        deletes_branch,
        keeps_branch,
        keeps_record: disposal == Disposal::Archive,
        // Restoring cuts a worktree from the kept branch — or, once the archive
        // has let that branch go, from the base the work landed on, cutting the
        // branch again there. This used to require `keeps_branch`, which made
        // the ordinary ending (merged, branch dropped) the one that could not
        // be restored, so Restore was disabled on nearly every archived run.
        restorable: disposal == Disposal::Archive && (keeps_branch || facts.base_exists),
        commits_at_risk: if deletes_branch && !redundant { facts.commits_ahead } else { 0 },
        loses_uncommitted: facts.dirty && disposal == Disposal::Delete,
        safe_because,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(commits_ahead: usize) -> BranchFacts {
        BranchFacts {
            owns_branch: true,
            commits_ahead,
            commits_known: true,
            base_exists: true,
            ..BranchFacts::default()
        }
    }

    #[test]
    fn archiving_a_merged_run_takes_the_branch_with_it() {
        // The whole point of the rework: after a merge the branch is a copy of
        // commits that are on the base, so keeping it is hoarding.
        let p = plan(&BranchFacts { merged: true, ..agent(3) }, Disposal::Archive);
        assert!(p.removes_worktree && p.deletes_branch);
        assert!(!p.keeps_branch);
        assert!(p.keeps_record);
        assert!(p.restorable, "the base is the start point once the branch is gone");
        assert_eq!(p.commits_at_risk, 0);
        assert_eq!(p.safe_because, Some(SafeBecause::Merged));
        assert!(!p.danger());
    }

    #[test]
    fn an_archive_with_nothing_left_to_cut_from_is_not_restorable() {
        // The branch goes and the base is not in the repo either — the one
        // ending that really cannot be brought back, and the only one whose
        // Restore button is disabled.
        let facts = BranchFacts { merged: true, base_exists: false, ..agent(3) };
        let p = plan(&facts, Disposal::Archive);
        assert!(p.deletes_branch && !p.keeps_branch);
        assert!(!p.restorable);
    }

    #[test]
    fn archiving_unmerged_work_keeps_the_branch() {
        let p = plan(&agent(3), Disposal::Archive);
        assert!(p.removes_worktree && p.keeps_branch && !p.deletes_branch);
        assert!(p.restorable);
        assert_eq!(p.commits_at_risk, 0, "nothing is deleted, so nothing is at risk");
        assert_eq!(p.safe_because, None);
    }

    #[test]
    fn a_pushed_branch_counts_as_safe() {
        // A branch whose commits are on a remote — pushed for review, or its PR
        // merged and fetched — survives its local copy being deleted.
        let p = plan(&BranchFacts { pushed: true, ..agent(2) }, Disposal::Archive);
        assert!(p.deletes_branch);
        assert_eq!(p.commits_at_risk, 0);
        assert_eq!(p.safe_because, Some(SafeBecause::Pushed));
    }

    #[test]
    fn merged_wins_over_pushed_in_the_explanation() {
        let p = plan(&BranchFacts { merged: true, pushed: true, ..agent(1) }, Disposal::Archive);
        assert_eq!(p.safe_because, Some(SafeBecause::Merged));
    }

    #[test]
    fn an_empty_branch_is_safe_to_delete_either_way() {
        // An agent that committed nothing has a branch identical to its base.
        let p = plan(&agent(0), Disposal::Archive);
        assert!(p.deletes_branch);
        assert_eq!(p.safe_because, Some(SafeBecause::Empty));
        assert!(!p.danger());
    }

    #[test]
    fn a_dirty_worktree_keeps_the_branch_even_after_a_merge() {
        // Archive commits what is uncommitted, so by the time the branch could
        // be deleted it is carrying a commit that is on nothing else.
        let p = plan(&BranchFacts { merged: true, dirty: true, ..agent(3) }, Disposal::Archive);
        assert!(p.keeps_branch && !p.deletes_branch);
        assert_eq!(p.safe_because, None);
        assert!(!p.loses_uncommitted, "archive commits it rather than dropping it");
        assert!(!p.danger());
    }

    #[test]
    fn deleting_a_dirty_worktree_is_dangerous_however_the_branch_stands() {
        // The uncommitted changes are on no branch and no remote, so a merged
        // branch does not make this safe.
        let p = plan(&BranchFacts { merged: true, dirty: true, ..agent(3) }, Disposal::Delete);
        assert!(p.loses_uncommitted);
        assert!(p.danger());
    }

    #[test]
    fn deleting_unmerged_work_is_the_only_dangerous_case() {
        let p = plan(&agent(4), Disposal::Delete);
        assert!(p.deletes_branch && !p.keeps_record);
        assert_eq!(p.commits_at_risk, 4);
        assert!(p.danger());
    }

    #[test]
    fn deleting_after_a_merge_is_not_dangerous() {
        // The complaint that started this: "delete" reading as catastrophic
        // when the work is already on main.
        let p = plan(&BranchFacts { merged: true, ..agent(3) }, Disposal::Delete);
        assert_eq!(p.commits_at_risk, 0);
        assert!(!p.danger());
        assert!(!p.keeps_record);
    }

    #[test]
    fn an_unreadable_commit_range_keeps_the_branch() {
        // A base branch renamed out from under a run makes the count come back
        // zero. Deleting on that basis would throw away the only copy of the
        // work because a *different* branch moved.
        let facts = BranchFacts { commits_known: false, ..agent(0) };
        let p = plan(&facts, Disposal::Archive);
        assert!(p.keeps_branch && !p.deletes_branch);
        assert_eq!(p.safe_because, None);
    }

    #[test]
    fn a_run_in_the_main_checkout_owns_nothing_to_remove() {
        // No worktree of ours, no branch of ours: archiving is the record and
        // the stamp, and the user's own changes stay where they are.
        let p = plan(&BranchFacts::default(), Disposal::Archive);
        assert!(!p.removes_worktree && !p.deletes_branch && !p.keeps_branch);
        assert!(p.keeps_record && p.restorable);
        assert!(!p.danger());
    }

    #[test]
    fn a_terminal_delete_removes_nothing_but_the_record() {
        let p = plan(&BranchFacts::default(), Disposal::Delete);
        assert!(!p.removes_worktree && !p.deletes_branch);
        assert!(!p.keeps_record);
        assert!(!p.danger());
    }

    #[test]
    fn a_branch_deleted_outside_agency_is_neither_kept_nor_deleted() {
        let p = plan(&BranchFacts { gone: true, ..agent(2) }, Disposal::Archive);
        assert!(p.removes_worktree, "the checkout may still be on disk");
        assert!(!p.deletes_branch && !p.keeps_branch);
        // Still restorable, onto the base: whatever was on the vanished branch
        // was lost by whoever deleted it, not by archiving, and the worktree
        // and the conversation do come back.
        assert!(p.restorable);
        assert_eq!(p.commits_at_risk, 0);
    }
}
