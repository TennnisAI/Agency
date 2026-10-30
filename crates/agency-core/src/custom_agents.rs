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
    /// Set when a run takes its copy, and part of the name the harness looks the
    /// agent up by (see [`native_name`]). Never read from a definition's file.
    #[serde(default)]
    pub launch_tag: String,
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

/// A name as it is shown: single spaces, however it was typed. Runs of spaces,
/// Unicode ones included, could push the rest of a label out of a narrow pill.
pub fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Characters that change how text around them reads without showing themselves:
/// bidi overrides and isolates, zero-width characters, soft hyphens and the like.
/// A name in a picker says where an agent came from, and a project's agents
/// arrive through git, so an override could make "Project: Reviewer" read as
/// something else.
fn hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
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
    if agent.name.chars().chain(agent.description.chars()).any(hidden_char) {
        bail!("names and descriptions can't contain invisible or text direction characters");
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
    let mut lines = text.split_inclusive('\n').peekable();
    if lines.next().map(str::trim_end) != Some("---") {
        bail!("missing frontmatter fence");
    }
    let (mut name, mut description, mut closed) = (None, String::new(), false);
    let mut body = String::new();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line == "---" {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        if key != "name" && key != "description" {
            continue;
        }
        let value = if block_scalar(value) {
            let mut parts = Vec::new();
            while let Some(next) =
                lines.next_if(|l| l.trim().is_empty() || l.starts_with([' ', '\t']))
            {
                parts.push(next.trim());
            }
            parts.join(" ")
        } else {
            unquote(value)
        };
        if key == "name" {
            name = Some(value);
        } else {
            description = value;
        }
    }
    if !closed {
        bail!("unterminated frontmatter");
    }
    body.extend(lines);
    let agent = CustomAgent {
        id: id.to_string(),
        name: name
            .map(|n| clean_name(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string()),
        // Whitespace collapsed because a harness's description can hold `\n`, which
        // `validate` refuses, and cut short because one carrying routing examples
        // runs far past the limit. Neither is worth losing the whole agent over.
        description: clean_name(&description)
            .chars()
            .take(MAX_DESCRIPTION_LEN)
            .collect::<String>()
            .trim_end()
            .to_string(),
        instructions: body.trim().to_string(),
        launch_tag: String::new(),
    };
    validate(&agent)?;
    Ok(agent)
}

/// Whether a frontmatter value is YAML's `>` or `|`, with the text on the indented
/// lines below. Read as a plain value, `description: >` was listed as a literal
/// `>`, and that was what got written into the emitted file.
fn block_scalar(value: &str) -> bool {
    let value = value.trim();
    value.starts_with(['>', '|']) && value[1..].chars().all(|c| matches!(c, '+' | '-' | '1'..='9'))
}

/// A frontmatter value: double-quoted with JSON's escapes, which is what we
/// write and a subset of YAML's, single-quoted as YAML has it, or plain as a
/// person would type it.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
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

/// Whether `git status --porcelain --untracked-files=all` output is anything that
/// should stop a merge or a spawn. An untracked file in [`PROJECT_DIR`] is not:
/// saving a "This project" agent left `?? .agency/` in the status, and every merge
/// then refused with "uncommitted changes" until it was committed. The spawn reads
/// the definition from the checkout, so a worktree cut without it loses nothing.
/// It stays visible to git, because committing it is how it gets shared. An edit
/// to one already committed still counts, as any tracked change does.
pub fn checkout_dirty(porcelain: &str) -> bool {
    let unshared = format!("?? {PROJECT_DIR}/");
    porcelain.lines().any(|l| !l.trim().is_empty() && !l.starts_with(&unshared))
}

/// A definition [`list`] left out, for Settings to say so. An agent missing from
/// the menu with the reason only in a log looks like the app lost it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkippedAgent {
    pub file: String,
    pub reason: String,
}

/// Every readable definition in `dir`, sorted by name, and the files that weren't.
/// One bad file should not hide the rest of the menu, and it is not ours to fix.
pub fn list(dir: &Path) -> (Vec<CustomAgent>, Vec<SkippedAgent>) {
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(dir) else { return (out, skipped) };
    for path in entries.flatten().map(|entry| entry.path()) {
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if path.extension().and_then(|e| e.to_str()) != Some("md") || !valid_id(id) {
            continue;
        }
        let read = std::fs::read_to_string(&path).map_err(anyhow::Error::from);
        match read.and_then(|text| parse(id, &text)) {
            Ok(agent) => out.push(agent),
            Err(e) => {
                log::warn!("skipping agent {}: {e:#}", path.display());
                skipped.push(SkippedAgent {
                    file: path.display().to_string(),
                    reason: format!("{e:#}"),
                });
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    skipped.sort_by(|a, b| a.file.cmp(&b.file));
    (out, skipped)
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
    Rules { dir: &'static str },
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
    ("cursor", Delivery::Rules { dir: ".cursor/rules" }),
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

/// The name a harness looks the agent up by. `agency-` so one exclude pattern
/// covers them, and the run's tag because a name a repo can predict is a name it
/// can answer to: `.claude/agents/anything.md` declaring `name: agency-reviewer`
/// would be what `--agent agency-reviewer` launched, `permissionMode` and all,
/// when the user picked their own agent called Reviewer. That would undo the
/// allowlist this module exists for, using a file Agency never wrote.
fn native_name(agent: &CustomAgent) -> String {
    match agent.launch_tag.as_str() {
        "" => format!("agency-{}", agent.id),
        tag => format!("agency-{}-{tag}", agent.id),
    }
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
    let name = native_name(agent);
    // A JSON string is a valid TOML basic string, which is how `-c` reads a value,
    // and the only reading that keeps a newline.
    let toml = quoted(text);
    let tokens = [("{{name}}", name.as_str()), ("{{toml}}", toml.as_str()), ("{{text}}", text)];
    (recipe.iter().map(|a| fill(a, &tokens)).collect(), leading)
}

/// Replace `tokens` in `recipe`, in one pass. Chained `replace` calls expanded a
/// `{{text}}` written inside instructions a second time on Codex, which put raw
/// newlines and quotes in the middle of the TOML string and broke the `-c` value.
fn fill(recipe: &str, tokens: &[(&str, &str)]) -> String {
    let (mut out, mut rest) = (String::new(), recipe);
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match tokens.iter().find(|(token, _)| rest.starts_with(token)) {
            Some((token, value)) => {
                out.push_str(value);
                rest = &rest[token.len()..];
            }
            None => {
                out.push_str("{{");
                rest = &rest[2..];
            }
        }
    }
    out.push_str(rest);
    out
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
        Delivery::Rules { dir } => Some((
            format!("{dir}/{}.mdc", native_name(agent)),
            format!(
                "---\ndescription: {}\nalwaysApply: true\n---\n{body}\n",
                quoted(agent.name.trim())
            ),
            format!("/{dir}/agency-*"),
        )),
        Delivery::Flag { .. } => None,
    }
}

/// Refuse a path that leaves the worktree through a symlink. The parts of `rel`
/// are the repo's to commit, and a `.claude/agents` linked at the user's own
/// config would have this write there, which Agency never does.
fn refuse_symlinks(worktree: &Path, rel: &str) -> Result<()> {
    let mut path = worktree.to_path_buf();
    for part in Path::new(rel) {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!("{} is a symlink, so a custom agent can't be written there", path.display())
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

/// Write whatever `harness` reads `agent` from into `worktree`, and keep it out
/// of git so no agent commits it into the project. An error means the harness
/// would not get this agent, so the launch has to stop.
pub fn emit(harness: &str, agent: &CustomAgent, worktree: &Path, repo: &Path) -> Result<()> {
    let Some((rel, contents, exclude)) = native_file(harness, agent) else { return Ok(()) };
    refuse_symlinks(worktree, &rel)?;
    let path = worktree.join(&rel);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
        crate::issuefs::atomic_write(&path, &contents)?;
    }
    if let Err(e) = crate::worktree::ensure_exclude_pattern(repo, &exclude) {
        log::warn!("excluding {rel} in {}: {e}", repo.display());
    }
    Ok(())
}

/// Undo [`emit`] once a run is over. The run's tag is in the name, so this can
/// only ever be the file that run wrote. Refuses a symlink for the reason `emit`
/// does, and a file already gone is the outcome it wanted.
pub fn remove_emitted(harness: &str, agent: &CustomAgent, worktree: &Path) -> Result<()> {
    let Some((rel, _, _)) = native_file(harness, agent) else { return Ok(()) };
    refuse_symlinks(worktree, &rel)?;
    match std::fs::remove_file(worktree.join(&rel)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
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
            launch_tag: String::new(),
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

    /// A picker's label says where an agent came from, and a shared file must not
    /// be able to disguise that.
    #[test]
    fn a_name_cannot_hide_or_reorder_what_is_shown_beside_it() {
        for bad in ["Rev\u{202e}iewer", "Rev\u{2067}iewer", "Rev\u{200b}iewer", "Rev\u{7}iewer"] {
            let text = format!("---\nname: {bad}\n---\nBody\n");
            assert!(parse("a", &text).is_err(), "{bad:?} parsed");
        }
        let text = "---\nname: A\ndescription: \"x\\u202ey\"\n---\nBody\n";
        assert!(parse("a", text).is_err());

        let spaced = parse("a", "---\nname: Rev  \u{a0}\u{2003}iewer\n---\nBody\n").unwrap();
        assert_eq!(spaced.name, "Rev iewer");
        // Not everything outside ASCII: a name in another script is a fine name.
        assert_eq!(parse("a", "---\nname: レビュー\n---\nBody\n").unwrap().name, "レビュー");
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
        assert_eq!(list(dir.path()).0, vec![a.clone()]);
        assert_eq!(load(dir.path(), "reviewer").unwrap(), Some(a));
        delete(dir.path(), "reviewer").unwrap();
        assert_eq!(load(dir.path(), "reviewer").unwrap(), None);
        delete(dir.path(), "reviewer").unwrap();
        assert!(load(dir.path(), "../reviewer").is_err());
        assert!(list(&dir.path().join("missing")).0.is_empty());
    }

    /// The file that couldn't be read is named, and the rest are still listed.
    #[test]
    fn a_bad_file_is_reported_without_hiding_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &agent()).unwrap();
        std::fs::write(dir.path().join("broken.md"), "no frontmatter").unwrap();
        std::fs::write(dir.path().join("README.md"), "not an agent").unwrap();
        let (agents, skipped) = list(dir.path());
        assert_eq!(agents, vec![agent()]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].file.ends_with("broken.md"), "{skipped:?}");
        assert!(skipped[0].reason.contains("frontmatter"), "{skipped:?}");
    }

    /// Copied in from a harness's own format, which a person is entitled to do.
    #[test]
    fn a_harness_format_file_reads_as_what_it_says() {
        let folded = "---\nname: A\ndescription: >-\n  Reviews a diff\n\n  for bugs\ntools: Bash\n---\nBody\n";
        let parsed = parse("a", folded).unwrap();
        assert_eq!(parsed.description, "Reviews a diff for bugs");
        assert_eq!(parsed.instructions, "Body");

        let literal = "---\nname: |\n  Reviewer\ndescription: x\n---\nBody\n";
        assert_eq!(parse("a", literal).unwrap().name, "Reviewer");

        let quoted = "---\nname: 'Bob''s reviewer'\ndescription: 'Reviews: a diff'\n---\nBody\n";
        let parsed = parse("a", quoted).unwrap();
        assert_eq!(parsed.name, "Bob's reviewer");
        assert_eq!(parsed.description, "Reviews: a diff");

        // A value that only starts like a block is a value.
        let plain = "---\nname: A\ndescription: > 3 files\n---\nBody\n";
        assert_eq!(parse("a", plain).unwrap().description, "> 3 files");
    }

    #[test]
    fn a_long_or_multi_line_description_does_not_lose_the_agent() {
        let long = "x".repeat(MAX_DESCRIPTION_LEN + 50);
        let parsed = parse("a", &format!("---\ndescription: {long}\n---\nBody\n")).unwrap();
        assert_eq!(parsed.description.chars().count(), MAX_DESCRIPTION_LEN);

        let text = "---\ndescription: \"one\\ntwo\"\n---\nBody\n";
        assert_eq!(parse("a", text).unwrap().description, "one two");
        // The form still says so: this is only for a file someone else wrote.
        assert!(validate(&CustomAgent { description: long, ..agent() }).is_err());
    }

    /// A template explained in the instructions is text, not a token. Observed on
    /// Codex, where `{{text}}` inside them broke the TOML string it landed in.
    #[test]
    fn a_token_in_the_instructions_is_not_expanded() {
        let a = CustomAgent {
            instructions: "Use {{text}}, {{toml}} and {{name}}.\nDone".into(),
            ..agent()
        };
        assert_eq!(
            launch_args("codex", &a).0[1],
            r#"developer_instructions="Use {{text}}, {{toml}} and {{name}}.\nDone""#
        );
        assert_eq!(launch_args("pi", &a).0[1], "Use {{text}}, {{toml}} and {{name}}.\nDone");
        assert_eq!(fill("a{{b}}c{{", &[("{{x}}", "y")]), "a{{b}}c{{");
    }

    #[test]
    fn only_an_unshared_agent_is_left_out_of_a_dirty_checkout() {
        assert!(!checkout_dirty(""));
        assert!(!checkout_dirty("?? .agency/agents/reviewer.md\n"));
        assert!(checkout_dirty("?? .agency/agents/reviewer.md\n?? notes.md\n"));
        // A committed agent's edit is a tracked change like any other.
        assert!(checkout_dirty(" M .agency/agents/reviewer.md\n"));
        assert!(checkout_dirty("?? .agency/agents-old/x.md\n"));
    }

    #[test]
    fn remove_emitted_takes_the_runs_own_file_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let mine = CustomAgent { launch_tag: "ab12".into(), ..agent() };
        let other = CustomAgent { launch_tag: "cd34".into(), ..agent() };
        emit("claude", &mine, dir.path(), dir.path()).unwrap();
        emit("claude", &other, dir.path(), dir.path()).unwrap();

        remove_emitted("claude", &mine, dir.path()).unwrap();
        assert!(!dir.path().join(".claude/agents/agency-reviewer-ab12.md").exists());
        assert!(dir.path().join(".claude/agents/agency-reviewer-cd34.md").exists());
        // Already gone, and a harness that wrote nothing, are both fine.
        remove_emitted("claude", &mine, dir.path()).unwrap();
        remove_emitted("codex", &mine, dir.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remove_emitted_does_not_follow_a_symlink_out() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mine = CustomAgent { launch_tag: "ab12".into(), ..agent() };
        std::fs::write(outside.path().join("agency-reviewer-ab12.md"), "theirs").unwrap();
        std::fs::create_dir(dir.path().join(".claude")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".claude/agents")).unwrap();
        assert!(remove_emitted("claude", &mine, dir.path()).is_err());
        assert!(outside.path().join("agency-reviewer-ab12.md").exists());
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
        assert_eq!(file("cursor").0, ".cursor/rules/agency-reviewer.mdc");
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

    /// A repo can commit a file answering to any name it can predict, so the name
    /// carries the run's tag, and no definition's file can set that tag.
    #[test]
    fn a_runs_agent_answers_to_a_name_no_repo_can_predict() {
        let tagged = CustomAgent { launch_tag: "ab12".into(), ..agent() };
        assert_eq!(launch_args("claude", &tagged).0[1], "agency-reviewer-ab12");
        assert_eq!(
            native_file("claude", &tagged).unwrap().0,
            ".claude/agents/agency-reviewer-ab12.md"
        );
        assert!(native_file("claude", &tagged).unwrap().1.contains("name: agency-reviewer-ab12\n"));
        assert_eq!(
            native_file("cursor", &tagged).unwrap().0,
            ".cursor/rules/agency-reviewer-ab12.mdc"
        );

        let planted = "---\nname: A\nlaunch_tag: ab12\n---\nBody\n";
        assert_eq!(parse("a", planted).unwrap().launch_tag, "");
        assert!(!render(&tagged).contains("ab12"));
    }

    /// A committed `.claude/agents` linked at the user's own config must not take
    /// the write, and neither may a link in the file's own place.
    #[cfg(unix)]
    #[test]
    fn emit_refuses_to_write_through_a_symlink() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".claude")).unwrap();
        symlink(outside.path(), dir.path().join(".claude/agents")).unwrap();
        let e = emit("claude", &agent(), dir.path(), dir.path()).unwrap_err();
        assert!(e.to_string().contains("symlink"), "{e}");
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        let target = outside.path().join("target.md");
        std::fs::write(&target, "theirs").unwrap();
        symlink(&target, dir.path().join(".claude/agents/agency-reviewer.md")).unwrap();
        assert!(emit("claude", &agent(), dir.path(), dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theirs");
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
