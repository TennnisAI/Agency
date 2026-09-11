#!/usr/bin/env python3
"""Write the `latest.json` the in-app updater reads, from a release's artifacts.

Agency's updater endpoint is
`https://github.com/TennnisAI/Agency/releases/latest/download/latest.json`
(crates/agency-app/tauri.conf.json). That file names the version on offer and,
per platform, the artifact's URL and its minisign signature. The app verifies
that signature against the public key compiled into it before replacing itself,
so the signature is the whole of the trust here — HTTPS only says the bytes came
from GitHub, not that we are the ones who built them.

Run by the publish job of .github/workflows/release.yml over the directory every
build artifact was downloaded into.

    python3 scripts/updater-manifest.py \\
        --config crates/agency-app/tauri.conf.json --dist dist --out dist/latest.json

Two things it refuses to do, both because the failure is otherwise invisible
until a user presses Install on a release that has already gone out:

  * emit a platform whose artifact is present but whose `.sig` is not. An
    unsigned build is what you get when TAURI_SIGNING_PRIVATE_KEY is missing
    from the workflow, and the bundler says so only in a line of its log.
  * emit a manifest with no platforms at all.

Asset URLs go through `releases/latest/download/` rather than the tag. The
manifest and the files it names are attached to the same release, so "latest"
cannot resolve the two to different builds, and a release whose tag came out
wrong (see docs/releasing.md step 12) still serves working links.
"""

import argparse
import json
import sys
from pathlib import Path

DOWNLOAD_BASE = "https://github.com/TennnisAI/Agency/releases/latest/download"

# Tauri's platform keys, and the artifact each one is served by. The macOS
# tarball's name carries no version, which is why it can be listed literally;
# the AppImages are formatted with the version being released.
#
# Only aarch64 for macOS: CI builds Apple Silicon alone (see release.yml). An
# Intel Mac therefore finds no entry, and the updater tells it there is nothing
# to install rather than handing it an ARM build.
TARGETS = [
    ("darwin-aarch64", "Agency.app.tar.gz"),
    ("linux-x86_64", "appimage/Agency_{version}_amd64.AppImage"),
    ("linux-aarch64", "appimage/Agency_{version}_aarch64.AppImage"),
]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", required=True, type=Path, help="tauri.conf.json")
    ap.add_argument("--dist", required=True, type=Path, help="directory of downloaded artifacts")
    ap.add_argument("--out", required=True, type=Path, help="manifest to write")
    args = ap.parse_args()

    version = json.loads(args.config.read_text())["version"]

    platforms = {}
    missing_sigs = []
    for key, rel in TARGETS:
        artifact = args.dist / rel.format(version=version)
        if not artifact.exists():
            print(f"skipping {key}: no {artifact}")
            continue
        sig = artifact.with_name(artifact.name + ".sig")
        if not sig.exists():
            missing_sigs.append(f"{artifact} has no {sig.name}")
            continue
        platforms[key] = {
            "signature": sig.read_text().strip(),
            "url": f"{DOWNLOAD_BASE}/{artifact.name}",
        }
        print(f"{key}: {artifact.name}")

    if missing_sigs:
        # The bundler skips the signature when TAURI_SIGNING_PRIVATE_KEY is not
        # set, and says so in one line of a long log. Publishing the release
        # anyway would leave the platform out of the manifest silently.
        for m in missing_sigs:
            print(f"error: {m}", file=sys.stderr)
        print("error: the updater signing key did not reach the build", file=sys.stderr)
        return 1

    if not platforms:
        print(f"error: no updater artifacts for {version} under {args.dist}", file=sys.stderr)
        return 1

    args.out.write_text(
        json.dumps({"version": version, "platforms": platforms}, indent=2) + "\n"
    )
    print(f"wrote {args.out} for {version}: {', '.join(platforms)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
