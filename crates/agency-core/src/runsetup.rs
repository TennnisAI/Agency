//! Guesses at a project's "start the app" command, used to prefill the Run
//! tab's setup card so configuring a run script is a click rather than a
//! hand-edited TOML file.
//!
//! Everything here is a suggestion: the user always sees the command before it
//! is saved, so a wrong guess costs an edit, never a surprise.

use serde::Serialize;
use std::path::Path;

/// One candidate run command offered by the setup UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSuggestion {
    /// Short chip label, e.g. "pnpm dev".
    pub label: String,
    /// The name to save the script under, e.g. "dev". Short, because it is what
    /// the Run tab lists and what the user picks between.
    pub name: String,
    /// The full command written to the config, e.g. "pnpm dev --port $AGENCY_PORT".
    pub command: String,
    /// Where the guess came from, e.g. "package.json script".
    pub detail: String,
    /// Whether this candidate serves a web app, and so should come with the
    /// URL and preview pane. A build or a one-shot task does not.
    pub web: bool,
}

impl RunSuggestion {
    fn new(label: &str, name: &str, command: &str, detail: &str, web: bool) -> RunSuggestion {
        RunSuggestion {
            label: label.into(),
            name: name.into(),
            command: command.into(),
            detail: detail.into(),
            web,
        }
    }
}

/// Ordered candidates for starting a project's app, best guess first. Empty
/// when nothing recognizable is in the repo root — the UI then just shows an
/// empty field.
pub fn suggest_run_commands(repo: &Path) -> Vec<RunSuggestion> {
    let mut out = Vec::new();
    out.extend(node_suggestions(repo));

    // A repo-root dev script is a strong signal, but the package manager entry
    // above is usually the more familiar one, so it comes second.
    for file in ["dev.sh", "run.sh", "start.sh"] {
        if repo.join(file).is_file() {
            let stem = file.trim_end_matches(".sh");
            out.push(RunSuggestion::new(
                &format!("./{file}"),
                stem,
                &format!("./{file}"),
                "script in the repo root",
                true,
            ));
        }
    }

    if repo.join("manage.py").is_file() {
        out.push(RunSuggestion::new(
            "manage.py runserver",
            "runserver",
            "python manage.py runserver 0.0.0.0:$AGENCY_PORT",
            "Django project",
            true,
        ));
    }
    if repo.join("Cargo.toml").is_file() {
        out.push(RunSuggestion::new("cargo run", "run", "cargo run", "Cargo project", false));
    }
    if repo.join("go.mod").is_file() {
        out.push(RunSuggestion::new("go run .", "run", "go run .", "Go module", false));
    }
    if has_make_target(repo, "dev") {
        out.push(RunSuggestion::new("make dev", "dev", "make dev", "Makefile target", true));
    }
    for file in ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"] {
        if repo.join(file).is_file() {
            out.push(RunSuggestion::new(
                "docker compose up",
                "compose",
                "docker compose up",
                file,
                true,
            ));
            break;
        }
    }

    out.truncate(6);
    out
}

/// Candidates read out of `package.json`: the dev-ish scripts that exist, in
/// the order a developer would reach for them, spelled for the package manager
/// the repo's lockfile implies.
fn node_suggestions(repo: &Path) -> Vec<RunSuggestion> {
    let Ok(text) = std::fs::read_to_string(repo.join("package.json")) else {
        return Vec::new();
    };
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let scripts = pkg.get("scripts").and_then(|s| s.as_object());
    let pm = package_manager(repo, &pkg);
    let port_style = port_style(&pkg);

    let Some(scripts) = scripts else { return Vec::new() };

    let mut out = Vec::new();
    for name in ["dev", "start", "serve", "develop"] {
        if !scripts.contains_key(name) {
            continue;
        }
        out.push(RunSuggestion::new(
            &format!("{pm} {name}"),
            name,
            &node_command(pm, name, port_style),
            "package.json script",
            true,
        ));
        if out.len() == 2 {
            break;
        }
    }
    // A build is the other half of what a project wants to run, and the reason
    // the Run tab holds a list rather than one command. It takes no port and
    // opens no browser.
    if scripts.contains_key("build") {
        let command =
            if pm == "npm" || pm == "bun" { format!("{pm} run build") } else { format!("{pm} build") };
        out.push(RunSuggestion::new(
            &format!("{pm} build"),
            "build",
            &command,
            "package.json script",
            false,
        ));
    }
    out
}

/// How a project's dev server takes its port: as a `--port` flag the package
/// manager can forward, or as a `PORT` environment variable.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PortStyle {
    Flag,
    Env,
}

fn node_command(pm: &str, script: &str, style: PortStyle) -> String {
    match style {
        // npm needs `--` to forward flags to the script; the others don't.
        PortStyle::Flag if pm == "npm" => format!("npm run {script} -- --port $AGENCY_PORT"),
        PortStyle::Flag if pm == "bun" => format!("bun run {script} --port $AGENCY_PORT"),
        PortStyle::Flag => format!("{pm} {script} --port $AGENCY_PORT"),
        PortStyle::Env if pm == "npm" => format!("PORT=$AGENCY_PORT npm run {script}"),
        PortStyle::Env if pm == "bun" => format!("PORT=$AGENCY_PORT bun run {script}"),
        PortStyle::Env => format!("PORT=$AGENCY_PORT {pm} {script}"),
    }
}

/// The repo's package manager, from `packageManager` if declared, else from the
/// lockfile that is actually checked in. Defaults to npm.
fn package_manager(repo: &Path, pkg: &serde_json::Value) -> &'static str {
    if let Some(declared) = pkg.get("packageManager").and_then(|v| v.as_str()) {
        for pm in ["pnpm", "yarn", "bun", "npm"] {
            if declared.starts_with(pm) {
                return pm;
            }
        }
    }
    for (file, pm) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
    ] {
        if repo.join(file).is_file() {
            return pm;
        }
    }
    "npm"
}

/// Dev servers that take `--port` (Vite and the frameworks built on it) versus
/// the `PORT=` convention everything else tends to follow.
fn port_style(pkg: &serde_json::Value) -> PortStyle {
    let has = |name: &str| {
        ["dependencies", "devDependencies"]
            .iter()
            .filter_map(|k| pkg.get(k).and_then(|d| d.as_object()))
            .any(|deps| deps.contains_key(name))
    };
    if ["vite", "next", "nuxt", "astro", "@sveltejs/kit", "@remix-run/dev"].iter().any(|d| has(d)) {
        PortStyle::Flag
    } else {
        PortStyle::Env
    }
}

/// Whether a root Makefile declares the given target. Deliberately shallow: a
/// line starting `<target>:` is the only form worth guessing from.
fn has_make_target(repo: &Path, target: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(repo.join("Makefile")) else {
        return false;
    };
    text.lines().any(|l| {
        l.strip_prefix(target)
            .is_some_and(|rest| rest.starts_with(':'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn empty_repo_has_no_suggestions() {
        let dir = tempdir().unwrap();
        assert!(suggest_run_commands(dir.path()).is_empty());
    }

    #[test]
    fn vite_pnpm_project_suggests_port_flag() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"dev":"vite","build":"vite build"},"devDependencies":{"vite":"^5"}}"#,
        );
        write(dir.path(), "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
        let s = suggest_run_commands(dir.path());
        assert_eq!(s[0].label, "pnpm dev");
        assert_eq!(s[0].command, "pnpm dev --port $AGENCY_PORT");
    }

    #[test]
    fn npm_project_without_vite_uses_port_env_and_run() {
        let dir = tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"scripts":{"start":"node server.js"}}"#);
        let s = suggest_run_commands(dir.path());
        assert_eq!(s[0].command, "PORT=$AGENCY_PORT npm run start");
    }

    #[test]
    fn npm_forwards_port_flag_with_double_dash() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"dev":"next dev"},"dependencies":{"next":"14"}}"#,
        );
        let s = suggest_run_commands(dir.path());
        assert_eq!(s[0].command, "npm run dev -- --port $AGENCY_PORT");
    }

    #[test]
    fn package_manager_field_wins_over_lockfile() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"packageManager":"yarn@4.1.0","scripts":{"dev":"vite"},"devDependencies":{"vite":"^5"}}"#,
        );
        write(dir.path(), "pnpm-lock.yaml", "");
        assert_eq!(suggest_run_commands(dir.path())[0].command, "yarn dev --port $AGENCY_PORT");
    }

    #[test]
    fn at_most_two_dev_package_json_scripts_are_offered() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"dev":"x","start":"y","serve":"z","develop":"w"}}"#,
        );
        assert_eq!(suggest_run_commands(dir.path()).len(), 2);
    }

    #[test]
    fn a_build_script_is_offered_without_a_browser_preview() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"dev":"vite","build":"vite build"},"devDependencies":{"vite":"^5"}}"#,
        );
        write(dir.path(), "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
        let s = suggest_run_commands(dir.path());
        let build = s.iter().find(|c| c.name == "build").expect("build offered");
        assert_eq!(build.command, "pnpm build");
        assert!(!build.web, "a build has nothing to open in a browser");
        assert!(s[0].web, "the dev server does");
    }

    #[test]
    fn non_node_projects_are_detected() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        write(dir.path(), "Makefile", "build:\n\tcargo build\ndev:\n\tcargo run\n");
        let cmds: Vec<String> = suggest_run_commands(dir.path()).into_iter().map(|s| s.command).collect();
        assert!(cmds.contains(&"cargo run".to_string()));
        assert!(cmds.contains(&"make dev".to_string()));
    }

    #[test]
    fn makefile_without_dev_target_is_not_suggested() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Makefile", "build:\n\tgo build\n# dev: not a target\n");
        assert!(suggest_run_commands(dir.path()).is_empty());
    }

    #[test]
    fn root_dev_script_is_suggested() {
        let dir = tempdir().unwrap();
        write(dir.path(), "dev.sh", "#!/bin/sh\n");
        let s = suggest_run_commands(dir.path());
        assert_eq!(s[0].command, "./dev.sh");
    }
}
