/// The bundle version and the crate version have to be the same string: the
/// DMG is named from `tauri.conf.json`, while `AppState::version()` (which the
/// update check compares against the latest release tag, and which stamps the
/// registry's pre-migration backup) reads `CARGO_PKG_VERSION`. Nothing else
/// ties the two together, so a release that bumps one and forgets the other
/// ships an app that reports a version its own DMG does not carry. Pinning a
/// literal here instead just moved the forgetting one file over.
#[test]
fn app_lib_version_matches_the_bundle() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let bundle = conf["version"].as_str().expect("tauri.conf.json has a version");
    assert_eq!(agency_app_lib::AppState::version(), bundle);
}
