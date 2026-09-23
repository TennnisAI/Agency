//! When to take a workspace checkpoint (AGE-140), and the thread that takes it.
//!
//! The git half is `agency_core::checkpoint`. The hard part is the trigger: we
//! read a terminal, not a structured protocol, so there is no crisp "turn
//! started" or "turn ended" message to hang a snapshot on. The nearest signals
//! are ones Agency already has and already tests:
//!
//! - **A turn starts** when the user presses Enter in an agent's pane
//!   (`sendq::classify_input` says `Submitted`) or the send queue types
//!   something in for them. A new run gets one at creation, taken on the
//!   spot rather than queued, before its agent or setup script starts: the
//!   state it was handed.
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
//! Captures run on one background thread: snapshotting a large worktree takes
//! long enough that it must not hold up a keystroke or the notifier tick. The
//! run-start one is the exception, since `create_run` is already off the main
//! thread and the agent must not start before it.
//!
//! Three gates guard a capture, taken in this order. The project's checkout
//! gate (`AppState::repo_gates`) comes first, for a run with no worktree: its
//! directory is the checkout a merge, a pull or a branch switch moves between
//! branches, and a snapshot taken halfway through one is half of each. Then the
//! run's own, which `retire` holds while it prunes. Then the workspace
//! directory's, which a restore holds. Several runs can share one directory
//! (every run working in the project checkout does), and a restore rewrites
//! that directory for all of them, so any of their captures reading it halfway
//! through would store a state that never existed. A restore takes the same
//! three in the same order, and the send queue checks the directory's before it
//! types into a pane (see [`Checkpointer::unless_busy`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use agency_core::checkpoint::{Checkpoint, HashCache, Kind};

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
    /// The project whose own checkout `dir` is, for a run without a worktree:
    /// the `repo_gates` key a capture has to hold. `None` for a worktree.
    pub checkout_of: Option<String>,
    pub kind: Kind,
}

/// What the capture thread is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Capture(Request),
    /// Everything queued before this for the run has been dropped, so it
    /// need not stay retired: sent by `retire` behind its prune, carrying the
    /// token that retirement was given.
    Forget(String, u64),
}

/// Collapse a burst of requests into the captures worth taking: one per run and
/// kind, in the order first asked for. A second identical request made while
/// the first was still queued would snapshot the same files moments later. A
/// `Forget` stays where it is, and a capture asked for after it is a new one.
pub fn coalesce(batch: Vec<Msg>) -> Vec<Msg> {
    let mut seen = HashSet::new();
    batch
        .into_iter()
        .filter(|m| match m {
            Msg::Capture(r) => seen.insert((r.run_id.clone(), r.kind)),
            Msg::Forget(run_id, _) => {
                seen.retain(|(id, _)| id != run_id);
                true
            }
        })
        .collect()
}

/// What the capture thread shares with the threads that restore and prune.
struct Shared {
    gates: KeyedGates,
    /// `AppState::repo_gates`, the same instance.
    repo_gates: Arc<KeyedGates>,
    /// Runs whose checkpoints have been pruned (archived, deleted, or never
    /// got going), each with the token its retirement was given. A capture
    /// still queued for one is dropped: taken after the prune, it would leave
    /// refs behind for a run that no longer exists, which is exactly the
    /// unbounded growth pruning is there to stop. Cleared by the `Forget` the
    /// retirement queues behind those captures; kept for good, the set grew
    /// with every run for the life of the app, and a run id ever issued again
    /// would have got no checkpoints at all.
    retired: Mutex<HashMap<String, u64>>,
    /// Each workspace directory's [`HashCache`], so a capture rehashes only the
    /// untracked files that changed since the last one.
    caches: Mutex<HashMap<PathBuf, HashCache>>,
}

/// The capture thread and the gates that keep a capture out of the middle of a
/// restore or a prune.
pub struct Checkpointer {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    tokens: AtomicU64,
}

impl Checkpointer {
    pub fn new(repo_gates: Arc<KeyedGates>) -> Checkpointer {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            gates: KeyedGates::new(),
            repo_gates,
            retired: Mutex::new(HashMap::new()),
            caches: Mutex::new(HashMap::new()),
        });
        let worker = shared.clone();
        let spawned =
            std::thread::Builder::new().name("checkpoints".into()).spawn(move || work(rx, &worker));
        if let Err(e) = spawned {
            // Requests then go nowhere; everything else still works.
            log::error!("checkpoints: could not start the capture thread: {e}");
        }
        Checkpointer { tx, shared, tokens: AtomicU64::new(0) }
    }

    /// Queue a capture. Never blocks on git.
    pub fn request(&self, req: Request) {
        let _ = self.tx.send(Msg::Capture(req));
    }

    /// Take a capture now, on this thread, under the same gates as the queue.
    pub fn capture_now(&self, req: &Request) -> anyhow::Result<Option<Checkpoint>> {
        take(&self.shared, req)
    }

    /// Run `f` with no capture of `run_id` or of anything in `dir` in progress,
    /// and none starting until it returns. The caller holds the checkout gate
    /// already, when there is one (`AppState::git_mutate`). `f` is handed
    /// `dir`'s hash cache.
    pub fn exclusive<T>(&self, run_id: &str, dir: &Path, f: impl FnOnce(&mut HashCache) -> T) -> T {
        self.shared.gates.with(&run_gate(run_id), || {
            self.shared.gates.with(&dir_gate(dir), || with_cache(&self.shared, dir, f))
        })
    }

    /// Run `f` with `dir`'s hash cache, for a read that needs no gate.
    pub fn reading<T>(&self, dir: &Path, f: impl FnOnce(&mut HashCache) -> T) -> T {
        with_cache(&self.shared, dir, f)
    }

    /// Run `f` unless a capture or a restore of `dir` holds it right now;
    /// `None` if one does. For the send queue: text typed into an agent's pane
    /// starts a turn, and a turn starting while a restore rewrites its files
    /// ends up with neither state.
    pub fn unless_busy<T>(&self, dir: &Path, f: impl FnOnce() -> T) -> Option<T> {
        self.shared.gates.try_with(&dir_gate(dir), f)
    }

    /// Stop taking checkpoints for `run_id` and run `prune` (which deletes the
    /// ones it has) with no capture of it in progress. Only once nothing can
    /// ask for another: the run's row is gone or archived, or never existed.
    pub fn retire<T>(&self, run_id: &str, prune: impl FnOnce() -> T) -> T {
        let token = self.tokens.fetch_add(1, Ordering::Relaxed);
        let out = self.shared.gates.with(&run_gate(run_id), || {
            self.shared.retired.lock().unwrap().insert(run_id.to_string(), token);
            prune()
        });
        let _ = self.tx.send(Msg::Forget(run_id.to_string(), token));
        out
    }

    /// Take checkpoints for `run_id` again: it was restored from the archive.
    pub fn revive(&self, run_id: &str) {
        self.shared.retired.lock().unwrap().remove(run_id);
    }
}

fn run_gate(run_id: &str) -> String {
    format!("run:{run_id}")
}

fn dir_gate(dir: &Path) -> String {
    format!("dir:{}", dir.display())
}

/// `f` with `dir`'s cache, taken out of the map for the duration so no lock is
/// held across git. Two users of one directory at once (a preview beside a
/// capture) each get a working cache; the one put back last wins, and either
/// is correct, since a hit still needs the file's stat to match.
fn with_cache<T>(shared: &Shared, dir: &Path, f: impl FnOnce(&mut HashCache) -> T) -> T {
    let mut cache = shared.caches.lock().unwrap().remove(dir).unwrap_or_default();
    let out = f(&mut cache);
    let mut caches = shared.caches.lock().unwrap();
    // A worktree archived or discarded takes its cache with it.
    caches.retain(|d, _| d.exists());
    caches.insert(dir.to_path_buf(), cache);
    out
}

/// One capture, under all three gates in the order the module notes give.
fn take(shared: &Shared, req: &Request) -> anyhow::Result<Option<Checkpoint>> {
    let gated = || {
        shared.gates.with(&run_gate(&req.run_id), || {
            // Read under the gate `retire` holds while it prunes.
            if shared.retired.lock().unwrap().contains_key(&req.run_id) {
                return Ok(None);
            }
            shared.gates.with(&dir_gate(&req.dir), || {
                with_cache(shared, &req.dir, |cache| {
                    agency_core::checkpoint::store::capture(&req.dir, &req.run_id, req.kind, cache)
                })
            })
        })
    };
    match &req.checkout_of {
        Some(project_id) => shared.repo_gates.with(project_id, gated),
        None => gated(),
    }
}

/// Take `run_id` off the retired list, unless it was revived and retired again
/// since `token` was issued: that retirement has a `Forget` of its own still to
/// come, and captures queued before it that must still be dropped.
pub fn forget(retired: &mut HashMap<String, u64>, run_id: &str, token: u64) {
    if retired.get(run_id) == Some(&token) {
        retired.remove(run_id);
    }
}

fn work(rx: Receiver<Msg>, shared: &Shared) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        for msg in coalesce(batch) {
            let req = match msg {
                Msg::Capture(req) => req,
                Msg::Forget(run_id, token) => {
                    forget(&mut shared.retired.lock().unwrap(), &run_id, token);
                    continue;
                }
            };
            // catch_unwind: a panic here would end checkpoints for the rest of
            // the app's life with nothing on screen to say so.
            let taken =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| take(shared, &req)));
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

    fn req(run: &str, kind: Kind) -> Msg {
        Msg::Capture(Request {
            run_id: run.into(),
            dir: PathBuf::from("/w"),
            checkout_of: None,
            kind,
        })
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

    #[test]
    fn a_capture_asked_for_after_a_forget_is_a_new_one() {
        let forget = Msg::Forget("a".into(), 1);
        let batch = vec![
            req("a", Kind::TurnEnded),
            forget.clone(),
            req("a", Kind::TurnEnded),
            req("b", Kind::TurnEnded),
        ];
        assert_eq!(coalesce(batch.clone()), batch);
    }

    #[test]
    fn a_stale_forget_leaves_a_later_retirement_alone() {
        let mut retired = HashMap::from([("a".to_string(), 2)]);
        forget(&mut retired, "a", 1);
        assert!(retired.contains_key("a"), "retired again since token 1");
        forget(&mut retired, "a", 2);
        assert!(retired.is_empty());
        forget(&mut retired, "b", 3);
    }
}
