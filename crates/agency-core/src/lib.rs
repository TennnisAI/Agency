pub mod git;
pub mod profile;
pub mod registry;
pub mod supervisor;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
