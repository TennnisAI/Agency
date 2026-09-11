# Releasing

Maintainer-only. Cutting a release, the signed and notarized macOS DMG plus
the Linux packages, and publishing it.
Split out of the README, which had grown to more signing instructions than
product description.

## The checklist

Work it top to bottom. The order is load-bearing in one place, flagged where it
falls: pushing `main` deploys the site, and the site names a DMG that does not
exist until the release is published.

**Before tagging**

1. Land everything the release should carry on `main`.
2. Bump the version in the five files below.
3. `cargo update -p agency-core -p agency-app` to move `Cargo.lock`.
4. `cargo fmt` and `cargo test -p agency-core -p agency-app`. Build the sidecar
   first (`./crates/agency-app/build-termd.sh`) or the tests that need a daemon
   fail for a reason that has nothing to do with the release.
5. `ui/node_modules/.bin/tsc --noEmit` and `ui/node_modules/.bin/vite build`.
6. Only if a dependency changed: `python3 scripts/third-party.py`. CI diffs
   `THIRD-PARTY.md` on every pull request, so it is normally already current.
7. Commit the bump. **Keep the site out of this commit** (step 12 says why).
8. Write the release notes into `CHANGELOG.md`, newest section first.
   `git log --no-merges --format='%h %s' v<previous>..main` is the raw
   material; read the bodies before believing the subjects, since some of what
   looks like a feature is a design note with nothing built. The same text goes
   in the GitHub release at step 12, so write it once, here.

**Tagging**

9. Push `main`, then tag it and push the tag:

   ```sh
   git push origin main
   git tag v<version> && git push origin v<version>
   ```

   The tag is what fires `release.yml`. It builds, signs and notarizes the DMG,
   builds the Linux packages for both architectures, and attaches all seven
   files to one **draft** release once every build has passed. That takes a
   while; the signing steps are the slow half.

**After the build, before publishing**

10. Download the DMG from the draft and check it on a second Mac or a fresh user
    account, since your own machine already trusts your certificate:

    ```sh
    spctl -a -vvv -t install ~/Downloads/Agency_<version>_aarch64.dmg
    ```

    Then install one Linux package and launch it. The build has already checked
    the .deb the way a distribution would, but nothing in CI has started the
    app from it:

    ```sh
    sudo apt install ./Agency_<version>_amd64.deb
    ```

11. Read the actual sizes of all seven files off the draft, and correct
    `site/download.html` wherever one has moved. The page prints each next to
    its filename: the DMG under the macOS button, each Linux package under its
    own button.
12. Paste the `CHANGELOG.md` section into the draft and **publish the
    release**. Until you do,
    it is not `releases/latest`, which is both why the download link 404s and
    why nobody on the previous version is offered the update yet.
13. Now push the site commit to `main`. `deploy-site.yml` fires on the push and
    the download button starts working.
14. Load getagency.dev/download and click the macOS button and one Linux
    button.
15. Point the AUR package at the release, commit that here, and push it to the
    AUR (the clone is from the one-time setup below):

    ```sh
    packaging/aur/update.sh <version>
    git add packaging/aur && git commit -m "point the AUR package at <version>"
    git push origin main
    cp packaging/aur/agency-bin/PKGBUILD packaging/aur/agency-bin/.SRCINFO ~/aur-agency-bin/
    git -C ~/aur-agency-bin commit -am "agency-bin <version>" && git -C ~/aur-agency-bin push
    ```

    `update.sh` reads the checksums off the published .deb files, so it fails
    until step 12 is done.

Nothing else needs doing for the update check: it reads the latest release tag
from the GitHub API at launch and compares it to the running version, so
publishing is the whole of it.

## Bumping the version

There is no single source of truth for the version, and nothing in CI compares
these files, so bump all five by hand:

| File | Why |
| --- | --- |
| `crates/agency-app/tauri.conf.json` | names the DMG and the Linux packages, and sets the bundle's `CFBundleShortVersionString` |
| `crates/agency-app/Cargo.toml` | `AppState::version()`, which the update check compares to the latest release tag |
| `crates/agency-core/Cargo.toml` | the daemon's own `CARGO_PKG_VERSION`, reported over the preview RPC |
| `ui/package.json` | cosmetic, but drifts silently if skipped |
| `crates/agency-app/linux/build.agency.app.metainfo.xml` | add a `<release>` at the top; it is the version Linux software centres report for the installed package |

`tests/smoke.rs` fails when the first two have drifted apart, which is the pair
that actually matters: one names the DMG, the other is what the app reports
about itself. `tests/linux_packaging.rs` fails when the metainfo's newest
release is not the bundle version. The other two are on you.

The site carries the version in two files, and in one of them it is part of
seven **hard-coded filenames**: `site/download.html` links
`releases/latest/download/Agency_<version>_aarch64.dmg` and the six Linux
packages the same way, and repeats three of those names in its install
commands. `site/index.html` shows `v<version>` in the hero. Those links 404 the
moment the new release becomes `latest` unless the filenames have been bumped,
which is why the site bump is a separate commit held until step 13. Push it with
the rest and every download button is broken for the length of the build
instead.

## Automated (the normal path)

`.github/workflows/release.yml` builds, signs and notarizes the DMG on an Apple
Silicon runner, and calls `.github/workflows/linux-packages.yml` for the Linux
packages, which build on native x86_64 and arm64 runners. A last job attaches
everything to a **draft** GitHub Release once all of them have passed, so a
release never goes out missing a platform. A version tag is the
trigger (step 9 above); `workflow_dispatch` runs it by hand from the Actions tab
if you need a build without cutting a tag.

Review the attached files, then publish the release by hand. It runs the same
`cargo tauri build` as the local script; the only difference is that Tauri does
signing **and** notarization from environment variables (certificate from a
secret, notarization via an app-specific password) rather than from your local
keychain and notary profile.

> CI builds for Apple Silicon (`aarch64`) only. Intel Macs would need a
> `universal-apple-darwin` build: both Rust targets, plus a universal sidecar.

The DMG carries `THIRD-PARTY.md` inside the app, so the licences of the 300-odd
crates and 100-odd packages linked into it travel with it, as MIT and
Apache-2.0 both require. CI regenerates and diffs that file on every pull
request (`.github/workflows/ci.yml`, the `app` job), so by tag time it is
already current. If you ever change a dependency on a branch that skips CI,
regenerate it before tagging (step 6):

```sh
pnpm --dir ui install --frozen-lockfile
python3 scripts/third-party.py
```

## Local builds

### Unsigned

Fine for yourself; it will trip Gatekeeper on any other machine.

```sh
# Build the agency-termd sidecar first - the app bundles it as an externalBin.
./crates/agency-app/build-termd.sh

cargo tauri build
```

Output lands in `target/release/bundle/`: the `.app` under `macos/`, the `.dmg`
under `dmg/`.

### Linux packages

```sh
./scripts/release-linux.sh
```

Builds the `.deb`, `.rpm` and AppImage and checks the `.deb` the way a
distribution would. On macOS it builds inside a container, since Tauri cannot
cross-compile to Linux, and the packages land in `target/linux/`. The script's
header lists the options. Release packages come from CI rather than from here:
x86_64 on an Apple Silicon Mac runs under emulation, which is slow enough to be
impractical.

### Signed and notarized

Produces a DMG that opens cleanly elsewhere. This is the build to hand to
testers.

```sh
./scripts/release-macos.sh
```

The script runs the sidecar build, then `cargo tauri build` (which deep-signs
the whole bundle, `agency-termd` included, using the identity in
`tauri.conf.json`), then notarizes and staples the DMG. The first run takes a
few minutes while Apple processes the upload.

Signing config lives in `crates/agency-app/tauri.conf.json` under
`bundle.macOS`: signing identity, hardened runtime, `Agency.entitlements`.

### Verifying

Check what Gatekeeper will decide, ideally on a second Mac or a fresh user
account, since your own machine already trusts your certificate:

```sh
spctl -a -vvv -t install target/release/bundle/dmg/Agency_*.dmg
```

If notarization is rejected, the log names the offending file or entitlement:

```sh
xcrun notarytool log <submission-id> --keychain-profile agency-notary
```

## One-time setup

### Signing certificate

Needed on a machine that has not cut a release before.

1. **Developer ID Application certificate** - create it in Xcode (Settings →
   Accounts → your team → Manage Certificates → **+** → *Developer ID
   Application*) or at developer.apple.com. Creating one requires the **Account
   Holder** role on the team. Verify:

   ```sh
   security find-identity -v -p codesigning | grep "Developer ID Application"
   ```

   If the identity string differs from the one in `tauri.conf.json`, update
   `bundle.macOS.signingIdentity` to match.

2. **Notary credentials** - create an app-specific password at
   [appleid.apple.com](https://appleid.apple.com) (Sign-In & Security →
   App-Specific Passwords), then store a notarytool profile:

   ```sh
   xcrun notarytool store-credentials agency-notary \
     --apple-id "<your-apple-id>" --team-id "<TEAM_ID>" \
     --password "<app-specific-password>"
   ```

   `release-macos.sh` reads the `agency-notary` profile; override with
   `AGENCY_NOTARY_PROFILE`.

### AUR

`agency-bin` on the AUR is how Arch users install Agency with their own tools
(`yay -S agency-bin`, or `makepkg -si` in a clone), and how they get updates.
`packaging/aur/agency-bin/` is the source of truth; the AUR repository is a
two-file mirror of it (`PKGBUILD` and `.SRCINFO`). The PKGBUILD repackages the
release .deb rather than building from source.

1. Create an account at aur.archlinux.org and add an SSH public key to it.
2. After the first release carrying Linux packages is published, point the
   package at it and create the AUR repository by pushing to it. Cloning a
   package name that does not exist yet gives an empty repository, and the
   first push creates the package:

   ```sh
   packaging/aur/update.sh <version>
   git clone ssh://aur@aur.archlinux.org/agency-bin.git ~/aur-agency-bin
   cp packaging/aur/agency-bin/PKGBUILD packaging/aur/agency-bin/.SRCINFO ~/aur-agency-bin/
   git -C ~/aur-agency-bin add PKGBUILD .SRCINFO
   git -C ~/aur-agency-bin commit -m "agency-bin <version>" && git -C ~/aur-agency-bin push
   ```

3. Once it is live, the download page and the README stop saying there is no
   AUR package: the Arch line becomes `yay -S agency-bin`.

`update.sh` writes `.SRCINFO` itself, from the PKGBUILD, because `makepkg
--printsrcinfo` needs Arch. It knows the fields the PKGBUILD uses today, so
after adding a field to the PKGBUILD, compare the two on Arch (a container
will do) before pushing:

```sh
makepkg --printsrcinfo | diff - .SRCINFO
```

### CI secrets

Under **Settings → Secrets and variables → Actions**. Secrets do not travel
between repositories, so these must be set up again on any new home for the
repo.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the exported Developer ID `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | the `Developer ID Application: …` identity string |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | the 10-character Apple team identifier |

Export the certificate: in **Keychain Access → login → My Certificates**, select
the Developer ID Application entry, right-click → **Export** to a `.p12` with a
password, then:

```sh
base64 -i Certificates.p12 | pbcopy   # paste into APPLE_CERTIFICATE
```
