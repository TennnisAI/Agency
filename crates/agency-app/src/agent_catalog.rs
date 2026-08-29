//! Built-in agent catalog: known CLI profiles with prompt/resume/loop recipes.
//!
//! Distinct from enabled profiles in the DB — the catalog is the menu of
//! agents Agency knows how to configure; users enable a subset via onboarding
//! or Settings. `shell` is seeded separately and is not part of this catalog.

use crate::model_probe::{ListFormat, ListScope, ModelListing};
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
    /// Not an argv recipe: after the GUI server answers, Agency adopts the
    /// worktree over the agent's HTTP API and queues the prompt on a session
    /// there (`workspace.create` / `session.prompt`). Putting the text on the
    /// command line would be read as an app selection and the server would die.
    AfterGuiReady,
}

/// An agent whose interactive surface is a browser GUI served from the
/// worktree, not a TUI in the pane. The pane still shows the server's own log;
/// the GUI is what the user actually works in, so the focus view renders it
/// beside the pane and offers it to the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebUi {
    /// argv prefix that boots the GUI server. Prepended ahead of the profile's
    /// own arguments: for `dsh` the launcher hands everything after its own
    /// flags to the booted app, so the `web` selector has to come first or a
    /// user argument would be read as the app selection.
    pub args: &'static [&'static str],
    /// The flag recipe that pins the GUI to the run's own port, `{{port}}`
    /// marking where the number goes. Kept apart from `args` so a run with no
    /// port block still boots (on the CLI's default port) instead of launching
    /// with a dangling flag.
    pub port_args: &'static [&'static str],
}

/// How an agent's CLI takes a *model* choice on the command line. Stated per
/// entry and never guessed: unlike a missing prompt, a wrong model flag is a
/// loud failure — the CLI rejects the unknown option and the session dies
/// before the agent ever starts. So an agent whose flag we have not read off
/// its own `--help` (or its published reference) gets `Unsupported`, and
/// Agency simply offers no model picker for it rather than a coin flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDelivery {
    /// A flag recipe, `{{model}}` marking where the model id goes.
    Flag(&'static [&'static str]),
    /// This CLI has no launch-time model flag: the model is chosen inside the
    /// CLI (its config file or a slash command), so Agency cannot set it.
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
    /// How a chosen model reaches this CLI (see [`ModelDelivery`]).
    pub model: ModelDelivery,
    /// Model ids offered in the picker. Deliberately only ids the vendor
    /// publishes as *stable aliases* ("opus", "flash") rather than dated model
    /// names, which go stale between Agency releases and would leave the menu
    /// offering models that no longer exist. Any other id is typed in.
    pub models: &'static [&'static str],
    /// How this CLI can be asked what it can run: the arguments that make it
    /// list its models, and the shape of what it prints (see
    /// [`crate::model_probe`]). None where the CLI has no such command, which
    /// is most of them. Read off each CLI's own `--help` and verified against a
    /// live run, exactly like the recipes above: a listing whose format we
    /// guessed would quietly offer the picker ids that are not ids.
    pub list_models: Option<ModelListing>,
    /// Set when this agent's interactive surface is a browser GUI served from
    /// the worktree (see [`WebUi`]). None for every terminal agent.
    pub web_ui: Option<WebUi>,
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
    /// Whether this agent's interactive surface is a browser GUI Agency serves
    /// beside the pane (see [`WebUi`]), so onboarding and Settings can say so.
    pub serves_web_ui: bool,
}

/// All built-in agent profiles Agency ships recipes for.
pub fn builtins() -> &'static [CatalogEntry] {
    // cursor/hermes/gemini/kimi are id-keyed (not cwd-keyed) so they
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
                // `--model <model>`: an alias for the latest of a family
                // ('opus', 'sonnet') or a full name ('claude-opus-5'). Read off
                // `claude --help`.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &["fable", "opus", "sonnet", "haiku"],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "codex",
                command: "codex",
                // Unverified (not installed here); `codex [PROMPT]` per its docs.
                prompt: PromptDelivery::Positional,
                resume_args: Some(vec!["resume".into(), "--last".into()]),
                loop_args: Some(vec!["exec".into(), "--full-auto".into(), "{{prompt}}".into()]),
                // `-m, --model <MODEL>`, per Codex's CLI reference
                // (`codex exec -m gpt-5.6 "…"`). Codex's model names are dated
                // and turn over fast, so none are listed: they would be stale
                // by the next release.
                model: ModelDelivery::Flag(&["-m", "{{model}}"]),
                models: &[],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "pi",
                command: "pi",
                // `pi [options] [@files...] [messages...]`.
                prompt: PromptDelivery::Positional,
                resume_args: Some(vec!["--continue".into()]),
                loop_args: None,
                // `--model <pattern>`, which takes "provider/id" and an
                // optional ":<thinking>" suffix ("sonnet:high"). Read off
                // `pi --help`.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &[],
                // Verified against 2026-08-20's build: a space-aligned table
                // whose `provider` and `model` columns compose the
                // `provider/id` pattern `--model` documents.
                list_models: Some(ModelListing {
                    args: &["--list-models"],
                    format: ListFormat::ProviderTable,
                    // pi's providers are configured once for the user
                    // (`~/.pi/`), so the answer is the same in every project.
                    scope: ListScope::User,
                }),
                web_ui: None,
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
                // `-m, --model`, "model to use in the format of
                // provider/model". Read off `opencode --help`.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &[],
                // Verified against 2026-08-20's build: one bare
                // `provider/model` per line, which is exactly what `--model`
                // takes.
                list_models: Some(ModelListing {
                    args: &["models"],
                    format: ListFormat::Ids,
                    // opencode merges an `opencode.json` from the directory it
                    // is run in over the user's own, so a provider configured
                    // for one project only lists there (AGE-135).
                    scope: ListScope::Project,
                }),
                web_ui: None,
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
                loop_args: Some(vec!["-p".into(), "{{prompt}}".into(), "--allow-all-tools".into()]),
                // `--model`, per GitHub's Copilot CLI reference (the
                // command-line half of its `/model` command). Its model names
                // are dated, so none are listed here.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &[],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "cursor",
                command: "cursor-agent",
                // `agent [options] [command] [prompt...]`.
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: Some(vec!["-p".into(), "{{prompt}}".into()]),
                // `--model <model>`, "Model to use (e.g., gpt-5,
                // sonnet-4-thinking)". Read off `cursor-agent --help`.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &[],
                // Verified against 2026-08-20's build: an "Available models"
                // title, then `<id> - <description>` rows whose id is the one
                // `--model` takes.
                list_models: Some(ModelListing {
                    args: &["--list-models"],
                    format: ListFormat::IdsWithDescriptions,
                    // cursor-agent's models come from the signed-in account,
                    // not from anything beside the code.
                    scope: ListScope::User,
                }),
                web_ui: None,
            },
            CatalogEntry {
                id: "hermes",
                command: "hermes",
                // Unverified (the local install is broken).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
                // Unverified for the same reason as the prompt recipe: the
                // local install is broken, so its `--help` cannot be read. No
                // picker rather than a guessed flag that would kill the launch.
                model: ModelDelivery::Unsupported,
                models: &[],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "gemini",
                command: "gemini",
                // Unverified (not installed here).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
                // `-m, --model`, whose aliases ('pro', 'flash') are the
                // stable names Google documents; the dated ids behind them are
                // not. Per the Gemini CLI options reference.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &["auto", "pro", "flash", "flash-lite"],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "kimi",
                command: "kimi",
                // Unverified (not installed here).
                prompt: PromptDelivery::Positional,
                resume_args: None,
                loop_args: None,
                // `-m, --model`, "specify a model alias for this launch",
                // per the kimi command reference. The aliases are per-provider,
                // so none are listed.
                model: ModelDelivery::Flag(&["--model", "{{model}}"]),
                models: &[],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "crush",
                command: "crush",
                // `crush [command] [--flags]`, and a prompt is read as a command:
                // `crush "say hi"` fails with 'Unknown command "say hi"'. None of
                // its flags carry an opening prompt, so there is nothing to hand
                // it; `crush run <prompt>` is headless-only. See AGE-80.
                prompt: PromptDelivery::Unsupported,
                // Sessions live in `.crush/crush.db` under the cwd (crush drops a
                // `*` .gitignore beside it), so a worktree only ever has its own
                // sessions and --continue can't reach another project's.
                resume_args: Some(vec!["--continue".into()]),
                // `run` is the headless one-shot AGE-80 spotted but never wired
                // up. It auto-approves the session's permissions itself (crush's
                // app.RunNonInteractive), which is why it rejects the TUI-only
                // --yolo; --quiet drops the spinner so the pane keeps a readable
                // transcript instead of redraw noise. Verified against v0.51.2.
                loop_args: Some(vec!["run".into(), "{{prompt}}".into(), "--quiet".into()]),
                // Verified against v0.51.2: `crush --help` has no model flag
                // at all. The model is picked inside the TUI or set in crush's
                // own config (`crush models` lists what it has), so there is
                // nothing Agency can pass at launch and no picker to hint at.
                model: ModelDelivery::Unsupported,
                models: &[],
                list_models: None,
                web_ui: None,
            },
            CatalogEntry {
                id: "dsh",
                command: "dsh",
                // Read off the launcher source at dsh-0.1.0-rc.7, re-checked
                // against 0.1.1-rc.2: `dsh web`'s flag family is still `--host`,
                // `--port`, `--trusted-host`, `--no-open`. The opening ask is
                // `session.prompt` after `workspace.create` adopts the cwd —
                // see `web_ui.rs`. A positional here is read as an app selection.
                prompt: PromptDelivery::AfterGuiReady,
                // Relaunching `dsh web` in the same worktree *is* the resume:
                // sessions persist under $DSH_HOME keyed to the workspace
                // directory, and the GUI reopens them itself. A resume recipe
                // would be arguments the launcher reads as an app selection.
                resume_args: None,
                // The headless profile's own contract: one task as the
                // positional argument, final answer on stdout, exit 0 on
                // completion and 1 on abort or error — and it opens no
                // listening port, so loop attempts never fight the GUI's.
                loop_args: Some(vec!["--profile".into(), "headless".into(), "{{prompt}}".into()]),
                // No launch-time model flag anywhere in the shipped apps: the
                // model is chosen in the GUI (Settings > Models) or in dsh's
                // own profile config, so there is nothing Agency can pass.
                model: ModelDelivery::Unsupported,
                models: &[],
                list_models: None,
                // `dsh web` serves its GUI on 127.0.0.1 (it refuses 0.0.0.0
                // with a usage error) and by default opens the user's browser
                // once the tree settles. --no-open keeps that handoff with
                // Agency, which renders the GUI in the run's own pane instead;
                // Open in browser stays one click, chosen, not sprung.
                web_ui: Some(WebUi {
                    // `web --patch …` is added at launch (see with_web_ui): their
                    // layout store always starts with the conversation sidebar
                    // open, and there is no flag for a collapsed default.
                    args: &["web", "--no-open"],
                    port_args: &["--port", "{{port}}"],
                }),
            },
        ]
    })
    .as_slice()
}

/// The web-GUI recipe for `agent`, if its interactive surface is a browser GUI
/// (see [`WebUi`]). None for every terminal agent and for custom profiles: we
/// have not read their CLIs, and a guessed server recipe would replace the
/// agent the user configured with a launch that does something else entirely.
pub fn web_ui(agent: &str) -> Option<WebUi> {
    find(agent)?.web_ui
}

/// How `agent`'s CLI takes an opening prompt. Unknown ids (custom profiles) keep
/// the trailing-positional default, which is what every profile got before the
/// catalog said anything about it.
pub fn prompt_delivery(agent: &str) -> PromptDelivery {
    find(agent).map(|e| e.prompt).unwrap_or(PromptDelivery::Positional)
}

/// How `agent`'s CLI takes a model choice. Unknown ids (custom profiles) get
/// `Unsupported`: we have not read their `--help`, and an invented flag would
/// stop the CLI from starting at all. A custom profile can still pin a model
/// by putting the flag in the profile's own arguments.
pub fn model_delivery(agent: &str) -> ModelDelivery {
    find(agent).map(|e| e.model).unwrap_or(ModelDelivery::Unsupported)
}

/// How `agent`'s CLI can be asked what models it has, if it can be asked at
/// all. Unknown ids (custom profiles) get None: we have not read their `--help`
/// either, and running an unknown binary with an invented flag to see what
/// falls out is not a probe, it is a guess with a process behind it.
pub fn model_listing(agent: &str) -> Option<ModelListing> {
    find(agent)?.list_models
}

/// The listing command as the user would type it (`opencode models`), for the
/// picker's hint and as the picker's signal that this agent can be asked at
/// all. Composed from the catalog's own binary and arguments rather than
/// written out a second time; a profile pointing the agent at some other binary
/// probes that one instead, and the probe's own failure message names it.
pub fn list_command(agent: &str) -> Option<String> {
    let entry = find(agent)?;
    let listing = entry.list_models?;
    Some(
        std::iter::once(entry.command)
            .chain(listing.args.iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Whether Agency can set `agent`'s model at launch.
pub fn supports_model(agent: &str) -> bool {
    model_delivery(agent) != ModelDelivery::Unsupported
}

/// The arguments that pin `model` for `agent`, per that CLI's own recipe.
/// Empty when no model was chosen, or when this CLI has no way to be told one
/// — the run then launches on whatever the agent itself defaults to.
pub fn model_args(agent: &str, model: Option<&str>) -> Vec<String> {
    let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        return Vec::new();
    };
    match model_delivery(agent) {
        ModelDelivery::Flag(recipe) => {
            recipe.iter().map(|a| a.replace("{{model}}", model)).collect()
        }
        ModelDelivery::Unsupported => {
            log::warn!(
                "{agent} has no launch-time model flag, so the run's model ({model}) cannot be \
                 applied; it starts on whatever that CLI is configured to use"
            );
            Vec::new()
        }
    }
}

/// Longest model id Agency will pass on. Nothing real comes close; the cap is
/// only here so a paste accident cannot become an unreadable command line.
const MAX_MODEL_LEN: usize = 128;

/// Validate a model id on its way in from the UI, before it is stored on a run
/// and spliced into an agent's argv. Allowlisted characters, not a denylist:
/// the accepted set covers every shape the supported CLIs take (`opus`,
/// `claude-opus-5`, `anthropic/claude-sonnet-5`, `sonnet:high`,
/// `github-copilot/gpt-5.1`) and nothing else. A leading `-` is refused
/// separately because such an id would be read by the CLI as another flag
/// rather than as the model.
///
/// `Ok(None)` means "no model": blank input is the agent's own default, which
/// is a real choice and not an error.
pub fn sanitize_model(raw: &str) -> Result<Option<String>, String> {
    let model = raw.trim();
    if model.is_empty() {
        return Ok(None);
    }
    if model.len() > MAX_MODEL_LEN {
        return Err(format!("model id is too long (max {MAX_MODEL_LEN} characters)"));
    }
    if model.starts_with('-') {
        return Err(format!("'{model}' starts with '-', which the agent would read as a flag"));
    }
    if !model.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@+".contains(c)) {
        return Err(format!(
            "'{model}' is not a model id — letters, digits and - _ . / : @ + only"
        ));
    }
    Ok(Some(model.to_string()))
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

    /// The point of a catalog entry over a hand-rolled custom profile is the
    /// recipes it carries, so an entry offering none of prompt/resume/loop is
    /// just a command name and belongs nowhere near onboarding — the state
    /// AGE-81 was filed over, after crush's recipes were left unfilled rather
    /// than its CLI actually lacking them.
    #[test]
    fn every_catalog_entry_carries_at_least_one_recipe() {
        for entry in builtins() {
            assert!(
                entry.prompt != PromptDelivery::Unsupported
                    || entry.resume_args.is_some()
                    || entry.loop_args.is_some(),
                "{} takes no prompt, resume or loop recipe — read its --help and wire up \
                 whatever it does support, or drop it from the catalog",
                entry.id
            );
        }
    }

    /// A model recipe with nowhere to put the model would launch the agent
    /// with a dangling flag, which every CLI here rejects outright.
    #[test]
    fn every_model_recipe_places_the_model() {
        for entry in builtins() {
            if let ModelDelivery::Flag(recipe) = entry.model {
                assert!(
                    recipe.iter().any(|a| a.contains("{{model}}")),
                    "{}'s model recipe has no {{{{model}}}} token: {recipe:?}",
                    entry.id
                );
            }
        }
    }

    /// Suggested ids are offered as one-click choices, so an agent that cannot
    /// be told a model at all must not suggest any.
    #[test]
    fn only_model_capable_agents_suggest_models() {
        for entry in builtins() {
            if entry.model == ModelDelivery::Unsupported {
                assert!(
                    entry.models.is_empty() && entry.list_models.is_none(),
                    "{} has no model flag, so no picker is shown for it and neither its \
                     suggestions nor its list-models hint can ever be reached",
                    entry.id
                );
            }
        }
    }

    /// The hint under the picker's field is composed from the same two pieces
    /// the probe runs, rather than written out a second time and left to drift.
    #[test]
    fn the_listing_hint_reads_as_the_user_would_type_it() {
        assert_eq!(list_command("opencode").as_deref(), Some("opencode models"));
        assert_eq!(list_command("pi").as_deref(), Some("pi --list-models"));
        // The binary, not the catalog id: cursor's CLI is `cursor-agent`.
        assert_eq!(list_command("cursor").as_deref(), Some("cursor-agent --list-models"));
        assert_eq!(list_command("claude"), None);
        assert_eq!(list_command("my-own-agent"), None);
        assert_eq!(model_listing("my-own-agent"), None);
    }

    /// A listing with no arguments would run the agent's bare CLI — launching
    /// an interactive session behind a menu, not asking it anything.
    #[test]
    fn every_listing_carries_arguments() {
        for entry in builtins() {
            if let Some(listing) = entry.list_models {
                assert!(!listing.args.is_empty(), "{}'s listing has no arguments", entry.id);
            }
        }
    }

    /// Suggestions go straight onto a command line, so they must survive the
    /// same validation a typed id does.
    #[test]
    fn suggested_models_are_valid_ids() {
        for entry in builtins() {
            for m in entry.models {
                assert_eq!(sanitize_model(m), Ok(Some((*m).to_string())), "{}: {m}", entry.id);
            }
        }
    }

    #[test]
    fn model_args_render_the_recipe_and_stay_empty_without_a_choice() {
        assert_eq!(model_args("claude", Some("opus")), vec!["--model", "opus"]);
        assert_eq!(model_args("codex", Some("gpt-5.6")), vec!["-m", "gpt-5.6"]);
        assert!(model_args("claude", None).is_empty());
        // Blank is the same as unset, not a model called "".
        assert!(model_args("claude", Some("  ")).is_empty());
        // No flag to pass it through, so nothing is invented.
        assert!(model_args("crush", Some("anything")).is_empty());
        assert!(model_args("my-own-agent", Some("anything")).is_empty());
    }

    #[test]
    fn sanitize_model_accepts_the_shapes_the_supported_clis_take() {
        for id in [
            "opus",
            "claude-opus-5",
            "anthropic/claude-sonnet-5",
            "sonnet:high",
            "github-copilot/gpt-5.1",
            "flash-lite",
        ] {
            assert_eq!(sanitize_model(id), Ok(Some(id.to_string())), "{id}");
        }
        assert_eq!(sanitize_model("  opus  "), Ok(Some("opus".to_string())));
        assert_eq!(sanitize_model(""), Ok(None));
        assert_eq!(sanitize_model("   "), Ok(None));
    }

    #[test]
    fn sanitize_model_refuses_ids_that_would_not_be_read_as_a_model() {
        // Would land in argv as another flag.
        assert!(sanitize_model("--dangerously-skip-permissions").is_err());
        // Shell metacharacters are quoted before they reach a shell, but an id
        // carrying them is not a model id in the first place.
        assert!(sanitize_model("opus; rm -rf /").is_err());
        assert!(sanitize_model("opus $(id)").is_err());
        assert!(sanitize_model(&"a".repeat(MAX_MODEL_LEN + 1)).is_err());
    }

    /// The port recipe is rendered by token substitution like the prompt and
    /// model recipes; a recipe without the token would pin every GUI to one
    /// port and two concurrent runs would fight over it.
    #[test]
    fn every_web_ui_places_the_port_and_boots_something() {
        for entry in builtins() {
            let Some(web) = entry.web_ui else { continue };
            assert!(
                !web.args.is_empty(),
                "{}'s web recipe boots nothing: the bare CLI is not a GUI server",
                entry.id
            );
            assert!(
                web.port_args.iter().any(|a| a.contains("{{port}}")),
                "{}'s port recipe has no {{{{port}}}} token: {:?}",
                entry.id,
                web.port_args
            );
        }
    }

    /// A custom profile gets no web recipe: we have not read its CLI, and a
    /// guessed server launch replaces whatever the user configured.
    #[test]
    fn web_ui_is_declared_not_guessed() {
        assert!(web_ui("dsh").is_some());
        assert!(web_ui("claude").is_none());
        assert!(web_ui("my-own-agent").is_none());
    }

    #[test]
    fn model_delivery_falls_back_to_unsupported_for_custom_profiles() {
        assert!(!supports_model("my-own-agent"));
        assert!(supports_model("claude"));
        assert_eq!(model_delivery("crush"), ModelDelivery::Unsupported);
    }

    #[test]
    fn prompt_delivery_falls_back_to_positional_for_custom_profiles() {
        assert_eq!(prompt_delivery("claude"), PromptDelivery::Positional);
        assert_eq!(prompt_delivery("copilot"), PromptDelivery::Args(&["-i", "{{prompt}}"]));
        assert_eq!(prompt_delivery("crush"), PromptDelivery::Unsupported);
        assert_eq!(prompt_delivery("dsh"), PromptDelivery::AfterGuiReady);
        assert_eq!(prompt_delivery("my-own-agent"), PromptDelivery::Positional);
    }
}
