# Releasing

Maintainer-only. Cutting a signed, notarized macOS build and publishing it.
Split out of the README, which had grown to more signing instructions than
product description.

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
