# Releasing

Maintainer-only. Cutting a signed, notarized macOS build and publishing it.
Split out of the README, which had grown to more signing instructions than
product description.

## Bumping the version

There is no single source of truth for the version, and nothing in CI compares
these files, so bump all four by hand before tagging:

| File | Why |
| --- | --- |
| `crates/agency-app/tauri.conf.json` | names the DMG and the bundle's `CFBundleShortVersionString` |
| `crates/agency-app/Cargo.toml` | `AppState::version()`, which the update check compares to the latest release tag |
| `crates/agency-core/Cargo.toml` | the daemon's own `CARGO_PKG_VERSION`, reported over the preview RPC |
| `ui/package.json` | cosmetic, but drifts silently if skipped |

Then `cargo update -p agency-core -p agency-app` to move `Cargo.lock`, and
`cargo test -p agency-core -p agency-app`: `tests/smoke.rs` fails when the crate
version and `tauri.conf.json` have drifted apart, which is the pair that
actually matters.

The site carries the version twice, and one of them is a **hard-coded DMG
filename**: `site/download.html` links
`releases/latest/download/Agency_<version>_aarch64.dmg`, and `site/index.html`
shows `v<version>` in the hero. That link 404s the moment the new release
becomes `latest` unless the filename has been bumped, so keep the site bump in
its own commit and push it to `main` **after** publishing the release, not
before: `deploy-site.yml` fires on every push to `main`, and a site that names a
DMG the published release does not have yet is a broken download button for the
length of the build.

## Automated (the normal path)

`.github/workflows/release.yml` builds, signs, notarizes, and attaches a DMG to
a **draft** GitHub Release on an Apple Silicon runner. Trigger it with a version
tag, or manually from the Actions tab:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Review the attached DMG, then publish the release by hand. It runs the same
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
regenerate it before tagging:

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
