pub mod config;
pub mod term;
pub mod files;
pub mod gh;
pub mod git;
pub mod mcp;
pub mod merge;
pub mod profile;
pub mod registry;
pub mod scripts;
pub mod setup;
pub mod supervisor;
pub mod title;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
