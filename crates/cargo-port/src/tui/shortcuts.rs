/// The current input context, derived from app state.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum InputContext {
    ProjectList,
    DetailFields,
    DetailTargets,
    CiRuns,
    Toasts,
    Lints,
    Output,
    Finder,
    Settings,
    SettingsEditing,
    Keymap,
    KeymapAwaiting,
    KeymapConflict,
}

impl InputContext {
    /// Overlay contexts own total focus.
    pub const fn is_overlay(self) -> bool {
        matches!(
            self,
            Self::Finder
                | Self::Settings
                | Self::SettingsEditing
                | Self::Keymap
                | Self::KeymapAwaiting
                | Self::KeymapConflict
        )
    }
}
