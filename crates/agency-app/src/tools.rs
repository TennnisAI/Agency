//! The command-line tools Agency itself leans on, and how to put each one on a
//! machine that lacks it.
//!
//! Observed 2026-09-10 on a fresh Ubuntu desktop: onboarding offered eleven
//! agents and an "Install…" button for each, every install line began with
//! `npm`, and the machine had neither npm nor git. Adding a folder and
//! choosing "Initialize repository" then failed with the bare
//! `No such file or directory (os error 2)`, which is what `Command::new("git")`
//! says when there is no git. Nothing in the app had ever asked whether the
//! tools it shells out to exist. This module is that question, answered once
//! per tool with a recipe the app can run on the user's behalf.
//!
//! Pure: the catalog, the platform-to-recipe table and the npm prefix
//! workaround are plain functions over plain values, so the whole table is
//! unit-tested on every platform, including the ones the tests do not run on.
//! `detect_platform` is the one probe, and it takes its PATH lookup as an
//! argument for the same reason.

use serde::Serialize;

/// A tool Agency needs, in the order the onboarding step lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Projects are repositories; every branch-shaped feature is git.
    Git,
    /// Most of the catalog installs with `npm install -g`.
    Node,
    /// Pull requests and issue import go through the GitHub CLI.
    Gh,
}

pub const TOOLS: [Tool; 3] = [Tool::Git, Tool::Node, Tool::Gh];

impl Tool {
    pub fn id(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Node => "node",
            Tool::Gh => "gh",
        }
    }

    pub fn from_id(id: &str) -> Option<Tool> {
        TOOLS.into_iter().find(|t| t.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Git => "Git",
            Tool::Node => "Node.js and npm",
            Tool::Gh => "GitHub CLI",
        }
    }

    /// The binary whose presence on PATH means the tool is installed. npm is
    /// the one that matters for Node: a node without npm installs nothing.
    pub fn binary(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Node => "npm",
            Tool::Gh => "gh",
        }
    }

    /// Why Agency wants it, in the user's terms.
    pub fn why(self) -> &'static str {
        match self {
            Tool::Git => {
                "Projects are git repositories. Each agent gets its own branch and worktree, \
                 and merging and Source Control need git too."
            }
            Tool::Node => {
                "Installs and runs the agents that ship through npm: Claude Code, Codex, \
                 Gemini CLI, Copilot CLI and others."
            }
            Tool::Gh => "Opens pull requests and imports GitHub issues. Optional.",
        }
    }

    /// Whether the app is materially worse without it. Only git: a project
    /// without git is a plain folder, which is a supported but narrow mode.
    pub fn required(self) -> bool {
        matches!(self, Tool::Git)
    }

    /// Where to read about installing it by hand, for the manual fallback.
    pub fn url(self) -> &'static str {
        match self {
            Tool::Git => "https://git-scm.com/downloads",
            Tool::Node => "https://nodejs.org/en/download",
            Tool::Gh => "https://cli.github.com",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    #[serde(rename = "macos")]
    Mac,
    Linux,
    Windows,
}

/// The package managers with a known, non-interactive install line. Linux
/// distributions without one of these (Alpine, NixOS, Void) get the manual
/// fallback rather than a guessed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Winget,
}

impl PackageManager {
    fn binary(self) -> &'static str {
        match self {
            PackageManager::Brew => "brew",
            PackageManager::Apt => "apt-get",
            PackageManager::Dnf => "dnf",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
            PackageManager::Winget => "winget",
        }
    }
}

/// What the machine offers for installing things.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub os: Os,
    pub package_manager: Option<PackageManager>,
    /// polkit's `pkexec`: a root prompt the desktop draws, so a system package
    /// can be installed from a GUI app with no terminal to type a password in.
    pub pkexec: bool,
}

/// The current OS, at compile time.
pub fn current_os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// Probe the machine. `on_path` answers whether a binary is installed; it is
/// a parameter so the table below can be tested against any machine.
pub fn detect_platform(os: Os, on_path: impl Fn(&str) -> bool) -> Platform {
    let candidates: &[PackageManager] = match os {
        Os::Mac => &[PackageManager::Brew],
        // Ordered by how often a desktop has exactly one of them. apt before
        // dnf: a Debian with `dnf` installed is a curiosity, a Fedora with
        // `apt-get` is a real (and broken) thing, and both are rare.
        Os::Linux => &[
            PackageManager::Apt,
            PackageManager::Dnf,
            PackageManager::Pacman,
            PackageManager::Zypper,
        ],
        Os::Windows => &[PackageManager::Winget],
    };
    Platform {
        os,
        package_manager: candidates.iter().copied().find(|pm| on_path(pm.binary())),
        pkexec: os == Os::Linux && on_path("pkexec"),
    }
}

/// How a tool gets installed on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InstallPlan {
    /// Agency can run this itself, in the background, and report the result.
    Run {
        command: String,
        /// Said beside the progress line when the command's own behaviour needs
        /// explaining: a root prompt that is about to appear, an installer
        /// window macOS opens.
        note: Option<String>,
    },
    /// Nothing safe to run unattended. `command` is the line to copy when
    /// there is one (a `sudo` install needs a terminal to type the password
    /// in); `hint` says what to do either way.
    Manual { command: Option<String>, hint: String, url: String },
}

/// The distribution package names, per manager. Kept in one place so a wrong
/// name is a one-line fix and the test below catches an entry going missing.
fn packages(tool: Tool, pm: PackageManager) -> Option<&'static str> {
    Some(match (pm, tool) {
        (PackageManager::Brew, Tool::Git) => "git",
        (PackageManager::Brew, Tool::Node) => "node",
        (PackageManager::Brew, Tool::Gh) => "gh",
        (PackageManager::Apt, Tool::Git) => "git",
        // Debian 12 and Ubuntu 22.04+ ship a Node the npm agents run on (18
        // and up). Older releases would get an npm too old for them, and the
        // install output says so in the agent's own words.
        (PackageManager::Apt, Tool::Node) => "nodejs npm",
        (PackageManager::Apt, Tool::Gh) => "gh",
        (PackageManager::Dnf, Tool::Git) => "git",
        (PackageManager::Dnf, Tool::Node) => "nodejs npm",
        (PackageManager::Dnf, Tool::Gh) => "gh",
        (PackageManager::Pacman, Tool::Git) => "git",
        (PackageManager::Pacman, Tool::Node) => "nodejs npm",
        (PackageManager::Pacman, Tool::Gh) => "github-cli",
        (PackageManager::Zypper, Tool::Git) => "git",
        // openSUSE versions its Node packages (nodejs22, npm22) and the
        // unversioned aliases vary by release, so no guess is offered.
        (PackageManager::Zypper, Tool::Node) => return None,
        (PackageManager::Zypper, Tool::Gh) => "gh",
        (PackageManager::Winget, Tool::Git) => "Git.Git",
        (PackageManager::Winget, Tool::Node) => "OpenJS.NodeJS.LTS",
        (PackageManager::Winget, Tool::Gh) => "GitHub.cli",
    })
}

/// The root-level install line for a Linux package manager, before any
/// privilege wrapper.
fn linux_install_line(pm: PackageManager, pkgs: &str) -> String {
    match pm {
        // `update` first: a fresh install's package lists are often stale
        // enough that `install` fails with a 404 on the first try.
        PackageManager::Apt => {
            format!(
                "apt-get update -q && DEBIAN_FRONTEND=noninteractive apt-get install -y -q {pkgs}"
            )
        }
        PackageManager::Dnf => format!("dnf install -y {pkgs}"),
        PackageManager::Pacman => format!("pacman -S --noconfirm --needed {pkgs}"),
        PackageManager::Zypper => format!("zypper --non-interactive install {pkgs}"),
        PackageManager::Brew | PackageManager::Winget => unreachable!("not a Linux manager"),
    }
}

/// The plan for `tool` on `platform`.
pub fn install_plan(tool: Tool, platform: &Platform) -> InstallPlan {
    let manual = |command: Option<String>, hint: &str| InstallPlan::Manual {
        command,
        hint: hint.to_string(),
        url: tool.url().to_string(),
    };
    match platform.os {
        Os::Mac => match platform.package_manager {
            Some(PackageManager::Brew) => InstallPlan::Run {
                command: format!("brew install {}", packages(tool, PackageManager::Brew).unwrap()),
                note: None,
            },
            // No Homebrew. git comes with the Command Line Tools, which have
            // their own installer; the rest needs Homebrew or a download.
            _ => match tool {
                Tool::Git => InstallPlan::Run {
                    command: "xcode-select --install".to_string(),
                    note: Some(
                        "macOS opens its Command Line Tools installer. Finish that window and \
                         git appears here."
                            .to_string(),
                    ),
                },
                Tool::Node | Tool::Gh => manual(
                    None,
                    "Install Homebrew from brew.sh and run this again, or download the \
                     installer from the site below.",
                ),
            },
        },
        Os::Linux => {
            let Some(pm) = platform.package_manager else {
                return manual(
                    None,
                    "Install it with your distribution's package manager, then come back.",
                );
            };
            let Some(pkgs) = packages(tool, pm) else {
                return manual(
                    None,
                    "Install it with your distribution's package manager, then come back.",
                );
            };
            let line = linux_install_line(pm, pkgs);
            if platform.pkexec {
                InstallPlan::Run {
                    command: format!("pkexec sh -c {}", shell_quote(&line)),
                    note: Some(
                        "Your desktop asks for your password first: system packages install as \
                         root."
                            .to_string(),
                    ),
                }
            } else {
                // Without polkit there is no way to ask for root from here.
                manual(
                    Some(format!("sudo sh -c {}", shell_quote(&line))),
                    "Run this in a terminal (it needs your password), then come back.",
                )
            }
        }
        Os::Windows => match platform.package_manager {
            Some(PackageManager::Winget) => InstallPlan::Run {
                command: format!(
                    "winget install --id {} -e --accept-source-agreements --accept-package-agreements",
                    packages(tool, PackageManager::Winget).unwrap()
                ),
                note: None,
            },
            _ => manual(None, "Download the installer from the site below."),
        },
    }
}

/// The serialization lane a tool's install shares with others, or `None` when
/// it holds no lock anything else contends for.
///
/// A system package manager takes one machine-wide lock (dpkg's
/// `lock-frontend`, rpm's `__db`, pacman's `db.lck`): two installs at once
/// fail with "Could not get lock" (observed 2026-09-10, git and gh together),
/// so every tool that installs through one shares a lane and waits its turn.
/// Homebrew and winget serialize internally, but a shared lane spares the
/// confusing mid-install error. `xcode-select --install` and the download
/// fallbacks contend for nothing, so they run free.
pub fn install_lane(tool: Tool, platform: &Platform) -> Option<String> {
    match install_plan(tool, platform) {
        InstallPlan::Run { .. } => {
            platform.package_manager.map(|pm| format!("pkg:{}", pm.binary()))
        }
        InstallPlan::Manual { .. } => None,
    }
}

/// The lane an agent CLI install shares. Every `npm install -g` writes the one
/// global prefix and shares npm's cache, so parallel ones race; they share a
/// lane. A curl-piped vendor installer contends for nothing and runs free.
pub fn agent_install_lane(command: &str) -> Option<String> {
    command.starts_with("npm install -g ").then(|| "npm-global".to_string())
}

/// Single-quote `s` for `sh -c`. The only character that needs care inside
/// single quotes is the single quote itself.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The script that runs an agent's install line in the background.
///
/// `npm install -g` is the catalog's install line, and on a machine whose
/// node came from a distribution package (or from the nodejs.org .pkg on
/// macOS) the global prefix is root-owned: the install dies with EACCES on
/// `/usr/local/lib/node_modules`. The user-level fix npm itself documents is
/// a prefix under the home directory. `~/.local` is the XDG one, and its
/// `bin/` is on the PATH the stock Debian, Ubuntu and Fedora profiles build
/// (once the directory exists, which the script guarantees). Only applied
/// when the default prefix is in fact unwritable, so a Homebrew or nvm node
/// keeps its own prefix and the agent lands where the user's other globals
/// are.
///
/// Anything that is not an `npm install -g` line runs as written.
pub fn install_script(command: &str) -> String {
    if !command.starts_with("npm install -g ") {
        return command.to_string();
    }
    let rest = &command["npm install -g ".len()..];
    format!(
        "p=\"$(npm prefix -g)\"; \
         if [ -w \"$p/lib/node_modules\" ] || {{ [ ! -e \"$p/lib/node_modules\" ] && [ -w \"$p/lib\" ]; }}; then \
         npm install -g {rest}; \
         else \
         mkdir -p \"$HOME/.local/bin\" && npm install -g --prefix \"$HOME/.local\" {rest}; \
         fi"
    )
}

/// One row of the onboarding step, and of the dialog an error opens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub id: String,
    pub label: String,
    pub binary: String,
    pub why: String,
    pub required: bool,
    pub installed: bool,
    /// Where the binary resolved, when it did.
    pub path: Option<String>,
    pub plan: InstallPlan,
}

/// Every tool's status on this machine. `resolve` finds a binary on PATH;
/// `works` confirms a resolved binary actually runs (see [`git_works`]).
pub fn statuses(
    platform: &Platform,
    resolve: impl Fn(&str) -> Option<String>,
    works: impl Fn(Tool, &str) -> bool,
) -> Vec<ToolStatus> {
    TOOLS
        .into_iter()
        .map(|tool| {
            let path = resolve(tool.binary()).filter(|p| works(tool, p));
            ToolStatus {
                id: tool.id().to_string(),
                label: tool.label().to_string(),
                binary: tool.binary().to_string(),
                why: tool.why().to_string(),
                required: tool.required(),
                installed: path.is_some(),
                path,
                plan: install_plan(tool, platform),
            }
        })
        .collect()
}

/// Whether the `git` at `path` is a real one.
///
/// macOS ships `/usr/bin/git` on every machine, Command Line Tools or not.
/// Without them it is a stub that exits 1 and asks macOS to open the Tools
/// installer, so "git is on PATH" is true and "git works" is false. Running
/// `git --version` to find out would pop that installer on every poll, so
/// the question goes to `xcode-select -p` instead, which answers without a
/// window: exit 0 with a developer directory, exit 2 without one.
pub fn git_works(path: &str) -> bool {
    if !cfg!(target_os = "macos") || path != "/usr/bin/git" {
        return true;
    }
    std::process::Command::new("xcode-select")
        .arg("-p")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(set: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |b| set.contains(&b)
    }

    fn command(plan: &InstallPlan) -> &str {
        match plan {
            InstallPlan::Run { command, .. } => command,
            InstallPlan::Manual { .. } => panic!("expected a runnable plan, got {plan:?}"),
        }
    }

    #[test]
    fn every_tool_round_trips_its_id() {
        for tool in TOOLS {
            assert_eq!(Tool::from_id(tool.id()), Some(tool));
        }
        assert_eq!(Tool::from_id("brew"), None);
    }

    #[test]
    fn a_mac_with_homebrew_installs_through_it() {
        let p = detect_platform(Os::Mac, on(&["brew", "git"]));
        assert_eq!(p.package_manager, Some(PackageManager::Brew));
        assert!(!p.pkexec, "pkexec is a Linux thing even when a mac has one");
        assert_eq!(command(&install_plan(Tool::Node, &p)), "brew install node");
        assert_eq!(command(&install_plan(Tool::Gh, &p)), "brew install gh");
    }

    #[test]
    fn a_mac_without_homebrew_gets_the_command_line_tools_for_git_only() {
        let p = detect_platform(Os::Mac, on(&[]));
        assert_eq!(p.package_manager, None);
        assert_eq!(command(&install_plan(Tool::Git, &p)), "xcode-select --install");
        assert!(matches!(install_plan(Tool::Node, &p), InstallPlan::Manual { command: None, .. }));
        assert!(matches!(install_plan(Tool::Gh, &p), InstallPlan::Manual { command: None, .. }));
    }

    #[test]
    fn ubuntu_with_polkit_installs_as_root_through_pkexec() {
        let p = detect_platform(Os::Linux, on(&["apt-get", "pkexec"]));
        assert_eq!(p.package_manager, Some(PackageManager::Apt));
        assert!(p.pkexec);
        assert_eq!(
            command(&install_plan(Tool::Git, &p)),
            "pkexec sh -c 'apt-get update -q && DEBIAN_FRONTEND=noninteractive apt-get install -y -q git'"
        );
        assert_eq!(
            command(&install_plan(Tool::Node, &p)),
            "pkexec sh -c 'apt-get update -q && DEBIAN_FRONTEND=noninteractive apt-get install -y -q nodejs npm'"
        );
    }

    #[test]
    fn linux_without_polkit_hands_over_a_sudo_line_to_copy() {
        let p = detect_platform(Os::Linux, on(&["dnf"]));
        match install_plan(Tool::Gh, &p) {
            InstallPlan::Manual { command: Some(c), hint, url } => {
                assert_eq!(c, "sudo sh -c 'dnf install -y gh'");
                assert!(hint.contains("terminal"));
                assert_eq!(url, "https://cli.github.com");
            }
            other => panic!("expected a copyable sudo line, got {other:?}"),
        }
    }

    #[test]
    fn arch_uses_pacman_and_its_own_package_names() {
        let p = detect_platform(Os::Linux, on(&["pacman", "pkexec"]));
        assert_eq!(
            command(&install_plan(Tool::Gh, &p)),
            "pkexec sh -c 'pacman -S --noconfirm --needed github-cli'"
        );
    }

    #[test]
    fn a_distribution_without_a_known_manager_gets_the_manual_fallback() {
        let p = detect_platform(Os::Linux, on(&["pkexec"]));
        assert_eq!(p.package_manager, None);
        assert!(matches!(install_plan(Tool::Git, &p), InstallPlan::Manual { command: None, .. }));
    }

    #[test]
    fn the_first_manager_found_wins_in_catalog_order() {
        let p = detect_platform(Os::Linux, on(&["zypper", "apt-get"]));
        assert_eq!(p.package_manager, Some(PackageManager::Apt));
    }

    #[test]
    fn zypper_has_no_guess_for_node() {
        let p = detect_platform(Os::Linux, on(&["zypper", "pkexec"]));
        assert!(matches!(install_plan(Tool::Node, &p), InstallPlan::Manual { command: None, .. }));
        assert_eq!(
            command(&install_plan(Tool::Git, &p)),
            "pkexec sh -c 'zypper --non-interactive install git'"
        );
    }

    #[test]
    fn windows_goes_through_winget() {
        let p = detect_platform(Os::Windows, on(&["winget"]));
        assert!(command(&install_plan(Tool::Git, &p)).starts_with("winget install --id Git.Git -e"));
    }

    #[test]
    fn shell_quote_survives_an_embedded_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn every_system_package_install_shares_one_lane() {
        // The dpkg-lock collision: git, node and gh all install through apt,
        // so all three must land in the same lane and serialize.
        let apt = detect_platform(Os::Linux, on(&["apt-get", "pkexec"]));
        let lanes: Vec<_> = TOOLS.into_iter().map(|t| install_lane(t, &apt)).collect();
        assert!(lanes.iter().all(|l| l.as_deref() == Some("pkg:apt-get")));

        // Homebrew is one lane too; a mac without it installs git through
        // xcode-select, which contends for nothing.
        let brew = detect_platform(Os::Mac, on(&["brew"]));
        assert_eq!(install_lane(Tool::Node, &brew).as_deref(), Some("pkg:brew"));
        let bare = detect_platform(Os::Mac, on(&[]));
        assert_eq!(install_lane(Tool::Git, &bare), None);
    }

    #[test]
    fn npm_agents_share_a_lane_and_curl_installers_do_not() {
        assert_eq!(
            agent_install_lane("npm install -g @anthropic-ai/claude-code").as_deref(),
            Some("npm-global")
        );
        assert_eq!(agent_install_lane("curl -fsSL https://x/install.sh | bash"), None);
    }

    #[test]
    fn npm_globals_fall_back_to_a_home_prefix_when_the_default_is_root_owned() {
        let script = install_script("npm install -g @anthropic-ai/claude-code");
        assert!(script.starts_with("p=\"$(npm prefix -g)\"; if [ -w \"$p/lib/node_modules\" ]"));
        assert!(script.contains("then npm install -g @anthropic-ai/claude-code;"));
        assert!(script.contains(
            "mkdir -p \"$HOME/.local/bin\" && npm install -g --prefix \"$HOME/.local\" @anthropic-ai/claude-code;"
        ));
    }

    #[test]
    fn non_npm_install_lines_run_as_written() {
        let curl = "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash";
        assert_eq!(install_script(curl), curl);
        assert_eq!(install_script("brew install gh"), "brew install gh");
    }

    #[test]
    fn statuses_report_every_tool_and_trust_the_works_check() {
        let p = detect_platform(Os::Linux, on(&["apt-get"]));
        let rows = statuses(
            &p,
            |b| (b == "git" || b == "npm").then(|| format!("/usr/bin/{b}")),
            |tool, _| tool != Tool::Node,
        );
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["git", "node", "gh"]);
        assert!(rows[0].installed);
        assert_eq!(rows[0].path.as_deref(), Some("/usr/bin/git"));
        assert!(!rows[1].installed, "a binary that does not work is not installed");
        assert_eq!(rows[1].path, None);
        assert!(!rows[2].installed);
        assert!(rows[0].required && !rows[1].required && !rows[2].required);
    }
}
