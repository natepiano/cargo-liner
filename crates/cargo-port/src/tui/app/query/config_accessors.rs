use crate::tui::app::App;
use crate::tui::config_state::Config;

impl App {
    /// Borrow the `Config` subsystem. Per the pass-through collapse plan
    /// (Phase 1), per-flag accessors live on `Config` itself; callers
    /// reach them via `app.config().<flag>()`.
    pub const fn config(&self) -> &Config { &self.config }
}
