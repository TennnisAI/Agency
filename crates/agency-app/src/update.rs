//! Updates: whether a newer release exists, whether this copy of Agency is one
//! Agency is allowed to replace, and how often to ask again.
//!
//! Two halves, deliberately kept apart.
//!
//! The **check** is a plain GET of GitHub's latest-release JSON, shelled out to
//! `curl`. It runs on launch, on a timer while the app stays open, and whenever
//! the user asks. It sends no identifiers: a User-Agent naming the app and
//! nothing else. It can be turned off in Settings, which suppresses the
//! automatic check only; the Help menu's "Check for Updates…" still works.
//!
//! The **install** is `tauri-plugin-updater`, and it only happens when the user
//! presses a button. It downloads a release artifact signed with Agency's own
//! updater key, verifies that signature against the public key compiled into
//! the app, and swaps the new build into place. Applying it never restarts the
//! app on its own: the new version is staged where the old one was and takes
//! effect on the next launch, which the user chooses. That is what keeps an
//! update from ever pulling the floor out from under running agents.
//!
//! Not every install is Agency's to replace. A `.deb`, `.rpm` or pacman package
//! belongs to the package manager that put it there, and writing over those
//! files behind its back leaves the system lying about what is installed. For
//! those, [`classify`] says so and [`manual_hint`] gives the user the line that
//! actually does the job.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Repo the release check reads. Kept next to the "Report an issue" URL in the
/// UI — both have to move together if the repo does.
const RELEASES_API: &str = "https://api.github.com/repos/TennnisAI/Agency/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/TennnisAI/Agency/releases/latest";

/// Wall-clock cap on the whole request. The check is never on a path the user is
/// waiting for, so a slow network should give up quietly rather than hang.
const TIMEOUT_SECS: u32 = 10;

/// How long a running Agency waits before asking again. Six hours sits in the
/// middle of what desktop apps do — Sparkle's default is a day, Chrome's
/// background check is about five hours — and the whole cost on the other end
/// is one unauthenticated GitHub API call. Before this, the check ran once at
/// launch and never again, so an app left open for a week never heard about
/// anything (AGE-229).
pub const CHECK_INTERVAL_SECS: i64 = 6 * 60 * 60;

/// First retry after a check that could not reach GitHub, doubling per
/// consecutive failure up to `CHECK_INTERVAL_SECS`. A laptop that spends the
/// morning on a captive portal should not spend it retrying every quarter hour.
pub const RETRY_BASE_SECS: i64 = 15 * 60;

/// Cap on the updater's request for `latest.json`, one small file. The plugin's
/// default is no cap at all.
pub const MANIFEST_TIMEOUT_SECS: u64 = 30;

/// Cap on the whole download, body included, which is how reqwest counts it.
/// With no cap, a connection that stalled mid-download (a laptop that slept, a
/// captive portal) held the install open forever, and the dialog in front of
/// it could not be closed; found in review. The plugin offers no read timeout,
/// so this is a total sized for a slow link to finish, not for a stall to be
/// noticed fast. The dialog closes mid-download now, so the wait is not the
/// user's; this is what eventually frees the install to be tried again.
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 45 * 60;

/// How this copy of Agency got onto the machine, which is what decides whether
/// Agency may replace itself or has to hand the job to whatever owns the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    /// A `.app` bundle, from the DMG. The updater swaps the bundle in place.
    MacApp,
    /// A `.app` bundle macOS will not let anything write to: opened straight
    /// from the mounted DMG, or from the read-only App Translocation copy macOS
    /// runs a quarantined download from. The updater's swap fails there with a
    /// raw filesystem error, found in review; moving the app into Applications
    /// is what fixes it, so that is what the user is told.
    MacReadOnly,
    /// A Linux AppImage, running under its own runtime. Replaced in place.
    AppImage,
    /// dpkg/apt owns these files.
    Deb,
    /// rpm/dnf/zypper owns these files.
    Rpm,
    /// pacman owns these files: `agency-bin`, built from the PKGBUILD in
    /// `packaging/aur/` until the AUR package is live.
    Pacman,
    /// Anything else: a `./dev.sh` build, a binary copied somewhere by hand.
    /// Offer the download page and nothing that writes to disk.
    Unknown,
}

impl InstallKind {
    /// Whether Agency can apply an update to this install itself.
    pub fn self_updating(self) -> bool {
        matches!(self, InstallKind::MacApp | InstallKind::AppImage)
    }
}

/// What the caller observed about the running process and the machine, so
/// [`classify`] stays a function of its inputs.
#[derive(Debug, Clone, Copy)]
pub struct Probe<'a> {
    /// `std::env::consts::OS`.
    pub os: &'a str,
    /// `$APPIMAGE`, which the AppImage runtime sets to the image's own path.
    pub appimage: Option<&'a str>,
    /// Path of the running executable.
    pub exe: &'a str,
    /// `/var/lib/pacman/local` is a directory.
    pub pacman_db: bool,
    /// `/var/lib/dpkg/status` exists.
    pub dpkg_db: bool,
    /// An rpm database is present.
    pub rpm_db: bool,
    /// The running executable sits on a read-only mount (macOS only).
    pub read_only: bool,
}

/// Work out which [`InstallKind`] `probe` describes.
pub fn classify(probe: &Probe) -> InstallKind {
    match probe.os {
        // The updater replaces the enclosing `.app`, so a binary running from
        // outside one (`./dev.sh`, `cargo run`) is not something it can swap —
        // and must not be, or a dev build would overwrite itself with a release.
        "macos" => {
            if !probe.exe.contains(".app/Contents/MacOS/") {
                InstallKind::Unknown
            } else if probe.read_only || probe.exe.contains("/AppTranslocation/") {
                // Translocation is a read-only mount, so `read_only` should
                // catch it alone; the path is the fallback for a statvfs that
                // could not answer, which reads as writable.
                InstallKind::MacReadOnly
            } else {
                InstallKind::MacApp
            }
        }
        "linux" => {
            // The only thing that distinguishes a running AppImage from an
            // ordinary binary, and the path the updater writes back to.
            if probe.appimage.is_some_and(|p| !p.is_empty()) {
                return InstallKind::AppImage;
            }
            // Everything a package manager installs lands under these; a binary
            // anywhere else came from a tarball or a build tree.
            if !probe.exe.starts_with("/usr/") && !probe.exe.starts_with("/opt/") {
                return InstallKind::Unknown;
            }
            // A machine normally runs exactly one of these. Pacman is tried
            // first because an Arch box can carry a dpkg database too (dpkg is
            // packaged for Arch, and debtap uses it), while nothing but Arch
            // has pacman's, so its answer is the most specific.
            if probe.pacman_db {
                InstallKind::Pacman
            } else if probe.dpkg_db {
                InstallKind::Deb
            } else if probe.rpm_db {
                InstallKind::Rpm
            } else {
                InstallKind::Unknown
            }
        }
        _ => InstallKind::Unknown,
    }
}

/// Probe the running process and the filesystem, then [`classify`] them.
pub fn current_kind() -> InstallKind {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let appimage = std::env::var("APPIMAGE").ok();
    classify(&Probe {
        os: std::env::consts::OS,
        appimage: appimage.as_deref(),
        exe: &exe,
        pacman_db: Path::new("/var/lib/pacman/local").is_dir(),
        dpkg_db: Path::new("/var/lib/dpkg/status").exists(),
        // Fedora 41 and dnf5 moved the rpm database under /usr/lib/sysimage;
        // older Fedora, RHEL and openSUSE keep it at /var/lib/rpm.
        rpm_db: Path::new("/var/lib/rpm").is_dir() || Path::new("/usr/lib/sysimage/rpm").is_dir(),
        read_only: on_read_only_mount(&exe),
    })
}

/// Whether `path` sits on a read-only mount: a DMG mounts read-only, and so
/// does the App Translocation copy macOS runs a quarantined download from. A
/// path statvfs cannot answer for reads as writable.
#[cfg(target_os = "macos")]
fn on_read_only_mount(path: &str) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path) else { return false };
    // SAFETY: `c_path` is NUL-terminated and outlives the call, and `stat` is a
    // plain-data out-parameter, read only after statvfs reports success.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        libc::statvfs(c_path.as_ptr(), &mut stat) == 0 && (stat.f_flag & libc::ST_RDONLY) != 0
    }
}

/// Only the macOS updater replaces a bundle on the mount it runs from.
#[cfg(not(target_os = "macos"))]
fn on_read_only_mount(_path: &str) -> bool {
    false
}

/// The command that updates a package-manager install, with the new version and
/// this machine's architecture already filled in — the user should never have
/// to work out which of six filenames is theirs.
///
/// `None` for an install Agency updates itself, and for one nothing true can be
/// said about.
pub fn manual_hint(kind: InstallKind, latest: &str, arch: &str) -> Option<String> {
    match kind {
        // Names the file exactly as release.yml attaches it; see the release
        // checklist in docs/releasing.md.
        InstallKind::Deb => {
            let a = if arch == "aarch64" { "arm64" } else { "amd64" };
            Some(format!("sudo apt install ./Agency_{latest}_{a}.deb"))
        }
        InstallKind::Rpm => Some(format!("sudo dnf install ./Agency-{latest}-1.{arch}.rpm")),
        // `agency-bin` is not on the AUR yet (README: "There is no AUR package
        // yet"), so a pacman install was built from the in-repo PKGBUILD. This
        // first said `yay -S agency-bin`, which fails with "target not found"
        // for everyone it is shown to; caught in review before it shipped. The
        // dialog says where to run it: packaging/aur/agency-bin in a checkout,
        // which release step 15 points at each release. It becomes
        // `yay -S agency-bin` once the AUR package is live.
        InstallKind::Pacman => Some("git pull && makepkg -si".to_string()),
        InstallKind::MacApp
        | InstallKind::MacReadOnly
        | InstallKind::AppImage
        | InstallKind::Unknown => None,
    }
}

/// Wall-clock seconds since the epoch, which is what [`Cadence`] measures in.
/// Wall clock rather than a monotonic instant on purpose: a laptop that slept
/// through the interval should check when it wakes, and only the wall clock
/// noticed the time passing.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// When the automatic check should next run, carried between ticks of the
/// update thread. Pure bookkeeping: the thread supplies the clock and does the
/// network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cadence {
    /// Unix seconds of the last attempt, successful or not.
    pub last_attempt: Option<i64>,
    /// Consecutive attempts that could not reach GitHub.
    pub failures: u32,
}

impl Cadence {
    /// Seconds to wait after the last attempt before trying again.
    pub fn wait_secs(&self) -> i64 {
        if self.failures == 0 {
            return CHECK_INTERVAL_SECS;
        }
        let shift = (self.failures - 1).min(31);
        RETRY_BASE_SECS.saturating_mul(1i64 << shift).min(CHECK_INTERVAL_SECS)
    }

    /// Whether a check should run now.
    pub fn due(&self, now: i64) -> bool {
        match self.last_attempt {
            None => true,
            // The clock moved backwards: a laptop waking in another timezone,
            // or an NTP step. Waiting for wall-clock time to catch up would
            // park the check for however far it jumped.
            Some(last) if now < last => true,
            Some(last) => now - last >= self.wait_secs(),
        }
    }

    /// Record an attempt. A success clears the backoff.
    pub fn record(&mut self, now: i64, ok: bool) {
        self.last_attempt = Some(now);
        self.failures = if ok { 0 } else { self.failures.saturating_add(1) };
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// Version of the running app, as the bundle reports it.
    pub current: String,
    /// Latest published release, with the leading `v` stripped. `None` when the
    /// check could not reach GitHub or found no releases.
    pub latest: Option<String>,
    /// True only when `latest` is a strictly newer release than `current`.
    pub update_available: bool,
    /// Where to send the user to get it by hand.
    pub url: String,
    /// The release notes, as GitHub holds them (Markdown). `None` when the
    /// release carries none.
    pub notes: Option<String>,
    /// How this copy was installed.
    pub install_kind: InstallKind,
    /// Whether the UI may offer an in-app install. False for a package-manager
    /// install, for a dev build, and whenever there is nothing to install.
    pub can_install: bool,
    /// The command a package-manager install needs instead, ready to copy.
    pub manual_hint: Option<String>,
    /// A version already installed in place and waiting for a restart. See
    /// [`UpdateCheck::with_install`].
    pub staged: Option<String>,
    /// An in-app install is downloading right now.
    pub installing: bool,
    /// Why the last in-app install failed, until another one starts.
    pub install_error: Option<String>,
    /// Why the check came back empty, for the UI to show verbatim. `None` on a
    /// successful check.
    pub error: Option<String>,
}

impl UpdateCheck {
    fn failed(current: &str, kind: InstallKind, error: String) -> Self {
        UpdateCheck {
            current: current.to_string(),
            latest: None,
            update_available: false,
            url: RELEASES_PAGE.to_string(),
            notes: None,
            install_kind: kind,
            can_install: false,
            manual_hint: None,
            staged: None,
            installing: false,
            install_error: None,
            error: Some(error),
        }
    }

    /// Fold in the in-app install: a version installed but not restarted into,
    /// a download under way, and one that failed.
    ///
    /// The running binary keeps reporting the old version until the restart, so
    /// without the staged version every check after Install and "Restart later"
    /// said the same release was still available: the Settings dot stayed lit,
    /// Diagnostics kept offering "Update…", and pressing Install downloaded the
    /// whole release a second time. A release newer than the staged one is still
    /// offered. While a download runs nothing offers Install, so a dialog opened
    /// meanwhile shows the download instead of a button the backend refuses.
    pub fn with_install(mut self, install: &Install) -> Self {
        if let Some(staged) = install.staged.as_deref() {
            let beyond_staged =
                self.latest.as_deref().is_some_and(|l| agency_core::version::is_newer(l, staged));
            if !beyond_staged {
                self.update_available = false;
                self.can_install = false;
                self.manual_hint = None;
            }
            self.staged = Some(staged.to_string());
        }
        self.installing = install.running;
        if install.running {
            self.can_install = false;
        }
        self.install_error = install.error.clone();
        self
    }
}

/// The in-app install as every open update dialog and the Settings row see it.
/// Pure bookkeeping: `install_update` does the download and reports here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Install {
    /// A version installed in place this launch and waiting for a restart.
    pub staged: Option<String>,
    /// A download is under way.
    pub running: bool,
    /// Why the last install failed, until the next one starts.
    pub error: Option<String>,
}

impl Install {
    /// Claim the install for one caller. False when one is already running.
    /// Settings and the Help menu can each hold an update dialog open, and
    /// nothing stopped both pressing Install: two downloads writing the same
    /// bundle. Found in review.
    pub fn begin(&mut self) -> bool {
        if self.running {
            return false;
        }
        self.running = true;
        self.error = None;
        true
    }

    /// Record how the claimed install ended. A failure keeps an earlier staged
    /// version: the download and its signature check, where installs fail,
    /// come before anything is written.
    pub fn finish(&mut self, outcome: &std::result::Result<String, String>) {
        self.running = false;
        match outcome {
            Ok(version) => {
                self.staged = Some(version.clone());
                self.error = None;
            }
            Err(e) => self.error = Some(e.clone()),
        }
    }
}

/// What to cache after a check: `next`, unless it could not reach GitHub and
/// `prev` could. A failed check says nothing about what has been released, and
/// letting it overwrite a good answer put the Settings dot out for as long as
/// the network was down.
pub fn keep_known(prev: Option<UpdateCheck>, next: UpdateCheck) -> UpdateCheck {
    match prev {
        Some(prev) if next.error.is_some() && prev.error.is_none() => prev,
        _ => next,
    }
}

/// The two fields of the release feed this cares about.
struct Release {
    tag: String,
    notes: Option<String>,
}

/// Fetch the latest release and compare it to `current`.
///
/// Never returns `Err` for an unreachable network — being offline is an
/// expected state, not a failure worth surfacing as an error dialog, so it
/// comes back as an `UpdateCheck` with `error` set for the UI to show quietly.
pub fn check(current: &str, kind: InstallKind) -> UpdateCheck {
    check_with_arch(current, kind, std::env::consts::ARCH)
}

fn check_with_arch(current: &str, kind: InstallKind, arch: &str) -> UpdateCheck {
    match fetch_latest_release() {
        Ok(rel) => {
            let available = agency_core::version::is_newer(&rel.tag, current);
            let latest = rel.tag.trim_start_matches('v').to_string();
            UpdateCheck {
                current: current.to_string(),
                update_available: available,
                can_install: available && kind.self_updating(),
                manual_hint: available.then(|| manual_hint(kind, &latest, arch)).flatten(),
                latest: Some(latest),
                url: RELEASES_PAGE.to_string(),
                notes: rel.notes,
                install_kind: kind,
                staged: None,
                installing: false,
                install_error: None,
                error: None,
            }
        }
        Err(e) => {
            log::info!("update check did not complete: {e:#}");
            UpdateCheck::failed(current, kind, format!("{e}"))
        }
    }
}

/// GET the latest-release JSON and pull out the tag and the notes.
///
/// Shells out to `curl` rather than using the HTTP client the updater plugin
/// brings: this is the call that runs unattended, on every launch and on a
/// timer, and keeping it a `curl` line is what makes "point a network monitor
/// at Agency" (privacy.html) a claim anyone can check in one read.
fn fetch_latest_release() -> Result<Release> {
    let out = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-time",
            &TIMEOUT_SECS.to_string(),
            // GitHub rejects API requests without a User-Agent.
            "--user-agent",
            "Agency-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            RELEASES_API,
        ])
        .output()
        .context("running curl for the update check")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr.trim();
        // curl exit 22 is the --fail path: a non-2xx response, which for this
        // endpoint most often means the repo is private or has no releases yet.
        return Err(anyhow!(
            "could not reach the releases feed{}",
            if detail.is_empty() { String::new() } else { format!(": {detail}") }
        ));
    }

    let body: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing the release feed")?;
    let tag = body
        .get("tag_name")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or_else(|| anyhow!("the release feed had no tag_name"))?;
    let notes = body
        .get("body")
        .and_then(|b| b.as_str())
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| b.to_string());
    Ok(Release { tag, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux(exe: &str) -> Probe<'_> {
        Probe {
            os: "linux",
            appimage: None,
            exe,
            pacman_db: false,
            dpkg_db: false,
            rpm_db: false,
            read_only: false,
        }
    }

    #[test]
    fn failed_check_reports_no_update_and_keeps_the_download_link() {
        let c = UpdateCheck::failed("0.1.0", InstallKind::MacApp, "offline".to_string());
        assert_eq!(c.current, "0.1.0");
        assert_eq!(c.latest, None);
        assert!(!c.update_available, "an unreachable feed must never claim an update");
        assert!(!c.can_install, "there is nothing to install when the check failed");
        assert_eq!(c.error.as_deref(), Some("offline"));
        assert_eq!(c.url, RELEASES_PAGE);
    }

    #[test]
    fn release_page_and_api_point_at_the_same_repo() {
        // The two URLs are written out separately; this catches one being
        // repointed at a new repo without the other.
        let repo = "TennnisAI/Agency";
        assert!(RELEASES_API.contains(repo));
        assert!(RELEASES_PAGE.contains(repo));
    }

    #[test]
    fn a_bundled_mac_app_is_self_updating_and_a_dev_build_is_not() {
        let bundled = Probe {
            os: "macos",
            appimage: None,
            exe: "/Applications/Agency.app/Contents/MacOS/Agency",
            pacman_db: false,
            dpkg_db: false,
            rpm_db: false,
            read_only: false,
        };
        assert_eq!(classify(&bundled), InstallKind::MacApp);
        // ./dev.sh runs the binary straight out of target/. Letting the updater
        // near it would replace the build under the developer.
        let dev = Probe { exe: "/Users/x/Agency/target/debug/Agency", ..bundled };
        assert_eq!(classify(&dev), InstallKind::Unknown);
        assert!(!InstallKind::Unknown.self_updating());
    }

    #[test]
    fn a_mac_app_macos_will_not_let_anything_write_to_is_not_self_updating() {
        // Opened straight from the mounted DMG.
        let dmg = Probe {
            os: "macos",
            appimage: None,
            exe: "/Volumes/Agency/Agency.app/Contents/MacOS/Agency",
            pacman_db: false,
            dpkg_db: false,
            rpm_db: false,
            read_only: true,
        };
        assert_eq!(classify(&dmg), InstallKind::MacReadOnly);
        assert!(!InstallKind::MacReadOnly.self_updating());
        assert_eq!(manual_hint(InstallKind::MacReadOnly, "0.3.0", "aarch64"), None);
        // A quarantined download run from App Translocation, caught by its path
        // even when statvfs could not say the mount is read-only.
        let translocated = Probe {
            exe:
                "/private/var/folders/x/T/AppTranslocation/1A2B/d/Agency.app/Contents/MacOS/Agency",
            read_only: false,
            ..dmg
        };
        assert_eq!(classify(&translocated), InstallKind::MacReadOnly);
        // /Volumes alone decides nothing: an app on a writable external drive
        // updates like any other.
        let external = Probe {
            exe: "/Volumes/Work/Applications/Agency.app/Contents/MacOS/Agency",
            read_only: false,
            ..dmg
        };
        assert_eq!(classify(&external), InstallKind::MacApp);
    }

    #[test]
    fn an_appimage_is_recognised_by_its_runtime_variable_not_its_path() {
        // The binary inside a mounted AppImage lives under a /tmp mount point
        // that says nothing about where the image itself is; $APPIMAGE does.
        let p = Probe {
            appimage: Some("/home/x/Apps/Agency_0.2.0_amd64.AppImage"),
            exe: "/tmp/.mount_Agencyab12/usr/bin/Agency",
            ..linux("")
        };
        assert_eq!(classify(&p), InstallKind::AppImage);
        assert!(InstallKind::AppImage.self_updating());
        // An empty $APPIMAGE is not an AppImage; it is a leaked variable.
        let empty = Probe { appimage: Some(""), exe: "/usr/bin/Agency", dpkg_db: true, ..p };
        assert_eq!(classify(&empty), InstallKind::Deb);
    }

    #[test]
    fn a_packaged_linux_install_names_its_package_manager_and_is_not_self_updating() {
        let deb = Probe { dpkg_db: true, ..linux("/usr/bin/Agency") };
        assert_eq!(classify(&deb), InstallKind::Deb);
        let rpm = Probe { rpm_db: true, ..linux("/usr/bin/Agency") };
        assert_eq!(classify(&rpm), InstallKind::Rpm);
        // An Arch box has a pacman db and may have a dpkg one beside it (dpkg
        // is packaged for Arch); pacman wins regardless.
        let arch = Probe { pacman_db: true, dpkg_db: true, ..linux("/usr/bin/Agency") };
        assert_eq!(classify(&arch), InstallKind::Pacman);
        for kind in [InstallKind::Deb, InstallKind::Rpm, InstallKind::Pacman] {
            assert!(!kind.self_updating(), "{kind:?} belongs to the package manager");
        }
        // Same databases, but a binary from a tarball is nobody's package.
        let loose = Probe { dpkg_db: true, ..linux("/home/x/bin/Agency") };
        assert_eq!(classify(&loose), InstallKind::Unknown);
    }

    #[test]
    fn manual_hints_name_the_file_the_release_actually_carries() {
        assert_eq!(
            manual_hint(InstallKind::Deb, "0.3.0", "x86_64").as_deref(),
            Some("sudo apt install ./Agency_0.3.0_amd64.deb")
        );
        assert_eq!(
            manual_hint(InstallKind::Deb, "0.3.0", "aarch64").as_deref(),
            Some("sudo apt install ./Agency_0.3.0_arm64.deb")
        );
        assert_eq!(
            manual_hint(InstallKind::Rpm, "0.3.0", "aarch64").as_deref(),
            Some("sudo dnf install ./Agency-0.3.0-1.aarch64.rpm")
        );
        assert_eq!(
            manual_hint(InstallKind::Pacman, "0.3.0", "x86_64").as_deref(),
            Some("git pull && makepkg -si"),
            "agency-bin is not on the AUR yet; a yay line fails with target not found"
        );
        // An install Agency updates itself has nothing to tell the user.
        assert_eq!(manual_hint(InstallKind::MacApp, "0.3.0", "aarch64"), None);
        assert_eq!(manual_hint(InstallKind::Unknown, "0.3.0", "x86_64"), None);
    }

    #[test]
    fn the_first_check_is_due_and_the_next_waits_the_interval() {
        let mut c = Cadence::default();
        assert!(c.due(1_000), "a launch with no attempt behind it checks now");
        c.record(1_000, true);
        assert!(!c.due(1_000 + CHECK_INTERVAL_SECS - 1));
        assert!(c.due(1_000 + CHECK_INTERVAL_SECS));
    }

    #[test]
    fn failures_back_off_and_a_success_clears_them() {
        let mut c = Cadence::default();
        c.record(0, false);
        assert_eq!(c.wait_secs(), RETRY_BASE_SECS);
        c.record(0, false);
        assert_eq!(c.wait_secs(), RETRY_BASE_SECS * 2);
        // Backoff never outgrows the ordinary interval: a machine that has been
        // offline for a day still checks within six hours of coming back.
        for _ in 0..40 {
            c.record(0, false);
        }
        assert_eq!(c.wait_secs(), CHECK_INTERVAL_SECS);
        c.record(0, true);
        assert_eq!(c.wait_secs(), CHECK_INTERVAL_SECS);
        assert_eq!(c.failures, 0);
    }

    #[test]
    fn a_backwards_clock_makes_the_check_due_rather_than_parking_it() {
        // A laptop that slept and woke with a corrected clock, or moved
        // timezone: without this the next check is however far the clock
        // jumped, which on a machine left open is forever.
        let mut c = Cadence::default();
        c.record(10_000, true);
        assert!(c.due(9_000));
    }

    fn found(latest: &str) -> UpdateCheck {
        UpdateCheck {
            current: "0.2.0".to_string(),
            latest: Some(latest.to_string()),
            update_available: true,
            url: RELEASES_PAGE.to_string(),
            notes: None,
            install_kind: InstallKind::AppImage,
            can_install: true,
            manual_hint: None,
            staged: None,
            installing: false,
            install_error: None,
            error: None,
        }
    }

    fn staged(version: &str) -> Install {
        Install { staged: Some(version.to_string()), ..Install::default() }
    }

    #[test]
    fn a_staged_release_is_not_offered_again_but_a_newer_one_is() {
        // Install, then "Restart later": the binary still says 0.2.0, and
        // without this the same 0.3.0 was offered (and downloaded) again.
        let same = found("0.3.0").with_install(&staged("0.3.0"));
        assert!(!same.update_available);
        assert!(!same.can_install);
        assert_eq!(same.staged.as_deref(), Some("0.3.0"));

        let newer = found("0.4.0").with_install(&staged("0.3.0"));
        assert!(newer.update_available, "a release past the staged one is still news");
        assert!(newer.can_install);
        assert_eq!(newer.staged.as_deref(), Some("0.3.0"));

        // Offline after installing: the restart is still owed.
        let offline = UpdateCheck::failed("0.2.0", InstallKind::MacApp, "offline".into())
            .with_install(&staged("0.3.0"));
        assert_eq!(offline.staged.as_deref(), Some("0.3.0"));
        assert!(!offline.update_available);

        assert_eq!(found("0.3.0").with_install(&Install::default()), found("0.3.0"));
    }

    #[test]
    fn a_running_install_takes_the_button_away_and_refuses_a_second_claim() {
        let mut install = Install::default();
        assert!(install.begin());
        assert!(!install.begin(), "two dialogs pressing Install must not start two downloads");
        let shown = found("0.3.0").with_install(&install);
        assert!(shown.installing);
        assert!(!shown.can_install);
        assert!(shown.update_available, "the release is still news until it is in place");

        install.finish(&Ok("0.3.0".to_string()));
        let shown = found("0.3.0").with_install(&install);
        assert!(!shown.installing);
        assert_eq!(shown.staged.as_deref(), Some("0.3.0"));
        assert!(install.begin(), "finishing releases the claim");
    }

    #[test]
    fn a_failed_install_is_reported_until_the_next_starts_and_keeps_what_was_staged() {
        let mut install = staged("0.3.0");
        assert!(install.begin());
        install.finish(&Err("signature did not verify".to_string()));
        let shown = found("0.4.0").with_install(&install);
        assert_eq!(shown.install_error.as_deref(), Some("signature did not verify"));
        assert!(shown.can_install, "a failed install can be tried again");
        assert_eq!(shown.staged.as_deref(), Some("0.3.0"));

        assert!(install.begin());
        assert_eq!(install.error, None, "a new attempt clears the old failure");
    }

    #[test]
    fn a_failed_check_never_replaces_a_good_one() {
        let good = found("0.3.0");
        let failed = UpdateCheck::failed("0.2.0", InstallKind::AppImage, "offline".into());
        assert_eq!(keep_known(Some(good.clone()), failed.clone()), good);
        // Nothing better to keep: the failure is the answer.
        assert_eq!(keep_known(None, failed.clone()), failed);
        assert_eq!(keep_known(Some(failed.clone()), failed.clone()), failed);
        // A good answer always replaces what came before it.
        assert_eq!(keep_known(Some(found("0.3.0")), found("0.4.0")), found("0.4.0"));
    }
}
