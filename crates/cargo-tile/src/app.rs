//! `App`: the state the framework borrows itself back through, per
//! [`AppContext`].

use std::path::PathBuf;
use std::rc::Rc;
#[cfg(feature = "attract")]
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
#[cfg(feature = "attract")]
use ratatui::layout::Rect;
use tui_pane::AppContext;
use tui_pane::AppIdentity;
use tui_pane::Appearance;
#[cfg(feature = "attract")]
use tui_pane::AttractHost;
#[cfg(feature = "attract")]
use tui_pane::AttractMode;
use tui_pane::ColorMode;
#[cfg(feature = "attract")]
use tui_pane::FavoritesHost;
#[cfg(feature = "attract")]
use tui_pane::FavoritesOverlay;
use tui_pane::FocusedPane;
use tui_pane::Framework;
#[cfg(feature = "attract")]
use tui_pane::KeyBind;
#[cfg(feature = "attract")]
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapEditContext;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::NoToastAction;
use tui_pane::ScrollDirection;
use tui_pane::SettingStep;
use tui_pane::SettingsFileSpec;
use tui_pane::SettingsHost;
use tui_pane::TerminalApp;
use tui_pane::TileGridHost;
#[cfg(feature = "attract")]
pub(crate) use tui_pane::Updates;
#[cfg(feature = "attract")]
use tui_pane::VisualDeadline;

use crate::app_global_action::AppGlobalAction;
use crate::census::ExcludedCommands;
use crate::census::InvocationId;
#[cfg(test)]
use crate::census::cpu_observation;
use crate::config::CargoTile;
use crate::config::LoadedConfig;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::KEYMAP_TOML_HEADER;
use crate::constants::MIN_CELL_HEIGHT;
use crate::constants::MIN_CELL_WIDTH;
use crate::constants::PANIC_LOG_FILENAME;
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
#[cfg(feature = "attract")]
pub(crate) type Attract = tui_pane::Attract<FrameLog>;

/// App-pane sections the keymap overlay walks, in display order. Every
/// [`AppPaneId`] belongs here or its pane-local shortcuts go unlisted.
const APP_PANE_DISPLAY_ORDER: &[AppPaneId] = &[
    AppPaneId::Main,
    #[cfg(feature = "attract")]
    AppPaneId::Attract(AttractMode::MovingBand),
    #[cfg(feature = "attract")]
    AppPaneId::Attract(AttractMode::MovingText),
    #[cfg(feature = "attract")]
    AppPaneId::Attract(AttractMode::Pixelate),
    #[cfg(feature = "attract")]
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
    #[cfg(feature = "attract")]
    Attract(AttractMode),
    /// The app-owned favorites modal and its local keymap scope.
    #[cfg(feature = "attract")]
    Favorites,
}

/// The display can freeze without an attract screen.
#[cfg(not(feature = "attract"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Updates {
    /// Apply new scans and animation steps.
    Live,
    /// Keep the current display.
    Frozen,
}

#[cfg(not(feature = "attract"))]
impl Updates {
    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Live => Self::Frozen,
            Self::Frozen => Self::Live,
        }
    }
}

/// Whether command cells include the process ancestry above their rows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ProcessHierarchy {
    /// Reserve no ancestry rows.
    #[default]
    Hidden,
    /// Show the complete retained hierarchy.
    Shown,
}

impl ProcessHierarchy {
    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Hidden => Self::Shown,
            Self::Shown => Self::Hidden,
        }
    }
}

/// Argument detail is independent of whether process ancestry is shown.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ArgumentDetail {
    /// Subcommands plus the configured selection.
    #[default]
    Default,
    /// Every command-line argument.
    All,
}

impl ArgumentDetail {
    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Default => Self::All,
            Self::All => Self::Default,
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
    /// `commands.excluded` as the census worker reads it, replaced when
    /// the list is edited in Settings.
    pub(crate) excluded_commands: ExcludedCommands,
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
    #[cfg(feature = "attract")]
    pub(crate) attract:           Attract,
    /// App-owned modal for browsing attract-screen favorites.
    #[cfg(feature = "attract")]
    pub(crate) favorites_overlay: FavoritesOverlay,
    /// Whether command cells show process ancestry.
    pub(crate) tree:              ProcessHierarchy,
    /// Which command arguments each row shows.
    pub(crate) arguments:         ArgumentDetail,
}

impl App {
    /// Build the app with the main pane focused and the keymap loaded.
    pub(crate) fn new(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
    ) -> Result<Self, KeymapError> {
        Self::new_with_keymap_path(
            loaded_config,
            startup_note,
            CargoTile::keymap_path(),
            ColorMode::from_environment(),
        )
    }

    fn new_with_keymap_path(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
        keymap_path: Option<PathBuf>,
        color_mode: ColorMode,
    ) -> Result<Self, KeymapError> {
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        framework.color_mode = color_mode;
        let keymap = keymap::build_keymap(&mut framework, keymap_path)?;
        let excluded_commands =
            ExcludedCommands::new(loaded_config.config.commands.excluded.clone());
        let mut tiles = TileGrid::new();
        tiles.set_min_tile_height(MIN_CELL_HEIGHT);
        tiles.set_min_tile_width(MIN_CELL_WIDTH);
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            loaded_config,
            excluded_commands,
            startup_note,
            capture_note: CaptureStartupNotice::Quiet,
            root_status: Vec::new(),
            shared_directory: SharedCaptureDirectory::default(),
            roster: Roster::new(),
            tiles,
            sccache: SccacheStats::new(),
            inline_error: None,
            started: Instant::now(),
            updates: Updates::Live,
            #[cfg(feature = "attract")]
            attract: Attract::new(),
            #[cfg(feature = "attract")]
            favorites_overlay: FavoritesOverlay::default(),
            tree: ProcessHierarchy::default(),
            arguments: ArgumentDetail::default(),
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
            ColorMode::Colored,
        )
    }

    /// Remember a system appearance and select the configured theme for it.
    pub(crate) fn apply_system_appearance(&mut self, appearance: Appearance) {
        self.startup_note =
            tui_pane::apply_system_appearance(appearance, &self.loaded_config.config.appearance);
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

impl SettingsHost for App {
    fn settings_row_count(&self) -> usize { settings::rows(self).selectable_count() }

    fn step_setting(&mut self, step: SettingStep) { settings::cycle(self, step); }

    fn setting_text(&self) -> Option<String> { settings::selected_text(self) }

    fn commit_setting_text(&mut self, text: &str) { settings::commit_text(self, text); }
}

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> { self.inline_error.as_deref() }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { APP_PANE_DISPLAY_ORDER }
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
        self.framework.color_mode.clear_colors(frame.buffer_mut());
        #[cfg(test)]
        cpu_observation::publish_frame();
    }

    fn click(&mut self, position: Position) { tui_pane::handle_tile_click(self, position); }

    fn scroll(&mut self, position: Position, direction: ScrollDirection) {
        tui_pane::handle_tile_scroll(self, position, direction);
    }

    /// `<os config dir>/cargo-tile/panic.log`, beside `config.toml`.
    fn panic_log(&self) -> Option<PathBuf> {
        SettingsFileSpec::new(CONFIG_DIRNAME, PANIC_LOG_FILENAME).resolved_path()
    }

    /// Settling the window costs several round trips to the window
    /// server, which is far longer than a frame, and `terminal.draw` is
    /// no place to spend them.
    #[cfg(feature = "attract")]
    fn before_draw(&mut self) { self.attract.identify(); }

    #[cfg(feature = "attract")]
    fn visual_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        self.favorites_overlay.visual_deadline(now, frame_period)
    }

    /// The favorites modal owns every key while it is open: a key its
    /// scope does not bind is still its own, and disarms a pending
    /// delete rather than reaching a global.
    #[cfg(feature = "attract")]
    fn modal_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        tui_pane::dispatch_favorites_key(self, keymap, bind)
    }

    /// An attract screen that is what the display is showing owns the
    /// keyboard, and it owns it ahead of everything else: the keys that
    /// steer it are the arrows and `+` `-`, which the grid underneath
    /// spends on focus and on opening and closing a tile. A band that
    /// could not be steered because a grid nobody can see moved its
    /// focus ring would not be steerable at all. Only the keys it
    /// actually binds are taken -- `q` still quits, `f` still freezes,
    /// and `a` gives the grid back.
    #[cfg(feature = "attract")]
    fn attract_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        tui_pane::dispatch_attract_key(self, keymap, bind)
    }

    #[cfg(feature = "attract")]
    fn resized(&mut self, area: Rect) { self.attract.record_terminal_resize(area); }

    /// The attract screen reclamps its parameters to the new size, so
    /// an open favorites table re-marks which row matches them.
    #[cfg(feature = "attract")]
    fn resize_settled(&mut self) { tui_pane::favorites_resize_settled(self); }

    /// Never while the attract screen is up: the strip already paints
    /// every cell it covers, and a full repaint inside one frame of it
    /// shows as a tear.
    #[cfg(feature = "attract")]
    fn holds_full_repaint(&self) -> bool { self.attract.showing() }

    #[cfg(feature = "attract")]
    fn before_exit(&mut self) {
        self.attract
            .record_completed_backdrop_attempts_before_exit();
    }
}

/// The tile grid is the whole body, registered under [`AppPaneId::Main`].
impl TileGridHost for App {
    type TileId = InvocationId;

    const TILE_GRID_PANE: AppPaneId = AppPaneId::Main;

    fn tile_grid_mut(&mut self) -> &mut TileGrid { &mut self.tiles }
}

/// Each attract animation's keys hang off an [`AppPaneId::Attract`] of
/// their own.
#[cfg(feature = "attract")]
impl AttractHost for App {
    const MOVING_BAND_PANE: AppPaneId = AppPaneId::Attract(AttractMode::MovingBand);
    const MOVING_TEXT_PANE: AppPaneId = AppPaneId::Attract(AttractMode::MovingText);
    const PIXELATE_PANE: AppPaneId = AppPaneId::Attract(AttractMode::Pixelate);

    fn attract(&self) -> &Attract { &self.attract }

    fn attract_mut(&mut self) -> &mut Attract { &mut self.attract }
}

/// The favorites modal's keys hang off [`AppPaneId::Favorites`].
#[cfg(feature = "attract")]
impl FavoritesHost for App {
    const FAVORITES_PANE: AppPaneId = AppPaneId::Favorites;

    fn favorites_overlay(&self) -> &FavoritesOverlay { &self.favorites_overlay }

    fn favorites_overlay_mut(&mut self) -> &mut FavoritesOverlay { &mut self.favorites_overlay }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::layout::Position;
    use tui_pane::Appearance;
    use tui_pane::ColorMode;
    use tui_pane::GlobalAction;
    use tui_pane::Hittable;
    use tui_pane::ScrollDirection;
    use tui_pane::TerminalApp;

    use super::App;
    use crate::progress::capture_roots::AccountCaptureDirectory;

    #[test]
    fn no_color_frame_keeps_the_focused_title_bold() {
        let mut app = App::new_for_test().expect("test app");
        app.framework.color_mode = ColorMode::NoColor;
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24))
            .expect("test terminal");
        terminal
            .draw(|frame| app.draw(frame, &keymap))
            .expect("app frame");
        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.fg == ratatui::style::Color::Reset
                    && cell.bg == ratatui::style::Color::Reset),
            "every frame clears colours"
        );
        let title = (0..buffer.area.width)
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol().chars().any(char::is_alphabetic))
            .expect("focused title");
        assert!(
            title.modifier.contains(ratatui::style::Modifier::BOLD),
            "the focused title stays bold"
        );
    }

    #[test]
    fn shortcuts_overlay_wheel_scrolls_and_keeps_tiles_still() {
        let mut app = App::new_for_test().expect("test app");
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 16))
            .expect("test terminal");
        terminal
            .draw(|frame| app.draw(frame, &keymap))
            .expect("tile frame");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| app.draw(frame, &keymap))
            .expect("settled tile frame");
        let tile_before = app.tiles.hit_test_at(Position::new(1, 1));

        keymap.dispatch_framework_global(GlobalAction::OpenGlobalShortcuts, &mut app);
        terminal
            .draw(|frame| app.draw(frame, &keymap))
            .expect("overlay frame");
        let viewport = app.framework.global_shortcuts_pane.viewport();
        assert!(
            viewport.content_height() > viewport.visible_rows(),
            "overlay must overflow"
        );
        let content = viewport.content_area();
        let inside = Position::new(content.x, content.y);
        let outside = Position::new(1, 1);
        assert!(!content.contains(outside));
        let start = viewport.pos();
        let offset = viewport.scroll_offset();
        app.scroll(outside, ScrollDirection::Down);
        assert_eq!(app.framework.global_shortcuts_pane.viewport().pos(), start);
        assert_eq!(
            app.framework
                .global_shortcuts_pane
                .viewport()
                .scroll_offset(),
            offset
        );

        app.scroll(inside, ScrollDirection::Down);
        assert_eq!(
            app.framework.global_shortcuts_pane.viewport().pos(),
            start + 1
        );
        app.scroll(inside, ScrollDirection::Up);
        assert_eq!(app.framework.global_shortcuts_pane.viewport().pos(), start);
        let steps = app.framework.global_shortcuts_pane.viewport().len();
        for _ in 0..steps {
            app.scroll(inside, ScrollDirection::Down);
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("wheel down frame");
        }
        assert!(
            app.framework
                .global_shortcuts_pane
                .viewport()
                .scroll_offset()
                > offset
        );
        for _ in 0..steps {
            app.scroll(inside, ScrollDirection::Up);
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("wheel up frame");
        }
        assert_eq!(
            app.framework
                .global_shortcuts_pane
                .viewport()
                .scroll_offset(),
            offset
        );
        assert_eq!(app.framework.global_shortcuts_pane.viewport().pos(), start);
        assert_eq!(app.tiles.hit_test_at(outside), tile_before);
    }

    #[test]
    fn roots_wait_for_worker_observations() {
        let app = App::new_for_test().expect("test app");
        assert_eq!(app.root_status, [] as [AccountCaptureDirectory; 0]);
    }

    #[test]
    fn applying_an_appearance_replaces_the_theme_notice() {
        let mut app = App::new_for_test().expect("test app");
        app.startup_note = Some("earlier notice".to_string());
        app.loaded_config.config.appearance.light_theme = "Missing Light".to_string();

        app.apply_system_appearance(Appearance::Light);

        assert_eq!(
            app.startup_note.as_deref(),
            Some("theme `Missing Light` not found — using a built-in")
        );
    }
}
