//! When to take a workspace checkpoint (AGE-140), and the thread that takes it.
//!
//! The git half is `agency_core::checkpoint`. The hard part is the trigger: we
//! read a terminal, not a structured protocol, so there is no crisp "turn
//! started" or "turn ended" message to hang a snapshot on. The nearest signals
//! are ones Agency already has and already tests:
//!
//! - **A turn starts** when the user presses Enter in an agent's pane
//!   (`sendq::classify_input` says `Submitted`) or the send queue types
//!   something in for them. A new run gets one at creation, before its agent
//!   starts, as the state it was handed.
//! - **A turn ends** at `activity`'s working -> quiet edge: the pane stopped
//!   changing for `WORKING_TTL_MS`. See [`went_quiet`].
//!
//! Both are approximate, and deliberately cheap to get wrong. A long tool call
//! that sits quiet past the TTL ends a "turn" early and the next output starts
//! a new streak, which costs one extra checkpoint. Enter on a permission prompt
//! reads as a new turn, same cost. And a capture that finds nothing changed
//! stores nothing at all, so extra triggers are nearly free. The end-of-turn
//! capture is the backbone: the start-of-turn one only adds anything when the
//! human changed files between turns, and it races the agent's first write
//! (the capture runs on this thread, after the Enter has already gone through),
//! which is why it is the one allowed to be approximate.
//!
//! Captures run on one background thread: `git add -A` over a large worktree
//! takes long enough that it must not hold up a keystroke or the notifier tick.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use agency_core::checkpoint::Kind;

use crate::activity::ActivityEntry;
use crate::gates::KeyedGates;

/// Whether the activity bookkeeping just crossed from working to quiet: the
/// end of a turn, as near as a terminal lets us tell.
///
/// Read off `ActivityEntry::working`, which `activity::update` settles once per
/// tick, so an edge is seen exactly once. The very first observation of a run
/// counts as working, so an app start produces one of these per live run about
/// ten seconds in; that capture picks up whatever changed while Agency was
/// closed, and stores nothing when nothing did.
pub fn went_quiet(prev: Option<&ActivityEntry>, next: &ActivityEntry) -> bool {
    prev.is_some_and(|p| p.working) && !next.working
}

/// One capture to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub run_id: String,
    /// The run's workspace: its worktree, or the project checkout for a run
    /// without one.
    pub dir: PathBuf,
    pub kind: Kind,
}

/// Collapse a burst of requests into the captures worth taking: one per run and
/// kind, in the order first asked for. A second identical request made while
/// the first was still queued would snapshot the same files moments later.
pub fn coalesce(batch: Vec<Request>) -> Vec<Request> {
    let mut seen = HashSet::new();
    batch.into_iter().filter(|r| seen.insert((r.run_id.clone(), r.kind))).collect()
}

/// The capture thread and the per-run gate that keeps a capture out of the
/// middle of a restore.
pub struct Checkpointer {
    tx: Mutex<Sender<Request>>,
    gates: Arc<KeyedGates>,
    /// Runs whose checkpoints have been pruned (archived, deleted, or never
    /// got going). A capture still queued for one is dropped: taken after the
    /// prune, it would leave refs behind for a run that no longer exists,
    /// which is exactly the unbounded growth pruning is there to stop.
    retired: Arc<Mutex<HashSet<String>>>,
}

impl Checkpointer {
    pub fn new() -> Checkpointer {
        let (tx, rx) = channel();
        let gates = Arc::new(KeyedGates::new());
        let retired = Arc::new(Mutex::new(HashSet::new()));
        let (worker_gates, worker_retired) = (gates.clone(), retired.clone());
        let spawned = std::thread::Builder::new()
            .name("checkpoints".into())
            .spawn(move || work(rx, &worker_gates, &worker_retired));
        if let Err(e) = spawned {
            // Requests then go nowhere; everything else still works.
            log::error!("checkpoints: could not start the capture thread: {e}");
        }
        Checkpointer { tx: Mutex::new(tx), gates, retired }
    }

    /// Queue a capture. Never blocks on git.
    pub fn request(&self, req: Request) {
        let _ = self.tx.lock().unwrap().send(req);
    }

    /// Run `f` with no capture of `run_id` in progress, and none starting until
    /// it returns.
    pub fn exclusive<T>(&self, run_id: &str, f: impl FnOnce() -> T) -> T {
        self.gates.with(run_id, f)
    }

    /// Stop taking checkpoints for `run_id` and run `prune` (which deletes the
    /// ones it has) with no capture of it in progress.
    pub fn retire<T>(&self, run_id: &str, prune: impl FnOnce() -> T) -> T {
        self.gates.with(run_id, || {
            self.retired.lock().unwrap().insert(run_id.to_string());
            prune()
        })
    }

    /// Take checkpoints for `run_id` again: it was restored from the archive.
    pub fn revive(&self, run_id: &str) {
        self.retired.lock().unwrap().remove(run_id);
    }
}

fn work(rx: Receiver<Request>, gates: &KeyedGates, retired: &Mutex<HashSet<String>>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        for req in coalesce(batch) {
            // catch_unwind: a panic here would end checkpoints for the rest of
            // the app's life with nothing on screen to say so.
            let taken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                gates.with(&req.run_id, || {
                    // Read under the gate `retire` holds while it prunes.
                    if retired.lock().unwrap().contains(&req.run_id) {
                        return Ok(None);
                    }
                    agency_core::checkpoint::store::capture(&req.dir, &req.run_id, req.kind)
                })
            }));
            match taken {
                Ok(Ok(Some(cp))) => {
                    log::debug!("checkpoints: {} #{} ({:?})", req.run_id, cp.seq, req.kind)
                }
                Ok(Ok(None)) => {}
                // A worktree removed a moment ago, a repository mid-rebase: the
                // next turn tries again, and nothing the user did is lost.
                Ok(Err(e)) => log::warn!("checkpoints: capturing {} failed: {e:#}", req.run_id),
                Err(_) => log::error!("checkpoints: capturing {} panicked", req.run_id),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{update, WORKING_TTL_MS};

    fn req(run: &str, kind: Kind) -> Request {
        Request { run_id: run.into(), dir: PathBuf::from("/w"), kind }
    }

    #[test]
    fn a_working_run_going_quiet_is_the_edge() {
        let mut e = update(None, true, 0);
        let prev = e;
        e = update(Some(e), false, WORKING_TTL_MS);
        assert!(went_quiet(Some(&prev), &e));
    }

    #[test]
    fn the_edge_fires_once() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, WORKING_TTL_MS);
        let prev = e;
        e = update(Some(e), false, WORKING_TTL_MS + 2_000);
        assert!(!went_quiet(Some(&prev), &e), "still quiet is not a new edge");
    }

    #[test]
    fn a_lull_under_the_ttl_is_not_the_end_of_a_turn() {
        let mut e = update(None, true, 0);
        let prev = e;
        e = update(Some(e), false, WORKING_TTL_MS - 1);
        assert!(!went_quiet(Some(&prev), &e));
    }

    #[test]
    fn nothing_before_the_first_observation() {
        let e = update(None, true, 0);
        assert!(!went_quiet(None, &e));
    }

    #[test]
    fn going_busy_is_not_the_edge() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, WORKING_TTL_MS);
        let prev = e;
        e = update(Some(e), true, 60_000);
        assert!(!went_quiet(Some(&prev), &e));
    }

    #[test]
    fn coalesce_keeps_one_per_run_and_kind_in_order() {
        let batch = vec![
            req("a", Kind::PromptSent),
            req("b", Kind::TurnEnded),
            req("a", Kind::PromptSent),
            req("a", Kind::TurnEnded),
            req("b", Kind::TurnEnded),
        ];
        assert_eq!(
            coalesce(batch),
            vec![req("a", Kind::PromptSent), req("b", Kind::TurnEnded), req("a", Kind::TurnEnded)]
        );
    }
}
