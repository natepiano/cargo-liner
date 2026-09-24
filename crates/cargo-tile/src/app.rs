//! `App`: the state the framework borrows itself back through, per
//! [`AppContext`].

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::AppContext;
use tui_pane::AppIdentity;
use tui_pane::AttractHost;
use tui_pane::AttractMode;
use tui_pane::AttractSettings;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapEditContext;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::NoToastAction;
use tui_pane::SettingStep;
use tui_pane::SettingsHost;
use tui_pane::TerminalApp;
use tui_pane::Updates;
use tui_pane::VisualDeadline;

use crate::config::CargoTile;
use crate::config::LoadedConfig;
use crate::constants::KEYMAP_TOML_HEADER;
use crate::favorites_overlay::FavoritesOverlay;
use crate::favorites_overlay::FavoritesOverlayContent;
use crate::globals::AppGlobalAction;
use crate::interaction;
use crate::keymap;
use crate::probe::FrameLog;
use crate::progress::capture_roots::AccountCaptureDirectory;
use crate::render;
use crate::root_scan::SharedCaptureDirectory;
use crate::roster::Roster;
use crate::sccache::SccacheStats;
use crate::settings;
use crate::tiles::TileGrid;

/// The attract screen, writing its lines and timings to the frame log.
pub(crate) type Attract = tui_pane::Attract<FrameLog>;

/// App-pane sections the keymap overlay walks, in display order. Every
/// [`AppPaneId`] belongs here or its pane-local shortcuts go unlisted.
const APP_PANE_DISPLAY_ORDER: [AppPaneId; 5] = [
    AppPaneId::Main,
    AppPaneId::Attract(AttractMode::MovingBand),
    AppPaneId::Attract(AttractMode::MovingText),
    AppPaneId::Attract(AttractMode::Pixelate),
    AppPaneId::Favorites,
];

/// The panes this app supplies to the framework; one variant per
/// app-side pane. A new TUI grows by adding variants here, giving each
/// a `Pane<App>` host in [`crate::keymap`], and laying them out in
/// [`crate::render`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppPaneId {
    /// The one content pane this template starts with.
    Main,
    /// One per attract-screen animation. Not a pane in the sense of
    /// having a rectangle -- [`tui_pane::Attract`] draws over the whole
    /// terminal -- but a scope of its own, so each animation binds its
    /// own keys and `keymap.toml` keeps a table for each.
    Attract(AttractMode),
    /// The app-owned favorites modal and its local keymap scope.
    Favorites,
}

/// Position of the app-owned modal layer.
pub(crate) enum AppOverlay {
    /// No app modal is open.
    Closed,
    /// The favorites modal is open with its content and parameter snapshot.
    Favorites(OpenFavoritesOverlayState),
}

/// Content and current-parameter snapshot owned for one open favorites modal.
pub(crate) struct OpenFavoritesOverlayState {
    /// Display-ready rows or file-state diagnostic shown in the modal.
    pub(crate) content:            FavoritesOverlayContent,
    /// Attract parameters in effect when the modal opened or last resized.
    pub(crate) current_parameters: OpenFavoritesCurrentParameters,
}

/// Attract parameters in effect for the lifetime of an open favorites snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OpenFavoritesCurrentParameters {
    attract_settings: AttractSettings,
}

impl OpenFavoritesCurrentParameters {
    /// Whether a recognized favorite has the same parameters as the attract screen.
    pub(crate) fn matches(&self, attract_settings: AttractSettings) -> bool {
        self.attract_settings == attract_settings
    }
}

impl From<AttractSettings> for OpenFavoritesCurrentParameters {
    fn from(attract_settings: AttractSettings) -> Self { Self { attract_settings } }
}

/// How much of each command a cell spells out.
///
/// The chain above a command and the table under it are the same tree,
/// and either way what a row is worth reading for is the pid and the
/// name of what runs. The arguments below that are where a cell spends
/// most of its width -- a test suite driving cargo in a temporary
/// directory per case gives every row a different absolute manifest
/// path, which wraps three deep and says nothing the row's own pid does
/// not. So [`Short`](Self::Short) is where the display starts, and the
/// whole line is a key away.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ProcessTree {
    /// The tree entire, with each of its commands named and nothing
    /// more: `cargo mend` out of `cargo mend --manifest-path
    /// /var/folders/T/x/Cargo.toml --json`.
    #[default]
    Short,
    /// The same tree with every command line spelled out in full.
    Long,
}

impl ProcessTree {
    /// The other of the two, which is all the key that toggles it asks
    /// for.
    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Short => Self::Long,
            Self::Long => Self::Short,
        }
    }
}

/// Capture startup information that remains in Settings after its toast expires.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) enum CaptureStartupNotice {
    /// Startup leaves no outstanding capture notice.
    #[default]
    Quiet,
    /// At least one toolchain could not establish a working capture shim.
    InstallationFailed(String),
    /// A newer installed shim remains in place and this reader needs upgrading.
    NewerShimKept(String),
    /// Newer shims remain installed while other toolchains need installation repair.
    NewerShimKeptWithFailures {
        /// The newer shims kept and the reader upgrade instruction.
        kept:     String,
        /// Separate failures affecting other toolchains.
        failures: String,
    },
}

/// Top-level application state.
pub(crate) struct App {
    /// Framework state — overlays, panes, toasts, settings pane.
    pub(crate) framework:         Framework<Self>,
    /// Resolved bindings.
    ///
    /// Behind an [`Rc`] because dispatch needs `&Keymap<App>` and
    /// `&mut App` at once, and because rebinding a key in the keymap
    /// overlay replaces this whole map mid-dispatch.
    pub(crate) keymap:            Rc<Keymap<Self>>,
    /// Parsed `config.toml` and any parse error, surfaced in the
    /// settings overlay.
    pub(crate) loaded_config:     LoadedConfig,
    /// Theme-resolution note from startup (a configured theme id that
    /// no file or built-in supplies), surfaced in the settings overlay.
    pub(crate) startup_note:      Option<String>,
    /// Outstanding installation failures and kept newer shims, retained in Settings.
    pub(crate) capture_note:      CaptureStartupNotice,
    /// Latest worker observations; settings rendering performs no capture reads.
    pub(crate) root_status:       Vec<AccountCaptureDirectory>,
    pub(crate) shared_directory:  SharedCaptureDirectory,
    /// The commands the display is holding: what the last scan found,
    /// plus whatever has finished and is still fading out of it.
    pub(crate) roster:            Roster,
    /// The tile grid: how many cells the pane holds and the motion
    /// between one arrangement and the next.
    pub(crate) tiles:             TileGrid,
    /// What sccache last reported, for the summary cell's top border,
    /// and where the poll that refreshes it stands.
    pub(crate) sccache:           SccacheStats,
    /// Message shown on the keymap overlay's selected row after a
    /// rejected capture.
    inline_error:                 Option<String>,
    /// When the app started, for the status line's uptime segment.
    pub(crate) started:           Instant,
    /// Whether the display is taking new work in or being held still.
    pub(crate) updates:           Updates,
    /// The attract screen shown over the grid while nothing is running.
    pub(crate) attract:           Attract,
    /// App-owned modal for browsing attract-screen favorites.
    pub(crate) favorites_overlay: FavoritesOverlay,
    /// How much of each command a cell spells out.
    pub(crate) tree:              ProcessTree,
}

impl App {
    /// Build the app with the main pane focused and the keymap loaded.
    pub(crate) fn new(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
    ) -> Result<Self, KeymapError> {
        Self::new_with_keymap_path(loaded_config, startup_note, CargoTile::keymap_path())
    }

    fn new_with_keymap_path(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
        keymap_path: Option<PathBuf>,
    ) -> Result<Self, KeymapError> {
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        let keymap = keymap::build_keymap(&mut framework, keymap_path)?;
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            loaded_config,
            startup_note,
            capture_note: CaptureStartupNotice::Quiet,
            root_status: Vec::new(),
            shared_directory: SharedCaptureDirectory::default(),
            roster: Roster::new(),
            tiles: TileGrid::new(),
            sccache: SccacheStats::new(),
            inline_error: None,
            started: Instant::now(),
            updates: Updates::Live,
            attract: Attract::new(),
            favorites_overlay: FavoritesOverlay::default(),
            tree: ProcessTree::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Result<Self, KeymapError> {
        Self::new_with_keymap_path(
            LoadedConfig {
                config: crate::config::Config::default(),
                error:  None,
            },
            None,
            None,
        )
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

impl SettingsHost for App {
    fn step_setting(&mut self, step: SettingStep) { settings::cycle(self, step); }
}

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> { self.inline_error.as_deref() }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { &APP_PANE_DISPLAY_ORDER }
}

impl KeymapEditContext for App {
    type AppGlobals = AppGlobalAction;

    const KEYMAP_TOML_HEADER: &'static str = KEYMAP_TOML_HEADER;

    fn keymap_file_path(&self) -> Option<PathBuf> { CargoTile::keymap_path() }

    fn set_keymap_inline_error(&mut self, message: String) { self.inline_error = Some(message); }

    fn clear_keymap_inline_error(&mut self) { self.inline_error = None; }

    /// Rebuild from the file the editor just wrote rather than from
    /// `content`: the builder is the one place that knows how a scope
    /// resolves, and re-running it is what keeps a rebind and a
    /// hand-edited `keymap.toml` on the same path.
    fn reload_keymap(&mut self, _content: &str) {
        match keymap::build_keymap(&mut self.framework, CargoTile::keymap_path()) {
            Ok(keymap) => self.keymap = Rc::new(keymap),
            Err(error) => self.inline_error = Some(format!("keymap reload failed: {error}")),
        }
    }
}

impl TerminalApp for App {
    type Identity = CargoTile;
    type Probe = FrameLog;

    fn keymap(&self) -> Rc<Keymap<Self>> { Rc::clone(&self.keymap) }

    fn draw(&mut self, frame: &mut Frame, keymap: &Keymap<Self>) {
        render::draw(frame, self, keymap);
    }

    fn click(&mut self, position: Position) { interaction::handle_click(self, position); }

    /// Settling the window costs several round trips to the window
    /// server, which is far longer than a frame, and `terminal.draw` is
    /// no place to spend them.
    fn before_draw(&mut self) { self.attract.identify(); }

    fn visual_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        self.favorites_overlay.visual_deadline(now, frame_period)
    }

    /// The favorites modal owns every key while it is open: a key its
    /// scope does not bind is still its own, and disarms a pending
    /// delete rather than reaching a global.
    fn modal_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        if !self.favorites_overlay.is_open() {
            return KeyOutcome::Unhandled;
        }
        if keymap.dispatch_app_pane(AppPaneId::Favorites, bind, self) == KeyOutcome::Unhandled {
            self.favorites_overlay.handle_unmapped_key();
        }
        KeyOutcome::Consumed
    }

    /// An attract screen that is what the display is showing owns the
    /// keyboard, and it owns it ahead of everything else: the keys that
    /// steer it are the arrows and `+` `-`, which the grid underneath
    /// spends on focus and on opening and closing a tile. A band that
    /// could not be steered because a grid nobody can see moved its
    /// focus ring would not be steerable at all. Only the keys it
    /// actually binds are taken -- `q` still quits, `f` still freezes,
    /// and `a` gives the grid back.
    fn attract_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        tui_pane::dispatch_attract_key(self, keymap, bind)
    }

    fn resized(&mut self, area: Rect) { self.attract.record_terminal_resize(area); }

    /// The attract screen reclamps its parameters to the new size, so
    /// an open favorites table re-marks which row matches them.
    fn resize_settled(&mut self) {
        if !self.favorites_overlay.is_open() {
            return;
        }
        let current_parameters = self.attract.current_settings().into();
        self.favorites_overlay
            .refresh_current_parameters(current_parameters);
    }

    /// Never while the attract screen is up: the strip already paints
    /// every cell it covers, and a full repaint inside one frame of it
    /// shows as a tear.
    fn holds_full_repaint(&self) -> bool { self.attract.showing() }

    fn before_exit(&mut self) {
        self.attract
            .record_completed_backdrop_attempts_before_exit();
    }
}

/// Each attract animation's keys hang off an [`AppPaneId::Attract`] of
/// their own.
impl AttractHost for App {
    const MOVING_BAND_PANE: AppPaneId = AppPaneId::Attract(AttractMode::MovingBand);
    const MOVING_TEXT_PANE: AppPaneId = AppPaneId::Attract(AttractMode::MovingText);
    const PIXELATE_PANE: AppPaneId = AppPaneId::Attract(AttractMode::Pixelate);

    fn attract(&self) -> &Attract { &self.attract }

    fn attract_mut(&mut self) -> &mut Attract { &mut self.attract }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::App;

    #[test]
    fn roots_wait_for_worker_observations() {
        let app = App::new_for_test().expect("test app");
        assert!(app.root_status.is_empty());
    }
}
