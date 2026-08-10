use std::path::{Path, PathBuf};

fn main() {
    ensure_sidecar();
    tauri_build::build();
}

/// Stage the `agency-termd` sidecar that Tauri's `externalBin` demands.
///
/// tauri-build validates `target/release/agency-termd-<triple>` and aborts when
/// it is missing, which makes a release artifact a hard prerequisite for
/// *compiling* this crate at all: on a fresh worktree (empty `target/`) plain
/// `cargo check`/`cargo test --workspace` die before touching a line of Rust,
/// with only "resource path ... doesn't exist" to go on. Nothing about a debug
/// build actually needs the release daemon, so supply one here.
///
/// Release stays strict — a bundle must carry the real thing — but says which
/// script builds it instead of naming a path and stopping.
fn ensure_sidecar() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    // crates/agency-app -> repo root, popped rather than `../..` so the paths
    // in the messages below are readable.
    let repo = manifest.parent().and_then(Path::parent).expect("crate is two levels deep");
    let triple = std::env::var("TARGET").expect("cargo sets this");
    // Matches `bundle.externalBin` in tauri.conf.json, which resolves relative
    // to the config file (this crate's directory).
    let sidecar = repo.join("target/release").join(format!("agency-termd-{triple}"));
    if sidecar.exists() {
        return;
    }
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "missing sidecar {}\n       build it first: ./crates/agency-app/build-termd.sh",
            sidecar.display()
        );
    }

    if let Some(parent) = sidecar.parent() {
        std::fs::create_dir_all(parent).expect("create target/release");
    }
    // `--target <triple>` nests the profile dir under the triple; a plain build
    // does not. Either way this is the daemon dev.sh builds.
    let debug_daemon = [
        repo.join(format!("target/{triple}/debug/agency-termd")),
        repo.join("target/debug/agency-termd"),
    ]
    .into_iter()
    .find(|p| p.exists());

    match debug_daemon {
        // tauri-build copies the sidecar next to the app executable, and that
        // copy is what the app spawns (`state::termd_bin`), so staging the real
        // debug daemon keeps `cargo run` working without dev.sh.
        Some(daemon) => {
            std::fs::copy(&daemon, &sidecar).expect("stage debug daemon as sidecar");
        }
        // Nothing has ever been built here. Compiling must not depend on the
        // daemon, but the app would spawn whatever we leave behind, so leave
        // something that explains itself rather than an empty file.
        None => write_placeholder(&sidecar),
    }
}

fn write_placeholder(sidecar: &Path) {
    println!(
        "cargo:warning=no agency-termd binary yet; staged a placeholder sidecar. \
         Run ./dev.sh to build and run the real daemon."
    );
    std::fs::write(
        sidecar,
        "#!/bin/sh\necho 'agency-termd placeholder: run ./dev.sh to build the daemon' >&2\nexit 127\n",
    )
    .expect("write placeholder sidecar");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(sidecar, std::fs::Permissions::from_mode(0o755))
            .expect("make placeholder sidecar executable");
    }
}
