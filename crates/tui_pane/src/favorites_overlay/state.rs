//! Whether the favorites overlay is open, and what it holds while it is.

use super::content::FavoritesOverlayContent;
use crate::AttractSettings;

/// Position of the favorites overlay.
pub(super) enum FavoritesOverlayState {
    /// The overlay is closed.
    Closed,
    /// The overlay is open with its content and parameter snapshot.
    Open(OpenFavoritesOverlayState),
}

/// Content and current-parameter snapshot owned for one open favorites overlay.
pub(super) struct OpenFavoritesOverlayState {
    /// Display-ready rows or file-state diagnostic shown in the overlay.
    pub(super) content:            FavoritesOverlayContent,
    /// Attract parameters in effect when the overlay opened or last resized.
    pub(super) current_parameters: OpenFavoritesCurrentParameters,
}

/// Attract parameters in effect for the lifetime of an open favorites snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OpenFavoritesCurrentParameters {
    attract_settings: AttractSettings,
}

impl OpenFavoritesCurrentParameters {
    /// Whether a recognized favorite has the same parameters as the attract screen.
    pub(super) fn matches(&self, attract_settings: AttractSettings) -> bool {
        self.attract_settings == attract_settings
    }
}

impl From<AttractSettings> for OpenFavoritesCurrentParameters {
    fn from(attract_settings: AttractSettings) -> Self { Self { attract_settings } }
}
