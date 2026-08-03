//! Named locks for commands that no longer run on the main thread.
//!
//! Tauri runs sync commands on the main thread, which serialized every mutating
//! command against every other one for free: a lock nobody had to name (see the
//! note at the top of `commands.rs`). Moving a command off that thread means
//! providing the mutual exclusion it was relying on, and the useful granularity
//! is a name. Two projects have no shared checkout to fight over, and two agents
//! have no shared session, so a single global lock would only reintroduce the
//! stalls the move was for.
//!
//! Gates are created on first use and then kept for the life of the app: a
//! handful of entries per project and per session, a few dozen bytes each. That
//! is cheaper than having to reason about a gate being dropped while a command
//! is waiting on it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, TryLockError};

/// A set of mutexes, one per key, created on demand.
///
/// The lock is held for the duration of a closure rather than handed back as a
/// guard: the mutex lives behind an `Arc` in the map, and a guard borrowed from
/// it can't outlive that `Arc` without a self-referential struct. Every caller
/// wants "run this under the gate" anyway.
pub struct KeyedGates {
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Default for KeyedGates {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyedGates {
    pub fn new() -> Self {
        Self { gates: Mutex::new(HashMap::new()) }
    }

    fn gate(&self, key: &str) -> Arc<Mutex<()>> {
        self.gates.lock().unwrap().entry(key.to_string()).or_default().clone()
    }

    /// Run `f` with `key`'s gate held, waiting for it if someone else has it.
    /// The direct stand-in for main-thread serialization: callers queue and
    /// every one of them runs.
    pub fn with<T>(&self, key: &str, f: impl FnOnce() -> T) -> T {
        let gate = self.gate(key);
        // A command that panicked mid-git poisons its gate. Taking it anyway
        // keeps one bad command from wedging every later command on that key
        // for the life of the app; the panic itself is already logged by the
        // hook in `lib.rs`, so nothing is being swallowed here.
        let _held = gate.lock().unwrap_or_else(|p| p.into_inner());
        f()
    }

    /// Run `f` only if `key`'s gate is free right now; `None` if it is held.
    /// For work where queueing would be wrong rather than slow: a merge that
    /// waited its turn would then run against a checkout it never looked at.
    pub fn try_with<T>(&self, key: &str, f: impl FnOnce() -> T) -> Option<T> {
        let gate = self.gate(key);
        let held = match gate.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let out = f();
        drop(held);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::KeyedGates;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};

    #[test]
    fn try_with_refuses_a_held_gate_but_not_a_different_key() {
        let gates = Arc::new(KeyedGates::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = {
            let gates = gates.clone();
            std::thread::spawn(move || {
                gates.with("project-a", || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
            })
        };
        entered_rx.recv().unwrap();
        assert!(gates.try_with("project-a", || ()).is_none(), "a held gate must refuse");
        assert!(
            gates.try_with("project-b", || ()).is_some(),
            "a different key is a different gate"
        );
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert!(
            gates.try_with("project-a", || ()).is_some(),
            "the gate is free again once the closure returns"
        );
    }

    #[test]
    fn with_runs_one_at_a_time_and_runs_all_of_them() {
        let gates = Arc::new(KeyedGates::new());
        let inside = Arc::new(AtomicUsize::new(0));
        let overlapped = Arc::new(AtomicUsize::new(0));
        let ran = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (gates, inside, overlapped, ran) =
                    (gates.clone(), inside.clone(), overlapped.clone(), ran.clone());
                std::thread::spawn(move || {
                    gates.with("one-repo", || {
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            overlapped.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                        ran.fetch_add(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(overlapped.load(Ordering::SeqCst), 0, "two closures shared the gate");
        assert_eq!(ran.load(Ordering::SeqCst), 8, "queued work must not be dropped");
    }

    #[test]
    fn a_panicking_closure_leaves_the_gate_usable() {
        let gates = Arc::new(KeyedGates::new());
        let panicking = {
            let gates = gates.clone();
            std::thread::spawn(move || gates.with("repo", || panic!("git blew up")))
        };
        assert!(panicking.join().is_err(), "the panic must still propagate");
        assert!(
            gates.try_with("repo", || 1).is_some(),
            "a poisoned gate must not wedge the project for good"
        );
        assert_eq!(gates.with("repo", || 2), 2);
    }
}
