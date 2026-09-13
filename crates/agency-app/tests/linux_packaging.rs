//! The Linux packages are described in three places that nothing else ties
//! together: `tauri.conf.json` (the control file, the desktop file, the file
//! maps), `linux/build.agency.app.metainfo.xml` (what a software centre shows
//! once the package is installed) and `linux/copyright` (the Debian copyright
//! file). Each of these tests pins a pair that was, or could have been, wrong
//! in a shipped package without anything failing.
//!
//! Pure file reads: no repo, no daemon, no running app.

use serde_json::Value;
use std::path::Path;

const METAINFO: &str = include_str!("../linux/build.agency.app.metainfo.xml");
const COPYRIGHT: &str = include_str!("../linux/copyright");

fn conf() -> Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
}

/// Text of the first `<tag ...>text</tag>` in the metainfo, ignoring attributes.
fn element(tag: &str) -> &'static str {
    let open = format!("<{tag}");
    let start = METAINFO.find(&open).unwrap_or_else(|| panic!("metainfo has a <{tag}>"));
    let body = &METAINFO[start + open.len()..];
    let body = &body[body.find('>').unwrap() + 1..];
    body[..body.find(&format!("</{tag}>")).unwrap()].trim()
}

/// A software centre matches the metainfo to the package through the
/// component id, the desktop file and the binary. The desktop file is named
/// from productName and `Exec`/`Icon` from the main binary, so all three have
/// to agree with the config or the "installed" page shows a nameless entry
/// with a placeholder icon.
#[test]
fn metainfo_identifies_the_same_app_as_the_bundle() {
    let conf = conf();
    let identifier = conf["identifier"].as_str().unwrap();
    let product = conf["productName"].as_str().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_Agency")).file_name().unwrap().to_str().unwrap();

    assert_eq!(element("id"), identifier);
    assert_eq!(element("name"), product);
    assert_eq!(element("launchable"), format!("{product}.desktop"));
    assert_eq!(element("binary"), binary);
    assert_eq!(element("summary"), conf["bundle"]["shortDescription"].as_str().unwrap());
}

/// The newest `<release>` is what a software centre reports as the installed
/// version. `tauri.conf.json` names the package; a bump that forgets the
/// metainfo ships 0.1.2 described as 0.1.1.
#[test]
fn metainfo_newest_release_is_the_bundle_version() {
    let version = conf()["version"].as_str().unwrap().to_string();
    let releases = element("releases");
    let first = releases.split("<release ").nth(1).expect("metainfo has at least one <release>");
    let first_version = first.split("version=\"").nth(1).unwrap().split('"').next().unwrap();
    assert_eq!(first_version, version);
}

/// Every Linux bundle installs the metainfo under the path AppStream scans,
/// named after the component id, and the source it points at exists. The map
/// is keyed by an absolute path on the target, so a typo is a package that
/// builds fine and a software centre that never finds the file.
#[test]
fn every_linux_bundle_installs_the_metainfo() {
    let conf = conf();
    let identifier = conf["identifier"].as_str().unwrap();
    let dest = format!("/usr/share/metainfo/{identifier}.metainfo.xml");
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for bundle in ["deb", "rpm", "appimage"] {
        let files = conf["bundle"]["linux"][bundle]["files"]
            .as_object()
            .unwrap_or_else(|| panic!("bundle.linux.{bundle}.files is set"));
        let src = files[&dest]
            .as_str()
            .unwrap_or_else(|| panic!("bundle.linux.{bundle}.files installs {dest}"));
        assert!(manifest_dir.join(src).is_file(), "{bundle}: {src} is missing");
        for src in files.values() {
            let src = src.as_str().unwrap();
            assert!(manifest_dir.join(src).is_file(), "{bundle}: {src} is missing");
        }
    }
}

/// The glibc floor is written by hand in the config and the release script
/// checks it against what the binaries import; this only makes sure there is
/// one to check. It is deb-only on purpose: the rpm bundler hands each
/// `depends` string to the package as a bare name, so "glibc >= 2.34" there
/// would be a requirement on a package called "glibc >= 2.34" that nothing
/// provides, and an rpm that no system can install.
#[test]
fn deb_declares_a_glibc_floor() {
    let conf = conf();
    let depends = conf["bundle"]["linux"]["deb"]["depends"].as_array().unwrap();
    let floor = depends
        .iter()
        .filter_map(|d| d.as_str())
        .find_map(|d| d.strip_prefix("libc6 (>= "))
        .expect("bundle.linux.deb.depends has a `libc6 (>= x.y)` entry");
    assert!(floor.ends_with(')') && floor.trim_end_matches(')').contains('.'), "floor: {floor}");
    assert!(
        conf["bundle"]["linux"]["rpm"]["depends"].is_null(),
        "rpm depends cannot carry versions"
    );
}

/// The copyright file names the same contact the control file's Maintainer
/// does, which is the crate's `authors` entry.
#[test]
fn copyright_contact_is_the_maintainer() {
    let author = env!("CARGO_PKG_AUTHORS").split(':').next().unwrap();
    assert!(
        author.contains('<') && author.ends_with('>'),
        "authors[0] is `Name <email>`: {author}"
    );
    assert!(COPYRIGHT.contains(&format!("Upstream-Contact: {author}\n")));
}

/// Package descriptions are read by users, so the UI copy rule applies: no em
/// dashes. Also no trailing period on the summary, which is the desktop
/// file's `Comment` and the control file's one-line description.
#[test]
fn package_copy_follows_the_ui_rules() {
    let conf = conf();
    let short = conf["bundle"]["shortDescription"].as_str().unwrap();
    let long = conf["bundle"]["longDescription"].as_str().unwrap();
    for (name, text) in
        [("shortDescription", short), ("longDescription", long), ("metainfo", METAINFO)]
    {
        assert!(!text.contains('\u{2014}'), "{name} contains an em dash");
    }
    assert!(!short.ends_with('.'), "shortDescription ends with a period");
}
