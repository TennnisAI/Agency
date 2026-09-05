//! Shared integration-test harness.
//!
//! Every `AppState::new` spawns an `agency-termd` daemon that, by design,
//! outlives the client (sessions survive app restarts). In a test that means a
//! leaked daemon process per constructed state unless someone tears it down —
//! and a manual `shutdown_daemon()` at the end of a test never runs if an
//! earlier assertion panics or the test returns early. `state(&dir)` wraps the
//! `AppState` in a guard that shuts the daemon down on drop, so cleanup is
//! automatic and runs even while unwinding from a panic.

use agency_app_lib::AppState;
use std::ops::Deref;
use tempfile::TempDir;

/// An `AppState` whose backing `agency-termd` daemon is shut down when the
/// guard drops. Derefs to `AppState`, so tests use it exactly like the state.
pub struct TestState {
    state: AppState,
}

impl Deref for TestState {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        &self.state
    }
}

impl Drop for TestState {
    fn drop(&mut self) {
        // Kill any sessions this test started, then shut the daemon down.
        // Errors are irrelevant during teardown (the daemon may already be
        // gone), so they are swallowed.
        self.state.test_teardown();
    }
}

/// Build an `AppState` on `dir` whose daemon is torn down when the returned
/// guard drops. `dir` must outlive the guard (declare it first in the test).
pub fn state(dir: &TempDir) -> TestState {
    let state = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    TestState { state }
}

/// [`state`] with the agents' session-store home pointed at `home` instead of
/// `$HOME`. Everything the archive does with a conversation — rescue it,
/// reinstate it, sweep it — reads and writes under that directory, so a test
/// that cannot move it can only be run against the developer's own sessions.
#[allow(dead_code)] // not every test file in this directory uses it
pub fn state_with_agent_home(dir: &TempDir, home: &std::path::Path) -> TestState {
    let state = AppState::with_agent_home(
        &dir.path().join("agency.db"),
        dir.path(),
        Some(home.to_path_buf()),
    )
    .unwrap();
    TestState { state }
}
