//! Syncing `.agency/issues/` between checkouts of the same project.
//!
//! This module is the merge, and nothing else: a pure three-way function over
//! three sets of parsed issue files (the last synced state, what is on this
//! machine now, what the other side has) that returns a [`Plan`] of writes and
//! deletes. Git and the filesystem live in the caller. See
//! `docs/tracked-issues.md` for why the transport is a ref that is never
//! checked out rather than tracked files.
//!
//! Identity is [`IssueFile::uid`], never the key. The key is minted from a
//! per-machine counter, so two machines filing offline both produce an
//! `AGE-175` and merging on the key would silently fuse two unrelated issues
//! into one. Two uids claiming one key is therefore a conflict this reports
//! rather than resolves — renumbering one of them means rewriting every
//! `links:` that names it, which is more than a merge should do on its own.

use std::collections::{BTreeMap, BTreeSet};

use crate::issuefs::{serialize_issue_file, IssueFile};
use crate::registry::IssueComment;

/// What the caller should do to the issues directory. Ordered so a rename
/// (delete then write) is safe to apply in sequence.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Create or overwrite the file for `key`, which is `file.key`.
    Write(Box<IssueFile>),
    /// Remove the file named by this key.
    Delete(String),
}

/// Something the merge could not decide alone. Reported, never fatal: a plan
/// with conflicts is still applied, and the conflict says what was chosen.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Conflict {
    /// The issue key the conflict is about, for showing a human.
    pub key: String,
    /// `body`, `title`, `status`, or `key` — the field both sides changed.
    pub field: String,
    pub detail: String,
}

/// The outcome of a merge: what to do, and what needed a judgement call.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    /// Keys of files left out of the merge entirely, with why. A file with no
    /// `uid:` has no identity to merge on; `issuefs::reconcile` writes one in,
    /// so this clears itself on the next pass.
    pub skipped: Vec<(String, String)>,
}

impl Plan {
    /// Did this plan decide there is nothing to do? Conflicts do not count as
    /// work: they are reported every pass until someone edits one side.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

/// How a sync pass treats the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Three-way merge against the last synced state. The steady state.
    Merge,
    /// This machine's tracker becomes the shared one, wholesale.
    ///
    /// The first sync of an existing backlog needs this, and the reason is
    /// worth stating: before the first sync, each machine backfilled its own
    /// `uid` for its own copy of the same issue, so a merge would see every
    /// issue as two issues racing for one key. Seeding from one side is the
    /// only thing that makes those agree.
    Publish,
    /// The shared tracker replaces this machine's, wholesale. The other half
    /// of seeding, for a machine joining an already-synced project.
    Adopt,
}

/// Merge three sets of issue files into a plan of changes to `local`.
///
/// `base` is the last state both sides agree they saw (empty on a first sync),
/// `local` is this machine's issues directory, `remote` is the other side's.
/// Files carrying no `uid` are skipped rather than guessed at.
pub fn plan(mode: Mode, base: &[IssueFile], local: &[IssueFile], remote: &[IssueFile]) -> Plan {
    let mut out = Plan::default();
    let base = index(base, &mut out, false);
    let local = index(local, &mut out, true);
    let remote = index(remote, &mut out, true);

    match mode {
        Mode::Publish => return out,
        Mode::Adopt => {
            for (uid, r) in &remote {
                if local.get(uid) != Some(r) {
                    out.actions.push(Action::Write(Box::new((*r).clone())));
                }
            }
            for (uid, l) in &local {
                if !remote.contains_key(uid) {
                    out.actions.push(Action::Delete(l.key.clone()));
                }
            }
            return out;
        }
        Mode::Merge => {}
    }

    // Two uids claiming one key cannot both be written: the key *is* the
    // filename. Neither side is wrong, so neither is touched, and the pair is
    // reported for a human (or a later renumbering pass) to break.
    let contested = contested_keys(&local, &remote);

    for uid in local.keys().chain(remote.keys()).cloned().collect::<BTreeSet<_>>() {
        let (l, r, b) = (local.get(&uid), remote.get(&uid), base.get(&uid));
        let key = l.or(r).map(|f| f.key.clone()).unwrap_or_default();
        if contested.contains(&key) {
            continue;
        }
        match (l, r) {
            // Present on the remote but gone here. In the base too, so this
            // side deleted it deliberately; leaving it absent is what makes the
            // deletion survive the next push instead of being resurrected.
            (None, Some(r)) => {
                if b.is_none() {
                    out.actions.push(Action::Write(Box::new((*r).clone())));
                }
            }
            // Gone on the remote. In the base, so the remote deleted it.
            (Some(l), None) => {
                if b.is_some() {
                    out.actions.push(Action::Delete(l.key.clone()));
                }
            }
            (Some(l), Some(r)) => {
                if l == r {
                    continue;
                }
                let merged = merge_issue(b, l, r, &mut out.conflicts);
                // A key that moved renames the file, and the old name has to go
                // or the issue exists twice.
                if merged.key != l.key {
                    out.actions.push(Action::Delete(l.key.clone()));
                }
                if merged != **l {
                    out.actions.push(Action::Write(Box::new(merged)));
                }
            }
            (None, None) => unreachable!("uid came from one of the two maps"),
        }
    }

    for key in contested {
        out.conflicts.push(Conflict {
            key: key.clone(),
            field: "key".into(),
            detail: format!(
                "two different issues both claim {key}; neither side was changed. \
                 Renumber one of them and sync again."
            ),
        });
    }
    out.conflicts.sort_by(|a, b| (&a.key, &a.field).cmp(&(&b.key, &b.field)));
    out
}

/// What to do about attachments: which to bring over, which to remove.
#[derive(Debug, Default, PartialEq)]
pub struct AssetPlan {
    /// Paths relative to the issues dir (`assets/AGE-14-shot.png`) to copy from
    /// the other side.
    pub fetch: Vec<String>,
    /// Paths to remove here.
    pub delete: Vec<String>,
}

/// Three-way merge of the attachment set.
///
/// Assets are identified by path and never edited in place — the app names each
/// one with the issue key and a timestamp, so a given path always holds the same
/// bytes. That makes this a pure set question (did someone add one, did someone
/// delete one) with no content merge to do, and it is why the bytes are fetched
/// only for what is actually missing rather than compared.
///
/// Orphans are deliberately not collected here. An asset whose issue was deleted
/// is still referenced by that issue's text in the ref's history, and a merge is
/// the wrong place to decide a file is unreachable.
pub fn plan_assets(mode: Mode, base: &[String], local: &[String], remote: &[String]) -> AssetPlan {
    let set = |xs: &[String]| xs.iter().cloned().collect::<BTreeSet<String>>();
    let (b, l, r) = (set(base), set(local), set(remote));
    match mode {
        // The local side is the source; nothing arrives and nothing goes.
        Mode::Publish => AssetPlan::default(),
        Mode::Adopt => AssetPlan {
            fetch: r.difference(&l).cloned().collect(),
            delete: l.difference(&r).cloned().collect(),
        },
        Mode::Merge => {
            let keep = |p: &String| {
                if b.contains(p) {
                    l.contains(p) && r.contains(p)
                } else {
                    l.contains(p) || r.contains(p)
                }
            };
            AssetPlan {
                fetch: r.iter().filter(|p| !l.contains(*p) && keep(p)).cloned().collect(),
                delete: l.iter().filter(|p| !keep(p)).cloned().collect(),
            }
        }
    }
}

/// Index files by uid, recording the ones that have none. `report` is false for
/// the base, whose uid-less files are not a problem anyone can act on.
fn index<'a>(files: &'a [IssueFile], out: &mut Plan, report: bool) -> Snapshot<'a> {
    let mut map = BTreeMap::new();
    for f in files {
        match &f.uid {
            Some(uid) => {
                map.insert(uid.clone(), f);
            }
            None => {
                if report && !out.skipped.iter().any(|(k, _)| *k == f.key) {
                    out.skipped.push((f.key.clone(), "no uid yet".into()));
                }
            }
        }
    }
    map
}

type Snapshot<'a> = BTreeMap<String, &'a IssueFile>;

/// Keys that more than one uid lays claim to across the two sides.
fn contested_keys(local: &Snapshot, remote: &Snapshot) -> BTreeSet<String> {
    let mut by_key: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (uid, f) in local.iter().chain(remote.iter()) {
        by_key.entry(f.key.as_str()).or_default().insert(uid.as_str());
    }
    by_key.into_iter().filter(|(_, uids)| uids.len() > 1).map(|(k, _)| k.to_string()).collect()
}

/// Three-way merge of one issue, field by field.
///
/// Field-level rather than whole-file last-writer-wins because the common real
/// case is two machines touching *different* fields of the same issue (a status
/// flipped on one, a due date set on the other), and a whole-file rule throws
/// one of them away for no reason.
fn merge_issue(
    base: Option<&&IssueFile>,
    local: &IssueFile,
    remote: &IssueFile,
    conflicts: &mut Vec<Conflict>,
) -> IssueFile {
    let base = base.copied();
    // Who wins a field both sides changed. Decided once for the whole issue so
    // a merged file is some consistent side's view rather than a mosaic, and
    // broken by content when the timestamps tie: `updated` has one-second
    // resolution and comes off two different clocks, so ties are not rare
    // enough to settle by whichever machine happened to sync last.
    let local_wins = (local.updated_at, serialize_issue_file(local))
        > (remote.updated_at, serialize_issue_file(remote));

    let mut note = |field: &str, detail: String| {
        conflicts.push(Conflict { key: local.key.clone(), field: field.into(), detail });
    };

    let key = pick(base.map(|b| &b.key), &local.key, &remote.key, local_wins, "key", &mut note);
    let mut merged = IssueFile {
        seq: crate::issuefs::parse_key(&key).map_or(local.seq, |(_, n)| n),
        key,
        uid: local.uid.clone().or_else(|| remote.uid.clone()),
        title: pick(
            base.map(|b| &b.title),
            &local.title,
            &remote.title,
            local_wins,
            "title",
            &mut note,
        ),
        body: pick(base.map(|b| &b.body), &local.body, &remote.body, local_wins, "body", &mut note),
        status: pick(
            base.map(|b| &b.status),
            &local.status,
            &remote.status,
            local_wins,
            "status",
            &mut note,
        ),
        priority: pick(
            base.map(|b| &b.priority),
            &local.priority,
            &remote.priority,
            local_wins,
            "priority",
            &mut note,
        ),
        due: pick(base.map(|b| &b.due), &local.due, &remote.due, local_wins, "due", &mut note),
        scheduled: pick(
            base.map(|b| &b.scheduled),
            &local.scheduled,
            &remote.scheduled,
            local_wins,
            "scheduled",
            &mut note,
        ),
        rank: pick(base.map(|b| &b.rank), &local.rank, &remote.rank, local_wins, "rank", &mut note),
        links: merge_set(base.map(|b| b.links.as_slice()), &local.links, &remote.links, |k| {
            k.clone()
        }),
        comments: merge_comments(base, local, remote, local_wins),
        // An issue was created once, however the two clocks disagree about
        // when; the earlier reading is the one closer to the truth. Zero means
        // the file didn't say, so it never wins.
        created_at: match (local.created_at, remote.created_at) {
            (0, r) => r,
            (l, 0) => l,
            (l, r) => l.min(r),
        },
        updated_at: local.updated_at.max(remote.updated_at),
        extra: merge_set(base.map(|b| b.extra.as_slice()), &local.extra, &remote.extra, |l| {
            l.split_once(':').map_or_else(|| l.clone(), |(k, _)| k.trim().to_string())
        }),
    };
    merged.links.retain(|k| *k != merged.key);
    merged
}

/// Three-way pick for one scalar. Unchanged on a side means that side yields;
/// changed on both means `local_wins` decides and the loss is reported.
fn pick<T: Clone + PartialEq>(
    base: Option<&T>,
    local: &T,
    remote: &T,
    local_wins: bool,
    field: &str,
    note: &mut impl FnMut(&str, String),
) -> T {
    if local == remote {
        return local.clone();
    }
    match base {
        Some(b) if b == local => remote.clone(),
        Some(b) if b == remote => local.clone(),
        _ => {
            note(
                field,
                format!(
                    "changed on both sides; kept the {} copy",
                    if local_wins { "local" } else { "incoming" }
                ),
            );
            if local_wins {
                local.clone()
            } else {
                remote.clone()
            }
        }
    }
}

/// Three-way merge of a set: an element present in the base survives only if
/// both sides kept it (so a deliberate removal is not undone), and an element
/// absent from the base survives if either side added it. Local order first,
/// then whatever the remote added, so a merge does not reshuffle a list the
/// user arranged.
fn merge_set<T: Clone, K: Ord>(
    base: Option<&[T]>,
    local: &[T],
    remote: &[T],
    id: impl Fn(&T) -> K,
) -> Vec<T> {
    let ids = |xs: &[T]| xs.iter().map(&id).collect::<BTreeSet<K>>();
    let (bi, li, ri) = (base.map(ids), ids(local), ids(remote));
    let keep = |k: &K| match &bi {
        Some(b) if b.contains(k) => li.contains(k) && ri.contains(k),
        _ => li.contains(k) || ri.contains(k),
    };
    let mut out: Vec<T> = Vec::new();
    let mut seen: BTreeSet<K> = BTreeSet::new();
    for x in local.iter().chain(remote.iter()) {
        let k = id(x);
        if keep(&k) && seen.insert(k) {
            out.push(x.clone());
        }
    }
    out
}

/// Merge the discussion. A comment is identified by who wrote it and when, so
/// the same comment arriving from both sides is one comment; a body that
/// differs under one identity is an edit, decided like any other field.
/// Ordered oldest first, which is the order the file stores them in.
fn merge_comments(
    base: Option<&IssueFile>,
    local: &IssueFile,
    remote: &IssueFile,
    local_wins: bool,
) -> Vec<IssueComment> {
    let id = |c: &IssueComment| (c.created_at, c.author.clone());
    let mut merged =
        merge_set(base.map(|b| b.comments.as_slice()), &local.comments, &remote.comments, id);
    let other: BTreeMap<_, _> = if local_wins { &remote.comments } else { &local.comments }
        .iter()
        .map(|c| (id(c), c))
        .collect();
    let winner: BTreeMap<_, _> = if local_wins { &local.comments } else { &remote.comments }
        .iter()
        .map(|c| (id(c), c))
        .collect();
    for c in &mut merged {
        let k = id(c);
        if let (Some(w), Some(o)) = (winner.get(&k), other.get(&k)) {
            if w.body != o.body {
                c.body = w.body.clone();
            }
        }
    }
    merged.sort_by(|a, b| (a.created_at, &a.author).cmp(&(b.created_at, &b.author)));
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::IssueStatus;

    const U1: &str = "11111111-1111-4111-8111-111111111111";
    const U2: &str = "22222222-2222-4222-8222-222222222222";

    fn issue(uid: &str, key: &str, title: &str) -> IssueFile {
        IssueFile {
            key: key.into(),
            uid: Some(uid.into()),
            seq: crate::issuefs::parse_key(key).unwrap().1,
            title: title.into(),
            body: String::new(),
            comments: vec![],
            status: IssueStatus::Todo,
            priority: 0,
            due: None,
            scheduled: None,
            rank: None,
            links: vec![],
            created_at: 1_000,
            updated_at: 1_000,
            extra: vec![],
        }
    }

    fn comment(author: &str, at: i64, body: &str) -> IssueComment {
        IssueComment { author: author.into(), created_at: at, body: body.into() }
    }

    fn written(p: &Plan) -> Vec<IssueFile> {
        p.actions
            .iter()
            .filter_map(|a| match a {
                Action::Write(f) => Some((**f).clone()),
                Action::Delete(_) => None,
            })
            .collect()
    }

    fn deleted(p: &Plan) -> Vec<String> {
        p.actions
            .iter()
            .filter_map(|a| match a {
                Action::Delete(k) => Some(k.clone()),
                Action::Write(_) => None,
            })
            .collect()
    }

    #[test]
    fn an_untouched_pair_produces_no_work() {
        let a = issue(U1, "AGE-1", "One");
        let p = plan(Mode::Merge, &[a.clone()], &[a.clone()], &[a]);
        assert!(p.is_empty() && p.conflicts.is_empty());
    }

    #[test]
    fn a_new_issue_on_each_side_lands_on_the_other() {
        let mine = issue(U1, "AGE-1", "Mine");
        let theirs = issue(U2, "AGE-2", "Theirs");
        let p = plan(Mode::Merge, &[], &[mine], &[theirs.clone()]);
        // Only the incoming one is work: ours is already on disk, and the push
        // that follows carries it.
        assert_eq!(written(&p), vec![theirs]);
        assert!(deleted(&p).is_empty());
    }

    #[test]
    fn different_fields_on_each_side_both_survive() {
        let base = issue(U1, "AGE-1", "One");
        let mut mine = base.clone();
        mine.status = IssueStatus::InProgress;
        mine.updated_at = 2_000;
        let mut theirs = base.clone();
        theirs.due = Some("2026-09-01".into());
        theirs.updated_at = 3_000;

        let p = plan(Mode::Merge, &[base], &[mine], &[theirs]);
        let m = &written(&p)[0];
        assert_eq!(m.status, IssueStatus::InProgress, "local-only change was lost");
        assert_eq!(m.due.as_deref(), Some("2026-09-01"), "incoming-only change was lost");
        assert_eq!(m.updated_at, 3_000);
        assert!(p.conflicts.is_empty(), "a clean field-level merge is not a conflict");
    }

    #[test]
    fn the_same_field_changed_on_both_sides_reports_and_takes_the_later() {
        let base = issue(U1, "AGE-1", "One");
        let mut mine = base.clone();
        mine.title = "Mine".into();
        mine.updated_at = 2_000;
        let mut theirs = base.clone();
        theirs.title = "Theirs".into();
        theirs.updated_at = 5_000;

        let p = plan(Mode::Merge, &[base], &[mine], &[theirs]);
        assert_eq!(written(&p)[0].title, "Theirs");
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.conflicts[0].field, "title");
        assert!(p.conflicts[0].detail.contains("incoming"));
    }

    /// What the issue ends up as, whichever side won: the file the plan writes,
    /// or the local file untouched when the merge agreed with what is already
    /// on disk. A plan carries only work, so "no write" is a result too.
    fn resolved(p: &Plan, local: &IssueFile) -> IssueFile {
        written(p).into_iter().next().unwrap_or_else(|| local.clone())
    }

    #[test]
    fn a_tie_on_updated_is_broken_by_content_not_by_who_synced_last() {
        let base = issue(U1, "AGE-1", "One");
        let mut mine = base.clone();
        mine.title = "Alpha".into();
        let mut theirs = base.clone();
        theirs.title = "Beta".into();
        // Same `updated` on both, which one-second resolution across two clocks
        // makes common. Both machines must reach the same answer, or they push
        // over each other forever.
        let one = plan(Mode::Merge, &[base.clone()], &[mine.clone()], &[theirs.clone()]);
        let two = plan(Mode::Merge, &[base], &[theirs.clone()], &[mine.clone()]);
        assert_eq!(resolved(&one, &mine).title, resolved(&two, &theirs).title);
    }

    #[test]
    fn a_deletion_survives_instead_of_being_resurrected() {
        let a = issue(U1, "AGE-1", "One");
        // Deleted here, still on the remote: stays deleted, no write.
        let p = plan(Mode::Merge, &[a.clone()], &[], &[a.clone()]);
        assert!(p.actions.is_empty(), "a deleted issue came back: {:?}", p.actions);
        // Deleted on the remote, still here: the deletion is applied.
        let p = plan(Mode::Merge, &[a.clone()], &[a], &[]);
        assert_eq!(deleted(&p), vec!["AGE-1"]);
    }

    #[test]
    fn comments_from_both_sides_are_kept_in_order() {
        let mut base = issue(U1, "AGE-1", "One");
        base.comments = vec![comment("Ada", 100, "first")];
        let mut mine = base.clone();
        mine.comments.push(comment("Sam", 300, "mine"));
        let mut theirs = base.clone();
        theirs.comments.push(comment("Lee", 200, "theirs"));

        let p = plan(Mode::Merge, &[base], &[mine], &[theirs]);
        let got: Vec<_> = written(&p)[0].comments.iter().map(|c| c.body.clone()).collect();
        assert_eq!(got, vec!["first", "theirs", "mine"], "a comment was lost or misordered");
    }

    #[test]
    fn a_deliberately_removed_comment_does_not_come_back() {
        let mut base = issue(U1, "AGE-1", "One");
        base.comments = vec![comment("Ada", 100, "first"), comment("Sam", 200, "second")];
        // Removed on the remote: the deletion has to travel here, which is the
        // direction that takes a write.
        let mine = base.clone();
        let mut theirs = base.clone();
        theirs.comments.remove(1);
        theirs.updated_at = 2_000;

        let p = plan(Mode::Merge, &[base.clone()], &[mine], &[theirs]);
        let got: Vec<_> = written(&p)[0].comments.iter().map(|c| c.body.clone()).collect();
        assert_eq!(got, vec!["first"]);

        // And the mirror: removed here, still present there. Nothing to write,
        // and above all no write that puts it back.
        let mut mine = base.clone();
        mine.comments.remove(1);
        mine.updated_at = 2_000;
        let p = plan(Mode::Merge, &[base.clone()], &[mine.clone()], &[base]);
        let got: Vec<_> = resolved(&p, &mine).comments.iter().map(|c| c.body.clone()).collect();
        assert_eq!(got, vec!["first"], "a deleted comment was resurrected");
    }

    #[test]
    fn links_merge_as_a_set_and_respect_removal() {
        let mut base = issue(U1, "AGE-1", "One");
        base.links = vec!["AGE-2".into(), "AGE-3".into()];
        let mut mine = base.clone();
        mine.links = vec!["AGE-2".into()]; // dropped AGE-3
        let mut theirs = base.clone();
        theirs.links = vec!["AGE-2".into(), "AGE-3".into(), "AGE-9".into()]; // added AGE-9

        let p = plan(Mode::Merge, &[base], &[mine], &[theirs]);
        assert_eq!(written(&p)[0].links, vec!["AGE-2", "AGE-9"]);
    }

    #[test]
    fn two_issues_claiming_one_key_are_reported_and_neither_is_touched() {
        // The shape a first sync makes if both machines backfilled their own
        // uid for what is really the same issue, and the shape two people
        // filing offline make for real.
        let mine = issue(U1, "AGE-5", "Mine");
        let theirs = issue(U2, "AGE-5", "Theirs");
        let p = plan(Mode::Merge, &[], &[mine], &[theirs]);
        assert!(p.actions.is_empty(), "wrote over a contested key: {:?}", p.actions);
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.conflicts[0].field, "key");
        assert!(p.conflicts[0].detail.contains("AGE-5"));
    }

    #[test]
    fn a_file_with_no_uid_is_skipped_not_guessed_at() {
        let mut orphan = issue(U1, "AGE-1", "One");
        orphan.uid = None;
        let p = plan(Mode::Merge, &[], &[orphan], &[]);
        assert!(p.actions.is_empty());
        assert_eq!(p.skipped, vec![("AGE-1".to_string(), "no uid yet".to_string())]);
    }

    #[test]
    fn publish_changes_nothing_locally_and_adopt_replaces_everything() {
        let mine = issue(U1, "AGE-1", "Mine");
        let theirs = issue(U2, "AGE-2", "Theirs");

        // Publish: this machine is the source, so there is nothing to write
        // here. The push that follows is what makes it shared.
        let p = plan(Mode::Publish, &[], &[mine.clone()], &[theirs.clone()]);
        assert!(p.actions.is_empty());

        // Adopt: the shared tracker replaces this one, including dropping an
        // issue this machine had and the shared one does not.
        let p = plan(Mode::Adopt, &[], &[mine], &[theirs.clone()]);
        assert_eq!(written(&p), vec![theirs]);
        assert_eq!(deleted(&p), vec!["AGE-1"]);
    }

    #[test]
    fn attachments_merge_as_a_set_and_only_fetch_what_is_missing() {
        let a = |s: &str| s.to_string();
        let base = vec![a("assets/one.png"), a("assets/two.png")];
        let local = vec![a("assets/one.png"), a("assets/two.png"), a("assets/mine.png")];
        // Dropped two.png, added theirs.png.
        let remote = vec![a("assets/one.png"), a("assets/theirs.png")];

        let p = plan_assets(Mode::Merge, &base, &local, &remote);
        assert_eq!(p.fetch, vec!["assets/theirs.png"], "did not bring over the new attachment");
        assert_eq!(p.delete, vec!["assets/two.png"], "a removal did not travel");
        // one.png is on both sides and is not re-fetched: the bytes at a path
        // never change, so having it is knowing it is current.
        assert!(!p.fetch.contains(&a("assets/one.png")));
        // And ours, which the remote has never seen, is left alone.
        assert!(!p.delete.contains(&a("assets/mine.png")));
    }

    #[test]
    fn seeding_moves_attachments_wholesale_in_one_direction() {
        let a = |s: &str| s.to_string();
        let local = vec![a("assets/mine.png")];
        let remote = vec![a("assets/theirs.png")];

        // Publishing makes this machine the source: nothing lands here.
        assert_eq!(plan_assets(Mode::Publish, &[], &local, &remote), AssetPlan::default());

        // Adopting replaces, attachments included, or the issues that arrive
        // have body links to files that were never written.
        let p = plan_assets(Mode::Adopt, &[], &local, &remote);
        assert_eq!(p.fetch, vec!["assets/theirs.png"]);
        assert_eq!(p.delete, vec!["assets/mine.png"]);
    }

    #[test]
    fn a_renamed_key_moves_the_file_instead_of_duplicating_it() {
        let base = issue(U1, "AGE-1", "One");
        let mut theirs = base.clone();
        theirs.key = "AGE-7".into();
        theirs.seq = 7;
        theirs.updated_at = 5_000;

        let p = plan(Mode::Merge, &[base.clone()], &[base], &[theirs]);
        assert_eq!(deleted(&p), vec!["AGE-1"], "the old filename was left behind");
        let w = &written(&p)[0];
        assert_eq!((w.key.as_str(), w.seq), ("AGE-7", 7), "seq did not follow the key");
    }

    #[test]
    fn unknown_frontmatter_keys_survive_a_merge() {
        // The passthrough exists so a newer schema's fields survive an older
        // build's write. A merge must not be the place they get dropped.
        let base = issue(U1, "AGE-1", "One");
        let mut mine = base.clone();
        mine.extra = vec!["owner: nic".into()];
        let mut theirs = base.clone();
        theirs.extra = vec!["epic: platform".into()];

        let p = plan(Mode::Merge, &[base], &[mine], &[theirs]);
        assert_eq!(written(&p)[0].extra, vec!["owner: nic", "epic: platform"]);
    }
}
