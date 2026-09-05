#!/usr/bin/env python3
"""Regenerate THIRD-PARTY.md from the two dependency trees Agency ships.

Why this exists: 315 Rust crates and 111 npm packages are statically linked or
bundled into the .app, and until this script the bundle carried not one of their
copyright notices. MIT requires its notice in "all copies or substantial
portions", and Apache-2.0 s4(a) requires the licence to travel with a derivative
work; a linked binary is both. The source repo was fine, the artifact people
install was not.

Only what actually reaches a user's machine is listed. On the Rust side that
means walking `cargo metadata`'s resolve graph following `normal` dependency
edges from the two workspace crates: dev-dependencies (tempfile) and
build-dependencies (tauri-build) compile nothing into the shipped binary, and
listing them would overstate the obligation. On the JavaScript side it means
walking package.json `dependencies` (never `devDependencies`: vite, typescript
and esbuild never leave the build machine) through pnpm's store, where a
package's own dependencies are its siblings inside `.pnpm/<id>/node_modules/`.

Licence texts are emitted once per distinct text, listing every package that
shipped that exact text. This is not cosmetic: two MIT files differing only in
the copyright line are two different notices and both have to appear, while the
forty crates shipping a byte-identical Apache-2.0 need it once.

Run from the repo root, with `ui/node_modules` installed:

    python3 scripts/third-party.py          # rewrite THIRD-PARTY.md
    python3 scripts/third-party.py --check  # exit 1 if it would change

`--check` is what CI runs (.github/workflows/ci.yml, the `app` job), because a
notices file is only worth having if it cannot silently fall behind a
`cargo add`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "THIRD-PARTY.md"

# Release builds are aarch64-apple-darwin only (README, Requirements). Resolving
# for that one target keeps Windows and Linux crates out of a notices file for a
# macOS app: `cargo metadata` without a filter reports the union of every
# platform's dependencies, which here is 571 crates against the 315 that ship.
TARGET = "aarch64-apple-darwin"

LICENSE_FILE_RE = re.compile(r"(?i)^(licen[cs]e|copying|notice|unlicen[cs]e)")

# A licence file is text. Anything past this is a vendored corpus that happens to
# match the name pattern, and inlining it would bury the notices it sits among.
MAX_LICENSE_BYTES = 64 * 1024

# Fonts are bundled as files rather than resolved from a dependency tree, so
# they are declared here and their licence texts read from beside them.
FONTS = [
    (
        "Geist",
        "ui/src/assets/fonts/LICENSE-geist.txt",
        "https://github.com/vercel/geist-font",
        "The UI typeface, bundled as `geist-latin-wght-normal.woff2` and "
        "`geist-latin-ext-wght-normal.woff2`.",
    ),
    (
        "JetBrains Mono",
        "ui/src/assets/fonts/LICENSE-jetbrains-mono.txt",
        "https://github.com/JetBrains/JetBrainsMono",
        "The monospace typeface used in terminals, diffs and code, bundled as "
        "`jetbrains-mono-latin-wght-normal.woff2`.",
    ),
]


def run(*args: str) -> str:
    p = subprocess.run(args, cwd=REPO, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"third-party.py: {' '.join(args)} failed:\n{p.stderr.strip()}")
    return p.stdout


def fenced(text: str) -> list[str]:
    """A code fence long enough that the text inside cannot end it early.

    No licence in either tree contains a backtick run today. One eventually
    will, and it would close the fence mid-notice and leave the rest of the
    file rendering as prose.
    """
    longest = max((len(m) for m in re.findall(r"`+", text)), default=0)
    fence = "`" * max(3, longest + 1)
    return [f"{fence}text", text, fence]


def read_license_files(directory: Path, extra: str | None = None) -> list[tuple[str, str]]:
    """Every licence text a package ships, as (filename, text), sorted by name.

    `extra` is Cargo's `license-file` manifest key, which names a file that need
    not match the usual pattern.
    """
    names = sorted(f for f in os.listdir(directory) if LICENSE_FILE_RE.match(f))
    if extra and extra not in names:
        names.append(extra)
    out = []
    for name in names:
        path = directory / name
        if not path.is_file() or path.stat().st_size > MAX_LICENSE_BYTES:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        # CRLF and trailing whitespace vary between publishers and would split
        # texts that are otherwise identical into separate blocks.
        text = "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").split("\n"))
        if text.strip():
            out.append((name, text.strip("\n")))
    return out


def rust_packages() -> list[dict]:
    md = json.loads(
        run("cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET)
    )
    by_id = {p["id"]: p for p in md["packages"]}
    nodes = {n["id"]: n for n in md["resolve"]["nodes"]}
    roots = set(md["workspace_members"])

    seen: set[str] = set()
    queue = list(roots)
    while queue:
        pid = queue.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            # kind is null for a normal dependency; dev/build edges carry a name.
            if None not in kinds:
                continue
            queue.append(dep["pkg"])

    out = []
    for pid in seen - roots:
        p = by_id[pid]
        directory = Path(p["manifest_path"]).parent
        out.append(
            {
                "name": p["name"],
                "version": p["version"],
                "license": p.get("license") or "see licence file",
                "url": p.get("repository") or f"https://crates.io/crates/{p['name']}",
                "texts": read_license_files(directory, p.get("license_file")),
            }
        )
    return sorted(out, key=lambda p: (p["name"].lower(), p["version"]))


def js_packages() -> list[dict]:
    ui = REPO / "ui"
    root = ui / "node_modules"
    if not root.is_dir():
        sys.exit("third-party.py: ui/node_modules is missing. Run: pnpm --dir ui install")

    manifest = json.loads((ui / "package.json").read_text())

    def sibling_root(real: Path, name: str) -> Path:
        # pnpm resolves a package's own dependencies as symlinks beside it inside
        # `.pnpm/<id>/node_modules/`, so the graph can be walked with realpath
        # alone. A scoped name sits one directory deeper than a bare one.
        return real.parents[1] if name.startswith("@") else real.parent

    found: dict[Path, dict] = {}
    unresolved: list[str] = []
    queue = [(name, root / name) for name in manifest.get("dependencies", {})]
    while queue:
        name, path = queue.pop()
        real = path.resolve()
        if real in found or not real.is_dir():
            continue
        pkg = json.loads((real / "package.json").read_text())
        found[real] = pkg
        siblings = sibling_root(real, name)
        deps = list(pkg.get("dependencies", {})) + list(pkg.get("optionalDependencies", {}))
        for dep in deps:
            # The sibling directory is where pnpm's isolated linker puts it. The
            # two fallbacks cover a hoisted store and a workspace root; a
            # dependency found by none of them is a hole in this file, so say so
            # rather than quietly shipping a notices file that is missing a
            # package. Under-reporting is the exact failure this guards.
            for candidate in (siblings / dep, root / ".pnpm" / "node_modules" / dep, root / dep):
                if candidate.exists():
                    queue.append((dep, candidate))
                    break
            else:
                if dep not in pkg.get("optionalDependencies", {}):
                    unresolved.append(f"{pkg.get('name')} -> {dep}")

    if unresolved:
        sys.exit(
            "third-party.py: could not resolve these dependencies in the pnpm "
            "store, so their licences would be missing:\n  "
            + "\n  ".join(sorted(set(unresolved)))
        )

    out = []
    for real, pkg in found.items():
        lic = pkg.get("license") or pkg.get("licenses") or "see licence file"
        if isinstance(lic, list):
            lic = " OR ".join(x.get("type", "?") if isinstance(x, dict) else str(x) for x in lic)
        elif isinstance(lic, dict):
            lic = lic.get("type", "?")
        repo = pkg.get("repository")
        if isinstance(repo, dict):
            repo = repo.get("url")
        url = pkg.get("homepage") or repo or f"https://www.npmjs.com/package/{pkg['name']}"
        url = re.sub(r"^git\+|\.git$", "", str(url)).replace("git://", "https://")
        out.append(
            {
                "name": pkg["name"],
                "version": pkg.get("version", ""),
                "license": str(lic).strip("()"),
                "url": url,
                "texts": read_license_files(real),
            }
        )
    return sorted(out, key=lambda p: (p["name"].lower(), p["version"]))


def index_table(packages: list[dict]) -> list[str]:
    lines = ["| Package | Version | Licence |", "| --- | --- | --- |"]
    for p in packages:
        lines.append(f"| [{p['name']}]({p['url']}) | {p['version']} | {p['license']} |")
    return lines


def license_blocks(packages: list[dict], heading: str) -> list[str]:
    """One block per distinct licence text, naming every package that ships it."""
    groups: dict[str, list[str]] = {}
    texts: dict[str, str] = {}
    order: list[str] = []
    no_text: list[dict] = []
    for p in packages:
        if not p["texts"]:
            no_text.append(p)
            continue
        for _, text in p["texts"]:
            # Group on the text with whitespace collapsed, print the first one
            # seen. 39 crates shipped the Apache-2.0 text differing only in line
            # wrapping and indentation, which byte-equality kept as 39 separate
            # 11 KB blocks: 418 KB of the same licence. Two Apache texts that
            # differ in their appendix (a filled-in copyright line) still differ
            # after collapsing, which is the point.
            key = " ".join(text.split())
            if key not in groups:
                groups[key] = []
                texts[key] = text
                order.append(key)
            label = f"{p['name']} {p['version']}"
            if label not in groups[key]:
                groups[key].append(label)

    lines = [f"### {heading}", ""]
    if no_text:
        lines += [
            "These packages publish no licence file of their own. The licence "
            "named beside each is the one its author declared in the package "
            "manifest, and its full text is among the blocks below.",
            "",
        ]
        for p in no_text:
            lines.append(f"- {p['name']} {p['version']} ({p['license']})")
        lines.append("")

    # Sorted by first package name so the file is stable across runs and diffs
    # show only what actually changed.
    for key in sorted(order, key=lambda k: (groups[k][0].lower(), k)):
        lines.append(f"**{', '.join(groups[key])}**")
        lines.append("")
        lines += fenced(texts[key])
        lines.append("")
    return lines


def render() -> str:
    rust = rust_packages()
    js = js_packages()

    lines: list[str] = []
    add = lines.append

    add("# Third-party notices")
    add("")
    add(
        "Agency is licensed under Apache-2.0 (see `LICENSE` and `NOTICE`). The "
        "app you install also contains the software listed here, under the "
        "licences reproduced below. Each licence text appears once, followed by "
        "the packages that ship it."
    )
    add("")
    add(
        "Generated by `scripts/third-party.py`, which CI re-runs on every pull "
        "request. Do not edit it by hand."
    )
    add("")
    add(
        "This file covers what is inside the app. It does not cover the coding "
        "agents Agency launches: those are separate programs you install "
        "yourself, under their own licences, and they talk to their own "
        "providers. `README.md` lists them, along with the optional tools "
        "Agency can offer to install for you."
    )
    add("")
    add(f"- Bundled fonts: {len(FONTS)}")
    add(f"- Rust crates: {len(rust)}")
    add(f"- JavaScript packages: {len(js)}")
    add("")

    add("## Bundled fonts")
    add("")
    for name, rel, url, note in FONTS:
        add(f"### {name}")
        add("")
        add(f"{note} Source: <{url}>.")
        add("")
        lines += fenced((REPO / rel).read_text(encoding="utf-8").strip("\n"))
        add("")

    add("## Design system")
    add("")
    add(
        "The colour tokens in `ui/src/theme.css` are the Catppuccin Mocha "
        "palette, published under the MIT licence at "
        "<https://github.com/catppuccin/catppuccin>. No Catppuccin code is "
        "bundled; the hex values are used with credit."
    )
    add("")

    add("## Rust crates")
    add("")
    add(
        "Statically linked into the `Agency` binary and the `agency-termd` "
        "sidecar. Resolved for `" + TARGET + "` following normal dependency "
        "edges only, so build-time and test-only crates are excluded."
    )
    add("")
    lines += index_table(rust)
    add("")
    lines += license_blocks(rust, "Rust licence texts")

    add("## JavaScript packages")
    add("")
    add(
        "Bundled into the frontend that ships inside the app. Build tooling "
        "(vite, typescript, esbuild and the rest of `devDependencies`) is "
        "excluded: none of it leaves the build machine."
    )
    add("")
    lines += index_table(js)
    add("")
    lines += license_blocks(js, "JavaScript licence texts")

    add("## Mozilla Public License 2.0 components")
    add("")
    add(
        "A few of the crates above are under MPL-2.0, which is a file-level "
        "copyleft: the source of those files must stay available to anyone who "
        "receives the binary. Agency does not modify them, so the published "
        "crate source at the URL beside each in the table is the source this "
        "build used. The versions are pinned in `Cargo.lock`."
    )
    add("")
    mpl = [p for p in rust + js if "MPL" in p["license"].upper()]
    for p in mpl:
        add(f"- [{p['name']} {p['version']}]({p['url']}) ({p['license']})")
    add("")

    return "\n".join(lines).rstrip("\n") + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--check",
        action="store_true",
        help="exit 1 if THIRD-PARTY.md is out of date instead of rewriting it",
    )
    args = ap.parse_args()

    text = render()
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            print(
                "THIRD-PARTY.md is out of date. Regenerate it with:\n"
                "  pnpm --dir ui install --frozen-lockfile\n"
                "  python3 scripts/third-party.py",
                file=sys.stderr,
            )
            return 1
        print("THIRD-PARTY.md is up to date.")
        return 0

    OUT.write_text(text, encoding="utf-8")
    print(f"Wrote {OUT.relative_to(REPO)} ({len(text) // 1024} KB).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
