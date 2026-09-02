pub mod attention;
pub mod branchname;
pub mod briefing;
pub mod cleanup;
pub mod config;
pub mod files;
pub mod gh;
pub mod git;
pub mod graphview;
pub mod guide;
pub mod issuefs;
pub mod loops;
pub mod mcp;
pub mod merge;
pub mod preview;
pub(crate) mod procutil;
pub mod profile;
pub mod record;
pub mod registry;
pub mod runsetup;
pub mod scripts;
pub mod search;
pub mod sessionstore;
pub mod setup;
pub mod skills;
pub mod supervisor;
pub mod term;
pub mod title;
pub mod transcript;
pub mod usage;
pub mod version;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
