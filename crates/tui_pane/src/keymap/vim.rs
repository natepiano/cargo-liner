//! `VimMode`: builder-time flag controlling whether vim navigation
//! extras are appended to the keymap.

/// Whether vim-mode extras (`h` / `j` / `k` / `l` navigation and each
/// pane's `vim_extras()`) are appended to the keymap at build time.
///
/// Inert flag in this module — the `KeymapBuilder` consumes it in
/// Phase 9 to apply the extras after TOML overlay (so the user's
/// `[navigation]` table replacement does not disable vim-mode).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VimMode {
    /// Vim navigation extras are not applied. The default.
    #[default]
    Disabled,
    /// Append vim navigation extras (`h` / `j` / `k` / `l` to the four
    /// nav directions, plus each pane's `vim_extras()`) after TOML
    /// overlay.
    Enabled,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::VimMode;

    #[test]
    fn default_is_disabled() {
        assert_eq!(VimMode::default(), VimMode::Disabled);
    }
}
