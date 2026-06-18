#[test]
fn app_lib_exposes_state_version() {
    assert_eq!(agency_app_lib::AppState::version(), "0.1.0");
}
