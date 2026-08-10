pub mod config;
pub mod files;
pub mod gh;
pub mod git;
pub mod guide;
pub mod issuefs;
pub mod loops;
pub mod mcp;
pub mod merge;
pub mod profile;
pub mod registry;
pub mod runsetup;
pub mod scripts;
pub mod search;
pub mod setup;
pub mod supervisor;
pub mod term;
pub mod title;
pub mod version;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
