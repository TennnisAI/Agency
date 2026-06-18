#[test]
fn crate_exposes_version() {
    assert!(!agency_core::version().is_empty());
}
