use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::tui::app::App;

impl App {
    pub const fn lint_enabled(&self) -> bool { self.config.current().lint.enabled }

    pub const fn invert_scroll(&self) -> ScrollDirection {
        self.config.current().mouse.invert_scroll
    }

    pub const fn include_non_rust(&self) -> NonRustInclusion {
        self.config.current().tui.include_non_rust
    }

    pub const fn ci_run_count(&self) -> u32 { self.config.current().tui.ci_run_count }

    pub const fn navigation_keys(&self) -> NavigationKeys {
        self.config.current().tui.navigation_keys
    }

    pub fn editor(&self) -> &str { &self.config.current().tui.editor }

    pub fn terminal_command(&self) -> &str { &self.config.current().tui.terminal_command }

    pub fn terminal_command_configured(&self) -> bool { !self.terminal_command().trim().is_empty() }
}
