//! Built-in agent catalog: known CLI profiles with prompt/resume/loop recipes.
//!
//! Distinct from enabled profiles in the DB — the catalog is the menu of
//! agents Agency knows how to configure; users enable a subset via onboarding
//! or Settings. `shell` is seeded separately and is not part of this catalog.

use agency_core::profile::AgentProfile;
use serde::Serialize;
use std::sync::OnceLock;

type Recipe = Option<Vec<String>>;

/// How an agent's CLI takes the opening prompt on a *fresh interactive* launch
/// (the headless one-shot shape is `loop_args`). Stated per entry rather than
/// defaulted, because a wrong guess here is silent: the prompt is the whole ask,
/// and the agent either never receives it or refuses to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDelivery {
    /// Trailing positional argument: `claude "<prompt>"`.
    Positional,
    /// A flag recipe, `{{prompt}}` marking where the text goes.
    Args(&'static [&'static str]),
    /// The CLI has no way to be handed an opening prompt, so a run created with
    /// one launches promptless and the user hands it over in the live terminal.
    Unsupported,
}

/// One catalog entry: id, binary name, how a prompt reaches it, and optional
/// resume/loop recipes.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub command: &'static str,
    /// Not persisted into the profile (unlike the recipes below): a promptless
    /// launch must drop the whole recipe, which the profile's own `args` cannot
    /// express — a `{{prompt}}` token there renders to a dangling flag. A user
    /// whose CLI has moved on can still override everything by putting
    /// `{{prompt}}` in the profile's arguments, which wins over this.
    pub prompt: PromptDelivery,
    pub resume_args: Recipe,
    pub loop_args: Recipe,
}

/// Catalog entry as sent to the UI (includes whether it's already enabled).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryInfo {
    pub id: String,
    pub command: String,
    pub resume_args: Option<Vec<String>>,
    pub loop_args: Option<Vec<String>>,
    pub enabled: bool,
    /// Whether `command` resolves on PATH (usable without a profile row).
    pub installed: bool,
    /// Whether Agency emits per-workspace MCP config for this agent.
    pub supports_mcp: bool,
    /// Whether Agency can register an OAuth MCP server with this agent's own
    /// CLI at user scope (the Authenticate flow).
    pub supports_mcp_auth: bool,
    /// Whether this CLI can be handed an opening prompt when the run is created.
    /// False means a dispatched issue or review starts the agent promptless.
    pub accepts_prompt: bool,
}

/// All built-in agent profiles Agency ships recipes for.
pub fn builtins() -> &'static [CatalogEntry] {
    // cursor/hermes/gemini/kimi/crush are id-keyed (not cwd-keyed) so they
    // start fresh rather than risk resuming the wrong global session. Loop
    // recipes are the agents' headless one-shot modes; agents without a known
    // headless mode get None and simply can't loop. claude gets acceptEdits so
    // unattended attempts don't die on the first file-edit permission prompt.
    //
    // `prompt` is read off each CLI's own `--help` and, where the CLI was
    // installed to check, a live launch. Marked "unverified" where neither was
    // possible; those keep the positional default they have always had.
    static LIST: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        vec![
            CatalogEntry {
                id: "claude",
                command: "claude",
                // `claude [options] [command] [prompt]`.
                prompt: PromptDelivery::Positional,
                resume_args: Some(vec!["--continue".into()]),
                loop_args: Some(vec![
                    "-p".into(),
                    "{{prompt}}".into(),
                    "--permission-mode".into(),
                    "acceptEdits".into(),
                ]),
            },
            CatalogEntry {
                id: "codex",
                command: "codex",
                // Unverified (not installed here); `codex [PROMPT]` per its docs.
                prompt: PromptDelivery::Positional,
                resume_args: Some(vec!["resume".into(), "--last".into()]),
                loop_args: Some(vec![
                    "exec".into(),
                    "--full-auto".into(),
                    "{{prompt}}".into(),
                ]),
            },
            CatalogEntry {
                id: "pi",
                command: "pi",
                // `pi [options] [@files...] [messages...]`.
                prompt: PromptDelivery::Positional,
                resume_args: Some(vec!["--continue".into()]),
                loop_args: None,
            },
            CatalogEntry {
                id: "opencode",
                command: "opencode",
                // opencode's positional is `[project]`, "path to start opencode
                // in" — a prompt there is read as a directory. The TUI takes the
                // prompt through `--prompt`; `run` is the headless shape below.
                prompt: PromptDelivery::Args(&["--prompt", "{{prompt}}"]),
                resume_args: Some(vec!["--continue".into()]),
                loop_args: Some(vec!["run".into(), "{{prompt}}".into()]),
            },
            CatalogEntry {
                id: "copilot",
                command: "copilot",
                // AGE-79: Copilot has no positional prompt at all. It exits with
                // "Invalid command format. Did you mean: copilot -i ..." — which
                // is the recipe: `-i, --interactive <prompt>`, "start interactive
                // mode and automatically execute this prompt".
                prompt: PromptDelivery::Args(&["-i", "{{prompt}}"]),
                resume_args: Some(vec!["--continue".into()]),
                // `-p/--prompt` is the non-interactive mode, and `--allow-all-tools`
                // is documented as required for it. Verified against 1.0.78 in a
                // fresh, untrusted git worktree: it edits files there and exits,
                // with no folder-trust prompt to hang a headless attempt.
                loop_args: Some(vec![
                    "-p".into(),
                    "{{prompt}}".into(),
                    "--allow-all-tools".into(),
                ]),
            },
            CatalogEntry {
                id: "cursor",
                command: "cursor-agent",
                // `agent [options] [command] [prompt...]`.
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: Some(vec!["-p".into(), "{{prompt}}".into()]),
            },
            CatalogEntry {
                id: "hermes",
                command: "hermes",
                // Unverified (the local install is broken).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "gemini",
                command: "gemini",
                // Unverified (not installed here).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "kimi",
                command: "kimi",
                // Unverified (not installed here).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "crush",
                command: "crush",
                // `crush [command] [--flags]`, and a prompt is read as a command:
                // `crush "say hi"` fails with 'Unknown command "say hi"'. None of
                // its flags carry an opening prompt, so there is nothing to hand
                // it; `crush run <prompt>` is headless-only. See AGE-80.
                prompt: PromptDelivery::Unsupported,
                resume_args: None,
                loop_args: None,
            },
        ]
    })
    .as_slice()
}

/// How `agent`'s CLI takes an opening prompt. Unknown ids (custom profiles) keep
/// the trailing-positional default, which is what every profile got before the
/// catalog said anything about it.
pub fn prompt_delivery(agent: &str) -> PromptDelivery {
    find(agent).map(|e| e.prompt).unwrap_or(PromptDelivery::Positional)
}

/// Look up a catalog entry by id.
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    builtins().iter().find(|e| e.id == id)
}

/// Build an `AgentProfile` from a catalog entry (empty args/env).
pub fn profile_for(entry: &CatalogEntry) -> AgentProfile {
    AgentProfile {
        name: entry.id.to_string(),
        command: entry.command.to_string(),
        args: vec![],
        env: vec![],
        resume_args: entry.resume_args.clone(),
        loop_args: entry.loop_args.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prompt recipe with nowhere to put the prompt would launch the agent with
    /// the ask silently missing, which is the failure AGE-79 was filed for.
    #[test]
    fn every_prompt_recipe_places_the_prompt() {
        for entry in builtins() {
            if let PromptDelivery::Args(recipe) = entry.prompt {
                assert!(
                    recipe.iter().any(|a| a.contains("{{prompt}}")),
                    "{}'s prompt recipe has no {{{{prompt}}}} token: {recipe:?}",
                    entry.id
                );
            }
        }
    }

    /// Loop recipes are rendered by the same token substitution, and a recipe
    /// without one falls back to appending the prompt positionally — which the
    /// CLIs that reject positionals must never rely on.
    #[test]
    fn loop_recipes_place_the_prompt_where_a_positional_would_be_refused() {
        for entry in builtins() {
            let Some(recipe) = &entry.loop_args else { continue };
            if entry.prompt == PromptDelivery::Positional {
                continue;
            }
            assert!(
                recipe.iter().any(|a| a.contains("{{prompt}}")),
                "{} takes no positional prompt, so its loop recipe needs the token: {recipe:?}",
                entry.id
            );
        }
    }

    #[test]
    fn prompt_delivery_falls_back_to_positional_for_custom_profiles() {
        assert_eq!(prompt_delivery("claude"), PromptDelivery::Positional);
        assert_eq!(prompt_delivery("copilot"), PromptDelivery::Args(&["-i", "{{prompt}}"]));
        assert_eq!(prompt_delivery("crush"), PromptDelivery::Unsupported);
        assert_eq!(prompt_delivery("my-own-agent"), PromptDelivery::Positional);
    }
}
