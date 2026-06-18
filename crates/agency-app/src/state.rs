pub struct AppState;

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}
