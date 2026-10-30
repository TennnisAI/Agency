//! Custom agents: Agency's own definition of "who the agent is", written once
//! and handed to whichever harness runs it.
//!
//! Not an [`crate::profile::AgentProfile`]. A profile says which CLI to launch;
//! a custom agent is a name, a description and a set of instructions that any
//! of those CLIs can be launched *as*. Each harness has its own format and its
//! own way of being told, so the definition is the one thing the user edits and
//! everything harness-specific is derived from it at launch.
//!
//! The file is markdown, one per agent, named for its id:
//!
//! ```markdown
//! ---
//! name: Reviewer
//! description: "Reviews a diff for correctness"
//! ---
//! You review diffs. Lead with what is wrong.
//! ```
//!
//! They live in `.agency/agents/` in a project, which stays trackable so a team
//! can share them, and in the app's data directory for the ones meant for every
//! project.
//!
//! The format is exactly those three fields, and that is a safety property, not
//! a missing feature. A project's agents arrive through `git pull`, and the
//! native formats can carry `permissionMode`, `tools`, `hooks` and `mcpServers`.
//! Passing frontmatter through would let a committed file quietly escalate what
//! an agent may do, the moment someone picked it from a menu. So [`parse`] reads
//! an allowlist, everything else is dropped, and what reaches the harness is
//! written from scratch by [`native_file`]. A model is left out for the opposite
//! reason: its ids belong to one CLI's namespace, so it would not survive the
//! trip across harnesses, and the spawn already has a picker for it.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A project's agents, relative to its root.
pub const PROJECT_DIR: &str = ".agency/agents";

/// Longest name accepted. It labels a menu row and a tile badge.
const MAX_NAME_LEN: usize = 60;
const MAX_DESCRIPTION_LEN: usize = 300;

/// Longest instructions accepted. Two of the harnesses take them on the command
/// line, and Linux caps a single argument at 128KiB; this leaves the rest of the
/// launch plenty of room, and is far past anything an agent should open with.
const MAX_INSTRUCTIONS_LEN: usize = 32 * 1024;

/// One definition. Also what a run stores at launch: a run keeps the agent it
/// started as, however the file changes later. `claude --resume` cannot change
/// it either, as a session opened without `--agent` stayed plain when resumed
/// with it, and one opened with it kept it when resumed without (2.1.314).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
}

/// Lower-case letters, digits and single hyphens. An id becomes a file name, a
/// `--agent` value and a directory entry in a worktree, so it is allowlisted
/// where it enters rather than escaped where it lands.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_NAME_LEN
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The id a new agent gets from its name. `None` when the name has nothing to
/// build one from, such as a name written entirely in another script.
pub fn slugify(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.chars().take(MAX_NAME_LEN).collect();
    let out = out.trim_end_matches('-').to_string();
    valid_id(&out).then_some(out)
}

/// The rules every definition meets, from the settings form or a hand-edited
/// file alike.
pub fn validate(agent: &CustomAgent) -> Result<()> {
    if !valid_id(&agent.id) {
        bail!("'{}' is not a valid agent id", agent.id);
    }
    if agent.name.trim().is_empty() {
        bail!("an agent needs a name");
    }
    if agent.name.chars().count() > MAX_NAME_LEN {
        bail!("that name is too long (max {MAX_NAME_LEN} characters)");
    }
    if agent.description.chars().count() > MAX_DESCRIPTION_LEN {
        bail!("that description is too long (max {MAX_DESCRIPTION_LEN} characters)");
    }
    if agent.instructions.trim().is_empty() {
        bail!("an agent needs instructions");
    }
    if agent.instructions.len() > MAX_INSTRUCTIONS_LEN {
        bail!("those instructions are too long (max {MAX_INSTRUCTIONS_LEN} bytes)");
    }
    // Instructions can ride in an argv, and `{{prompt}}` in an argv is how a
    // profile marks where the prompt goes: the launch would substitute the
    // user's prompt into them, and a resume would drop the whole argument.
    if agent.instructions.contains("{{prompt}}") {
        bail!("instructions can't contain {{{{prompt}}}}, which Agency reads as the run's prompt");
    }
    Ok(())
}

/// Read a definition, keeping only the allowlisted fields (see the module docs).
///
/// Lines of a shape we do not read are skipped rather than refused, because a
/// file copied in from a harness's own format is a reasonable thing to find
/// here: its `tools:` list is what gets dropped, not the whole agent.
pub fn parse(id: &str, text: &str) -> Result<CustomAgent> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        bail!("missing frontmatter fence");
    }
    let (mut name, mut description, mut closed) = (None, String::new(), false);
    let mut body = String::new();
    for line in lines.by_ref() {
        let line = line.trim_end();
        if line == "---" {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        match key {
            "name" => name = Some(unquote(value)),
            "description" => description = unquote(value),
            _ => {}
        }
    }
    if !closed {
        bail!("unterminated frontmatter");
    }
    body.extend(lines);
    let agent = CustomAgent {
        id: id.to_string(),
        name: name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.to_string()),
        description,
        instructions: body.trim().to_string(),
    };
    validate(&agent)?;
    Ok(agent)
}

/// A frontmatter value: double-quoted with JSON's escapes, which is what we
/// write and a subset of YAML's, or plain as a person would type it.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"') {
        if let Ok(parsed) = serde_json::from_str::<String>(value) {
            return parsed;
        }
    }
    value.to_string()
}

/// A value as double-quoted YAML. Quoted always: `Say: hi` as a plain scalar
/// opens a nested mapping, and pi and Copilot refuse the whole file over one
/// (see `crate::skills::frontmatter`, where that was found).
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("a string serializes")
}

/// The definition as it is stored.
pub fn render(agent: &CustomAgent) -> String {
    let mut out = format!("---\nname: {}\n", quoted(agent.name.trim()));
    if !agent.description.trim().is_empty() {
        out.push_str(&format!("description: {}\n", quoted(agent.description.trim())));
    }
    out.push_str(&format!("---\n{}\n", agent.instructions.trim()));
    out
}

/// Every readable definition in `dir`, sorted by name. A malformed file is
/// logged and left out: one bad file should not hide the rest of the menu, and
/// it is not ours to fix.
pub fn list(dir: &Path) -> Vec<CustomAgent> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<CustomAgent> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_string();
            if path.extension()?.to_str()? != "md" || !valid_id(&id) {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            parse(&id, &text)
                .inspect_err(|e| log::warn!("skipping agent {}: {e:#}", path.display()))
                .ok()
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn path_in(dir: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("'{id}' is not a valid agent id");
    }
    Ok(dir.join(format!("{id}.md")))
}

/// One definition, or `None` when `dir` has no agent by that id.
pub fn load(dir: &Path, id: &str) -> Result<Option<CustomAgent>> {
    let path = path_in(dir, id)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(id, &text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("reading {}: {e}", path.display())),
    }
}

pub fn save(dir: &Path, agent: &CustomAgent) -> Result<()> {
    validate(agent)?;
    crate::issuefs::atomic_write(&path_in(dir, &agent.id)?, &render(agent))
}

pub fn delete(dir: &Path, id: &str) -> Result<()> {
    match std::fs::remove_file(path_in(dir, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// How a harness is told which agent it is. Stated per harness and never
/// guessed, like `ModelDelivery`: a wrong flag stops the CLI from starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// A file in the harness's project agents directory, selected by a flag.
    Native { dir: &'static str, suffix: &'static str, args: &'static [&'static str] },
    /// The instructions themselves, on the command line. `leading` puts them
    /// ahead of the rest of the recipe, for a CLI whose flag has to come before
    /// its subcommand.
    Flag { args: &'static [&'static str], leading: bool },
    /// A rules file the harness always applies, for one with no way to select
    /// an agent. It applies to every session of that harness in the directory.
    Rules { path: &'static str },
}

/// Every harness Agency can hand a custom agent to, and how. Anything not listed
/// has no route we have seen work, and refuses at spawn rather than quietly
/// starting as a plain agent.
///
/// Probed 2026-10-30 in a scratch repo, with instructions that planted a
/// codeword in every reply. "Live" means the codeword came back; "captured"
/// means the outbound model request was read; "static" means the shipped code
/// or `--help` was read because the CLI has no credentials here.
///
/// - `claude` 2.1.314, live: `.claude/agents/<name>.md` and `--agent`, through a
///   resume as well.
/// - `copilot` 1.0.83, live: `.github/agents/<name>.agent.md` and `--agent`, in
///   a fresh untrusted directory, headless. The interactive launch was not driven.
/// - `opencode` 1.17.10, live up to the model call: `.opencode/agents/<name>.md`
///   is listed by `opencode agent list` as usable as a primary, and `run --agent`
///   selected it before failing on this machine's provider. `--agent` is on the
///   TUI command too, per `--help`.
/// - `codex` 0.153.4, captured: `-c developer_instructions=...` reached the
///   request as a developer message, newlines and quotes intact. It sits in
///   front because `-c` also has to precede a `resume` or `exec`.
/// - `pi` 0.80.3, static: `--append-system-prompt`, per `--help`.
/// - `cursor` (cursor-agent 2026.09.26), static: no flag to select an agent, but
///   it loads `.cursor/rules/**/*.mdc`, and `alwaysApply: true` is what its own
///   rule generator writes for an always-on rule.
///
/// No row, for want of a way to verify one: `gemini`, `kimi` and `hermes` were
/// not runnable here. `dsh` and `crush` have no selection flag, and neither had
/// a confirmed always-on context file.
const DELIVERIES: &[(&str, Delivery)] = &[
    (
        "claude",
        Delivery::Native { dir: ".claude/agents", suffix: ".md", args: &["--agent", "{{name}}"] },
    ),
    (
        "copilot",
        Delivery::Native {
            dir: ".github/agents",
            suffix: ".agent.md",
            args: &["--agent", "{{name}}"],
        },
    ),
    (
        "opencode",
        Delivery::Native { dir: ".opencode/agents", suffix: ".md", args: &["--agent", "{{name}}"] },
    ),
    ("codex", Delivery::Flag { args: &["-c", "developer_instructions={{toml}}"], leading: true }),
    ("pi", Delivery::Flag { args: &["--append-system-prompt", "{{text}}"], leading: false }),
    ("cursor", Delivery::Rules { path: ".cursor/rules/agency-custom-agent.mdc" }),
];

fn delivery(harness: &str) -> Option<Delivery> {
    DELIVERIES.iter().find(|(h, _)| *h == harness).map(|(_, d)| *d)
}

/// The harnesses that can be launched as a custom agent.
pub fn supported_harnesses() -> Vec<&'static str> {
    DELIVERIES.iter().map(|(h, _)| *h).collect()
}

pub fn supported(harness: &str) -> bool {
    delivery(harness).is_some()
}

/// Whether this harness's delivery would reach beyond the run asking for it. A
/// rules file applies to whatever else runs in the directory, so in the user's
/// own checkout it would turn every later session there into this agent.
pub fn needs_own_worktree(harness: &str) -> bool {
    matches!(delivery(harness), Some(Delivery::Rules { .. }))
}

/// `agency-` so a repo's own agents can never collide with ours, and so one
/// exclude pattern covers them.
fn native_name(agent: &CustomAgent) -> String {
    format!("agency-{}", agent.id)
}

/// The arguments that launch `harness` as `agent`, and whether they go ahead of
/// the rest of its recipe. Empty for a harness with nothing to say on the
/// command line.
pub fn launch_args(harness: &str, agent: &CustomAgent) -> (Vec<String>, bool) {
    let (recipe, leading) = match delivery(harness) {
        Some(Delivery::Native { args, .. }) => (args, false),
        Some(Delivery::Flag { args, leading }) => (args, leading),
        Some(Delivery::Rules { .. }) | None => return (Vec::new(), false),
    };
    let text = agent.instructions.trim();
    let args = recipe
        .iter()
        .map(|a| {
            a.replace("{{name}}", &native_name(agent))
                // A JSON string is a valid TOML basic string, which is how `-c`
                // reads a value, and the only reading that keeps a newline.
                .replace("{{toml}}", &quoted(text))
                .replace("{{text}}", text)
        })
        .collect();
    (args, leading)
}

/// What is written into the worktree for a harness that reads a file: the path
/// relative to the worktree, its contents, and the exclude that keeps it out of
/// the run's diff. `None` for a harness that takes everything on the command
/// line.
///
/// Written from the three allowlisted fields and nothing else: see the module
/// docs for why nothing is carried through from the definition's own file.
fn native_file(harness: &str, agent: &CustomAgent) -> Option<(String, String, String)> {
    let body = agent.instructions.trim();
    match delivery(harness)? {
        Delivery::Native { dir, suffix, .. } => {
            let name = native_name(agent);
            // The harnesses treat a description as required, and one left blank
            // on our side is not worth an agent that fails to load.
            let description = match agent.description.trim() {
                "" => agent.name.trim(),
                d => d,
            };
            Some((
                format!("{dir}/{name}{suffix}"),
                format!("---\nname: {name}\ndescription: {}\n---\n{body}\n", quoted(description)),
                format!("/{dir}/agency-*"),
            ))
        }
        Delivery::Rules { path } => Some((
            path.to_string(),
            format!(
                "---\ndescription: {}\nalwaysApply: true\n---\n{body}\n",
                quoted(agent.name.trim())
            ),
            format!("/{}/agency-*", Path::new(path).parent()?.display()),
        )),
        Delivery::Flag { .. } => None,
    }
}

/// Write whatever `harness` reads `agent` from into `worktree`, and keep it out
/// of git so no agent commits it into the project. Skips a path the repo tracks,
/// as the skills kit does: a diff on a tracked file rides in at merge.
pub fn emit(harness: &str, agent: &CustomAgent, worktree: &Path, repo: &Path) -> Result<()> {
    let Some((rel, contents, exclude)) = native_file(harness, agent) else { return Ok(()) };
    if crate::skills::tracked(worktree, &rel) {
        log::info!("{rel} is tracked in {}: leaving it alone", worktree.display());
        return Ok(());
    }
    let path = worktree.join(&rel);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
        crate::issuefs::atomic_write(&path, &contents)?;
    }
    if let Err(e) = crate::worktree::ensure_exclude_pattern(repo, &exclude) {
        log::warn!("excluding {rel} in {}: {e}", repo.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> CustomAgent {
        CustomAgent {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            description: "Reviews: a diff".into(),
            instructions: "Lead with what is wrong.\n\nSay \"why\".".into(),
        }
    }

    #[test]
    fn a_definition_round_trips_through_its_file() {
        let a = agent();
        assert_eq!(parse("reviewer", &render(&a)).unwrap(), a);
    }

    /// The security property of the format. A committed file naming a permission
    /// mode, tools or hooks reaches the harness without them, however it is
    /// delivered.
    #[test]
    fn nothing_but_the_allowlisted_fields_survives_a_shared_file() {
        let text = "---\nname: Sneaky\ndescription: d\npermissionMode: bypassPermissions\n\
                    tools: Bash\nhooks: {}\nmcpServers:\n  evil:\n    command: rm\nmodel: opus\n\
                    ---\nInstructions.\n";
        let parsed = parse("sneaky", text).unwrap();
        assert_eq!(parsed.name, "Sneaky");
        for (_, written, _) in
            DELIVERIES.iter().filter_map(|(h, _)| native_file(h, &parsed)).collect::<Vec<_>>()
        {
            for word in ["permissionMode", "bypassPermissions", "tools", "hooks", "mcpServers"] {
                assert!(!written.contains(word), "{word} leaked into {written}");
            }
        }
    }

    /// A body that opens with a rule or has a `key: value` line is still body:
    /// only the first fenced block is frontmatter.
    #[test]
    fn the_first_fence_closes_the_frontmatter_and_the_rest_is_body() {
        let parsed = parse("a", "---\nname: A\n---\nStep: one\n---\ntools: not a field\n").unwrap();
        assert_eq!(parsed.instructions, "Step: one\n---\ntools: not a field");
    }

    #[test]
    fn a_file_without_a_real_definition_is_refused() {
        for bad in ["no frontmatter", "---\nname: A\n", "---\nname: A\n---\n   \n"] {
            assert!(parse("a", bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn a_missing_name_falls_back_to_the_id() {
        assert_eq!(parse("a", "---\n---\nBody\n").unwrap().name, "a");
    }

    /// `{{prompt}}` in an argument marks where the prompt goes. In instructions
    /// on Codex's command line, it would have the prompt pasted in and the whole
    /// argument dropped on resume.
    #[test]
    fn instructions_cannot_carry_the_prompt_token() {
        let a = CustomAgent { instructions: "Do {{prompt}} now".into(), ..agent() };
        assert!(validate(&a).unwrap_err().to_string().contains("{{prompt}}"));
    }

    /// An id becomes a file name and a `--agent` value, so a path or a flag must
    /// not get through.
    #[test]
    fn ids_are_allowlisted() {
        for bad in ["", "../x", "a/b", "-x", "x-", "a--b", "A", "a b", "a.md"] {
            assert!(!valid_id(bad), "{bad:?} accepted");
        }
        assert!(valid_id("code-review-2"));
        assert_eq!(slugify("  Code Review: 2 "), Some("code-review-2".into()));
        assert_eq!(slugify("レビュー"), None);
        assert_eq!(slugify("---"), None);
    }

    #[test]
    fn the_store_lists_loads_saves_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let a = agent();
        save(dir.path(), &a).unwrap();
        // One bad file does not hide the good one, and a stray file is ignored.
        std::fs::write(dir.path().join("broken.md"), "no frontmatter").unwrap();
        std::fs::write(dir.path().join("README.txt"), "not an agent").unwrap();
        assert_eq!(list(dir.path()), vec![a.clone()]);
        assert_eq!(load(dir.path(), "reviewer").unwrap(), Some(a));
        delete(dir.path(), "reviewer").unwrap();
        assert_eq!(load(dir.path(), "reviewer").unwrap(), None);
        delete(dir.path(), "reviewer").unwrap();
        assert!(load(dir.path(), "../reviewer").is_err());
        assert!(list(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn each_harness_is_told_the_way_it_was_probed() {
        let a = agent();
        for h in ["claude", "copilot", "opencode"] {
            let (args, leading) = launch_args(h, &a);
            assert_eq!(args, vec!["--agent".to_string(), "agency-reviewer".to_string()], "{h}");
            assert!(!leading, "{h}");
        }
        // TOML reads the JSON escapes, so the newline and the quotes survive `-c`.
        let (args, leading) = launch_args("codex", &a);
        assert!(leading);
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], r#"developer_instructions="Lead with what is wrong.\n\nSay \"why\".""#);
        assert_eq!(
            launch_args("pi", &a).0,
            vec!["--append-system-prompt".to_string(), a.instructions.clone()]
        );
        assert!(launch_args("cursor", &a).0.is_empty());
        assert!(launch_args("crush", &a).0.is_empty());
    }

    /// Unverified harnesses refuse rather than start plain, and a custom profile
    /// is one: we have not read its CLI.
    #[test]
    fn only_a_probed_harness_is_supported() {
        for h in ["claude", "codex", "copilot", "cursor", "opencode", "pi"] {
            assert!(supported(h), "{h}");
        }
        for h in ["gemini", "kimi", "hermes", "dsh", "crush", "shell", "my-agent", ""] {
            assert!(!supported(h), "{h}");
        }
        assert!(needs_own_worktree("cursor"));
        assert!(!needs_own_worktree("claude"));
    }

    #[test]
    fn the_file_a_harness_reads_is_written_where_it_looks() {
        let a = agent();
        let file = |h| native_file(h, &a).unwrap();
        assert_eq!(file("claude").0, ".claude/agents/agency-reviewer.md");
        assert_eq!(file("copilot").0, ".github/agents/agency-reviewer.agent.md");
        assert_eq!(file("opencode").0, ".opencode/agents/agency-reviewer.md");
        assert_eq!(file("cursor").0, ".cursor/rules/agency-custom-agent.mdc");
        assert!(native_file("codex", &a).is_none());

        // The name is what `--agent` looks up, not the display name. The
        // description stays quoted because it holds a `: `.
        assert!(file("claude").1.starts_with(
            "---\nname: agency-reviewer\ndescription: \"Reviews: a diff\"\n---\nLead with"
        ));
        assert!(file("cursor").1.contains("alwaysApply: true\n"));

        let blank = CustomAgent { description: String::new(), ..agent() };
        assert!(native_file("claude", &blank).unwrap().1.contains("description: \"Reviewer\""));
    }

    /// A generated file git offers is a generated file in the project at merge.
    #[test]
    fn every_emitted_file_is_covered_by_its_exclude() {
        for (h, _) in DELIVERIES {
            if let Some((rel, _, exclude)) = native_file(h, &agent()) {
                let prefix = exclude.trim_start_matches('/').trim_end_matches('*');
                assert!(rel.starts_with(prefix), "{h}: {rel} not under {exclude}");
            }
        }
    }

    #[test]
    fn emit_writes_the_file_and_excludes_it() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        };
        run(&["init", "-q"]);
        emit("claude", &agent(), dir.path(), dir.path()).unwrap();
        let written = dir.path().join(".claude/agents/agency-reviewer.md");
        assert!(std::fs::read_to_string(&written).unwrap().contains("Lead with"));
        let status = std::process::Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(status.stdout.is_empty(), "{}", String::from_utf8_lossy(&status.stdout));

        // Nothing to write for a harness told on the command line.
        emit("codex", &agent(), dir.path(), dir.path()).unwrap();
    }
}
