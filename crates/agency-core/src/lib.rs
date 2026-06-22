pub mod git;
pub mod merge;
pub mod profile;
pub mod registry;
pub mod setup;
pub mod supervisor;
pub mod tmux;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
