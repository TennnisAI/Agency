//! Asking an agent's own CLI which models it has.
//!
//! The picker (AGE-114) offers only the stable vendor aliases, because a
//! hardcoded list of dated model names goes stale between releases. That leaves
//! Codex, Cursor, opencode, pi, Copilot and Kimi with nothing to offer, so the
//! first use of any model there means knowing its id and typing it. Four of
//! these CLIs will say what they have if asked, and this is the asking: run the
//! listing command, parse its output, hand the picker a list of ids.
//!
//! On demand only, never at launch. Every one of these commands is a network
//! round trip: measured on 2026-08-20, `opencode models` 0.95s,
//! `pi --list-models` 0.95s, `cursor-agent --list-models` 1.35s,
//! `crush models` 1.36s. The caller caches the answer for the app session, and
//! nothing runs until a picker is actually opened.
//!
//! [`parse`] is the part with the judgement in it and is where the tests are;
//! [`probe`] is only the spawn around it.

use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The shape of a listing command's stdout. Named after what the output looks
/// like rather than after the CLI that prints it, because two of them already
/// print the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    /// One bare id per line and nothing else, `provider/model` in both cases
    /// seen: `opencode models`, `crush models`.
    Ids,
    /// A whitespace-aligned table under a `provider  model  context  …` header.
    /// The id `pi --model` takes is the first two columns joined by a slash
    /// ("anthropic  claude-opus-4-5" is `anthropic/claude-opus-4-5`), which is
    /// the form its own `--help` documents: `pi --list-models`.
    ProviderTable,
    /// `<id> - <human description>` lines under a title:
    /// `cursor-agent --list-models`.
    IdsWithDescriptions,
}

/// Whose configuration a listing command reads, and so where it has to be run
/// from for its answer to be the true one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListScope {
    /// Only user-level configuration, so one answer serves every project and
    /// the directory the command runs in does not change it.
    User,
    /// Configuration from the directory it is run in as well as the user's:
    /// opencode reads an `opencode.json` beside the code, crush a `.crush/`.
    /// Asked from anywhere else, a provider configured for one project is
    /// simply not in the list (AGE-135).
    Project,
}

/// How one CLI is asked what it can run: the arguments that make it list its
/// models, the shape of what it prints back, and whose configuration the answer
/// comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelListing {
    /// Arguments after the agent's own command (`pi` + `--list-models`).
    pub args: &'static [&'static str],
    pub format: ListFormat,
    /// Whether the answer depends on where the command is run (see
    /// [`ListScope`]). Read off each CLI's own configuration documentation, and
    /// wrong in the safe direction if it is wrong: a `User` marking on a
    /// project-scoped CLI hides that project's own providers, which is the bug
    /// this exists to fix, while a `Project` marking on a user-scoped one only
    /// costs a probe per project.
    pub scope: ListScope,
}

/// Most models any of these CLIs is allowed to report. `cursor-agent` already
/// lists ~90 and opencode ~55, so this is not a limit anyone reaches; it is
/// only here so a CLI that starts streaming something other than a model list
/// cannot fill a menu (or the IPC payload) without bound.
const MAX_MODELS: usize = 500;

/// How long a listing command gets before it is killed, against the ~1s these
/// take when they answer. Generous because the picker stays usable while it
/// runs and a slow network is not a failure; short enough that a CLI sitting on
/// an interactive prompt we cannot see eventually reports as one.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Model ids from a listing command's stdout, in the order the CLI printed
/// them (which is its own ranking, and better than alphabetical).
///
/// Every candidate goes through [`crate::agent_catalog::sanitize_model`], which
/// is what drops the banners, headers, blank lines and prose these commands mix
/// in with the ids: a real id has no spaces, and nothing that fails validation
/// could be passed to the CLI anyway.
pub fn parse(format: ListFormat, stdout: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Only meaningful for `ProviderTable`: a row of it is two bare words, which
    // is a shape ordinary prose also has ("no models configured" would
    // otherwise be offered as the model `no/models`). So the header is what
    // says the table has started, and its column count is what a row has to
    // match to be one. `None` until the header is seen; nothing before it is
    // read as a model.
    let mut columns: Option<usize> = None;
    for line in stdout.lines() {
        let line = strip_ansi(line);
        let line = line.trim();
        let candidate = match format {
            ListFormat::Ids => line.to_string(),
            ListFormat::ProviderTable => {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.first() == Some(&"provider") && cols.get(1) == Some(&"model") {
                    columns = Some(cols.len());
                    continue;
                }
                if columns != Some(cols.len()) {
                    continue;
                }
                format!("{}/{}", cols[0], cols[1])
            }
            // The id is everything before the description; a line with no
            // description at all is taken whole.
            ListFormat::IdsWithDescriptions => {
                line.split_once(" - ").map_or(line, |(id, _)| id).trim().to_string()
            }
        };
        let Ok(Some(model)) = crate::agent_catalog::sanitize_model(&candidate) else { continue };
        if !out.contains(&model) {
            out.push(model);
        }
        if out.len() >= MAX_MODELS {
            break;
        }
    }
    out
}

/// Drop ANSI escape sequences. These are TUI CLIs, and although all four print
/// clean text into a pipe today, a colored id would otherwise fail validation
/// and silently disappear from the menu rather than showing up wrong.
pub(crate) fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // CSI (`ESC [ … final`) is the only form these emit; anything else is
        // dropped along with the single character that follows it.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Run `command`'s listing arguments and return what it says it can run.
///
/// `cwd` is where the CLI is run from, and it matters: opencode and crush both
/// read project-local config, so the answer is only as global as the directory.
/// The caller picks it from the listing's [`ListScope`] — the project being
/// worked in for a CLI that reads config from it, the user's home for one that
/// does not, that being a neutral place where a CLI finds its own user-level
/// configuration and no project's.
pub fn probe(
    command: &str,
    listing: ModelListing,
    cwd: Option<&std::path::Path>,
) -> Result<Vec<String>> {
    let display = format!("{command} {}", listing.args.join(" "));
    let mut cmd = Command::new(command);
    cmd.args(listing.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let mut child = cmd.spawn().with_context(|| format!("could not run `{display}`"))?;

    // Both pipes are drained on their own threads. Polling `try_wait` without
    // reading them deadlocks the moment a listing outgrows the 64K pipe buffer,
    // and pi's table is already a third of that.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let deadline = Instant::now() + TIMEOUT;
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(e).with_context(|| format!("waiting on `{display}`")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();

    let Some(status) = finished else {
        bail!("`{display}` did not answer within {}s", TIMEOUT.as_secs());
    };
    let models = parse(listing.format, &stdout);
    if models.is_empty() {
        // The exit code is not the test. `opencode models` exits 0 and prints
        // nothing when no provider is configured, and what went wrong is in the
        // text, so the message the user sees is the CLI's own last words.
        let why = tail(&stderr, &stdout)
            .unwrap_or_else(|| format!("it exited {} with nothing to parse", code(status)));
        bail!("`{display}` listed no models: {why}");
    }
    Ok(models)
}

pub(crate) fn drain(
    pipe: Option<impl std::io::Read + Send + 'static>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn code(status: std::process::ExitStatus) -> String {
    status.code().map_or_else(|| "on a signal".into(), |c| c.to_string())
}

/// The last couple of non-empty lines of whichever stream said anything,
/// stderr first: enough to carry "not logged in" back to the picker without
/// pasting a stack trace into a menu.
fn tail(stderr: &str, stdout: &str) -> Option<String> {
    const LINES: usize = 2;
    [stderr, stdout].into_iter().find_map(|text| {
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(LINES)..].join("; ");
        (!tail.is_empty()).then_some(tail)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `opencode models` (v1.0, 2026-08-20). `crush models`
    /// prints the same shape.
    const OPENCODE: &str = "\
opencode/big-pickle
opencode/deepseek-v4-flash-free
openai/gpt-5.6
openai/gpt-5.6-pro
anthropic/claude-opus-5
";

    /// Captured from `pi --list-models`, whose columns are space-padded to
    /// align and whose first row is a header.
    const PI: &str = "\
provider   model                       context  max-out  thinking  images
anthropic  claude-opus-4-5             200K     64K      yes       yes
anthropic  claude-sonnet-5             1M       128K     yes       yes
openai     gpt-5.6                     400K     128K     yes       yes
google     gemini-3.7-pro              1.0M     64K      yes       yes
";

    /// Captured from `cursor-agent --list-models`: a title, a blank line, then
    /// `<id> - <description>` rows.
    const CURSOR: &str = "\
Available models

auto - Auto (current, default)
gpt-5.3-codex-high - Codex 5.3 High
claude-opus-5-thinking-high - Claude Opus 5 1M Thinking
claude-fable-5-thinking-high - Claude Fable 5 1M Thinking (NO ZDR)
composer-2.5 - Composer 2.5
";

    #[test]
    fn ids_are_taken_a_line_at_a_time() {
        assert_eq!(
            parse(ListFormat::Ids, OPENCODE),
            [
                "opencode/big-pickle",
                "opencode/deepseek-v4-flash-free",
                "openai/gpt-5.6",
                "openai/gpt-5.6-pro",
                "anthropic/claude-opus-5"
            ]
        );
    }

    /// The table's first two columns compose the `provider/id` pattern pi's
    /// own `--model` takes; the header row is not a model.
    #[test]
    fn the_provider_table_composes_ids_and_drops_its_header() {
        assert_eq!(
            parse(ListFormat::ProviderTable, PI),
            [
                "anthropic/claude-opus-4-5",
                "anthropic/claude-sonnet-5",
                "openai/gpt-5.6",
                "google/gemini-3.7-pro"
            ]
        );
    }

    /// A two-word line looks exactly like a table row, so the header is what
    /// starts the table and its column count is what a row has to match.
    /// Without this, a complaint printed above the table becomes a model
    /// called `not/logged`.
    #[test]
    fn the_provider_table_reads_nothing_outside_the_table() {
        let framed = "\
not logged in to google
provider   model     context  max-out  thinking  images
openai     gpt-5.6   400K     128K     yes       yes
1 model shown
";
        assert_eq!(parse(ListFormat::ProviderTable, framed), ["openai/gpt-5.6"]);
    }

    /// The description is prose and never reaches the command line; the "Available
    /// models" title has a space in it, which is what disqualifies it as an id.
    #[test]
    fn described_ids_keep_only_the_id() {
        assert_eq!(
            parse(ListFormat::IdsWithDescriptions, CURSOR),
            [
                "auto",
                "gpt-5.3-codex-high",
                "claude-opus-5-thinking-high",
                "claude-fable-5-thinking-high",
                "composer-2.5"
            ]
        );
    }

    /// Whatever a CLI prints alongside its list, nothing reaches the menu that
    /// could not be handed back to that CLI as a model.
    #[test]
    fn banners_prompts_and_prose_are_not_models() {
        let noisy = "\
Fetching models…
Available models

  openai/gpt-5.6
-–weird-dash-model
a-very-long-id-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
openai/gpt-5.6
";
        // The indented id survives (trimmed); the ellipsis line and the title
        // are prose; the leading dash would be read as a flag; the 130-char id
        // is past the cap; and the repeat is not offered twice.
        assert_eq!(parse(ListFormat::Ids, noisy), ["openai/gpt-5.6"]);
    }

    /// A colored id would otherwise fail validation and vanish from the menu.
    #[test]
    fn color_codes_do_not_hide_a_model() {
        assert_eq!(parse(ListFormat::Ids, "\x1b[32mopenai/gpt-5.6\x1b[0m\n"), ["openai/gpt-5.6"]);
        assert_eq!(
            parse(ListFormat::IdsWithDescriptions, "\x1b[1mauto\x1b[0m - Auto (default)\n"),
            ["auto"]
        );
    }

    #[test]
    fn nothing_parseable_is_an_empty_list_not_a_guess() {
        assert!(parse(ListFormat::Ids, "").is_empty());
        assert!(parse(ListFormat::Ids, "error: not logged in\n").is_empty());
        assert!(parse(ListFormat::ProviderTable, "no models configured\n").is_empty());
    }

    /// A CLI that cannot even be started is a failure the picker reports, not
    /// an empty list it renders as "this agent has nothing".
    #[test]
    fn a_missing_binary_is_an_error() {
        let listing =
            ModelListing { args: &["models"], format: ListFormat::Ids, scope: ListScope::User };
        let err = probe("agency-no-such-agent-cli", listing, None).unwrap_err().to_string();
        assert!(err.contains("could not run `agency-no-such-agent-cli models`"), "{err}");
    }

    /// Exit 0 with nothing to parse is the "no provider configured" shape, and
    /// the CLI's own words are what the picker shows.
    #[test]
    fn an_empty_listing_carries_the_clis_own_complaint() {
        let listing = ModelListing {
            args: &["-c", "echo 'not logged in' >&2"],
            format: ListFormat::Ids,
            scope: ListScope::User,
        };
        let err = probe("sh", listing, None).unwrap_err().to_string();
        assert!(err.contains("listed no models: not logged in"), "{err}");
    }

    #[test]
    fn a_real_listing_is_parsed_off_the_child() {
        let listing = ModelListing {
            args: &["-c", "printf 'openai/gpt-5.6\\nanthropic/claude-opus-5\\n'"],
            format: ListFormat::Ids,
            scope: ListScope::User,
        };
        assert_eq!(
            probe("sh", listing, None).unwrap(),
            ["openai/gpt-5.6", "anthropic/claude-opus-5"]
        );
    }

    /// The wait loop polls `try_wait` rather than blocking on the child, which
    /// only works because the pipes are drained on their own threads: without
    /// them a listing past the 64K pipe buffer wedges the child in `write` and
    /// this test hangs until the 20s timeout kills it.
    #[test]
    fn a_large_listing_does_not_deadlock_on_a_full_pipe() {
        // 200K of ids, well past the 64K pipe buffer that a non-draining
        // implementation deadlocks on.
        let listing = ModelListing {
            args: &["-c", "for i in $(seq 1 12000); do echo openai/gpt-$i; done"],
            format: ListFormat::Ids,
            scope: ListScope::User,
        };
        let models = probe("sh", listing, None).unwrap();
        assert_eq!(models.len(), MAX_MODELS);
        assert_eq!(models[0], "openai/gpt-1");
    }
}
