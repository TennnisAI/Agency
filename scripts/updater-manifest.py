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
        --config crates/agency-app/tauri.conf.json --dist dist --out dist/latest.json \\
        [--tag v0.3.0]

It writes nothing, and so fails the release, when:

  * a platform's artifact is missing. Every entry in TARGETS is built by
    release.yml, and `createUpdaterArtifacts` is off in tauri.conf.json unless
    the build turns it on, so a missing one is a build leg that did not. The
    manifest would otherwise leave that platform out without a word.
  * an artifact has no `.sig`, for the same reason.
  * a signature was made by a key other than the one compiled into the app.
    The bundler only warns about that ("does not match the public key"), and
    what ships is a release every installed copy downloads and then rejects.
  * `--tag` is given and is not `v<version>`.

Asset URLs name the tag (`releases/download/v<version>/`), not `latest`. The
manifest itself is fetched through `latest`, but the files it points at must
be the ones its signatures were made from. With `latest` in those URLs too, a
client that read this manifest moments before the next release was published
would download the newer artifact against this one's signature and fail
verification. The tag is derived from the bundle version rather than the
workflow ref, because the 0.2.0 draft came out of release.yml carrying a
placeholder tag; docs/releasing.md step 12 is what makes the published tag
`v<version>`, and these links depend on it.
"""

import argparse
import base64
import json
import sys
from pathlib import Path

DOWNLOAD_BASE = "https://github.com/TennnisAI/Agency/releases/download"

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


def key_id(encoded: str) -> str:
    """The key ID in a Tauri-encoded minisign public key or signature.

    Tauri stores both as base64 of minisign's text form: a comment line, then a
    base64 payload whose first two bytes name the algorithm and whose next
    eight are the key ID, little-endian. Returned the way minisign prints it.
    """
    lines = [l for l in base64.b64decode(encoded).decode().splitlines() if l.strip()]
    payload = base64.b64decode(lines[1])
    return payload[2:10][::-1].hex().upper()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", required=True, type=Path, help="tauri.conf.json")
    ap.add_argument("--dist", required=True, type=Path, help="directory of downloaded artifacts")
    ap.add_argument("--out", required=True, type=Path, help="manifest to write")
    ap.add_argument("--tag", help="the release's git tag, checked against the bundle version")
    args = ap.parse_args()

    config = json.loads(args.config.read_text())
    version = config["version"]
    tag = f"v{version}"
    if args.tag is not None and args.tag != tag:
        print(
            f"error: tag {args.tag} does not match bundle version {version}; "
            f"the manifest's links name {tag}",
            file=sys.stderr,
        )
        return 1

    trusted = key_id(config["plugins"]["updater"]["pubkey"])

    platforms = {}
    problems = []
    for key, rel in TARGETS:
        artifact = args.dist / rel.format(version=version)
        sig = artifact.with_name(artifact.name + ".sig")
        if not artifact.exists():
            problems.append(f"{key}: no {artifact}")
            continue
        if not sig.exists():
            problems.append(f"{key}: {artifact} has no {sig.name}")
            continue
        signature = sig.read_text().strip()
        signer = key_id(signature)
        if signer != trusted:
            problems.append(
                f"{key}: {sig.name} is signed by key {signer}, but the app trusts {trusted}"
            )
            continue
        platforms[key] = {
            "signature": signature,
            "url": f"{DOWNLOAD_BASE}/{tag}/{artifact.name}",
        }
        print(f"{key}: {artifact.name} (key {signer})")

    if problems:
        for p in problems:
            print(f"error: {p}", file=sys.stderr)
        return 1

    args.out.write_text(
        json.dumps({"version": version, "platforms": platforms}, indent=2) + "\n"
    )
    print(f"wrote {args.out} for {version}: {', '.join(platforms)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
