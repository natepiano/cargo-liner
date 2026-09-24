//! An app that carries the favorites overlay the way a real one does,
//! for the overlay's own tests: a main pane, the three attract scopes,
//! the favorites scope, and the save and open globals under the TOML
//! names the [`FavoritesHost`] contract asks for.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tempfile::TempDir;

use super::FavoritesHost;
use super::FavoritesOverlay;
use super::FavoritesOverlayPane;
use super::dispatch_favorites_key;
use super::favorites_resize_settled;
use super::globals;
use crate::AppContext;
use crate::AppIdentity;
use crate::Attract;
use crate::AttractHost;
use crate::AttractMode;
use crate::Bindings;
use crate::FocusedPane;
use crate::Framework;
use crate::Globals;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Keymap;
use crate::KeymapEditContext;
use crate::KeymapError;
use crate::KeymapUiContext;
use crate::Mode;
use crate::MovingBandPane;
use crate::MovingTextPane;
use crate::NoProbe;
use crate::NoToastAction;
use crate::Pane;
use crate::PixelatePane;
use crate::SettingStep;
use crate::SettingsHost;
use crate::SettingsNavigation;
use crate::TerminalApp;
use crate::Updates;
use crate::VisualDeadline;

/// App-pane sections the keymap overlay walks, in display order.
const TEST_PANE_DISPLAY_ORDER: [TestPaneId; 5] = [
    TestPaneId::Main,
    TestPaneId::Attract(AttractMode::MovingBand),
    TestPaneId::Attract(AttractMode::MovingText),
    TestPaneId::Attract(AttractMode::Pixelate),
    TestPaneId::Favorites,
];

/// The test app's panes: a main pane, one scope per attract animation,
/// and the favorites scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum TestPaneId {
    Main,
    Attract(AttractMode),
    Favorites,
}

crate::action_enum! {
    /// The test app's globals, named as the [`FavoritesHost`] contract asks.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(super) enum TestGlobalAction {
        /// Save the attract parameters on screen.
        SaveFavorite => ("save_favorite", "Save attract parameters");
        /// Open the saved favorites.
        OpenFavorites => ("open_favorites", "Open attract favorites");
    }
}

impl Globals<TestApp> for TestGlobalAction {
    type Actions = Self;

    fn render_order() -> &'static [Self::Actions] { <Self as crate::Action>::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! {
            KeyBind::ctrl('s') => Self::SaveFavorite,
            KeyBind::ctrl('o') => Self::OpenFavorites,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
        |action, app| match action {
            Self::SaveFavorite => globals::save_favorite(app),
            Self::OpenFavorites => globals::open_favorites(app),
        }
    }
}

/// Where the test app would keep its files.
pub(super) enum TestIdentity {}

impl AppIdentity for TestIdentity {
    const BINARY_NAME: &'static str = "tui-pane-favorites-test";
    const CONFIG_DIRNAME: &'static str = "tui-pane-favorites-test";
    const DEFAULT_LIGHT_THEME: &'static str = "light";
    const DEFAULT_DARK_THEME: &'static str = "dark";
}

/// The content pane under the overlay.
struct MainPane;

impl Pane<TestApp> for MainPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Main;

    fn mode() -> fn(&TestApp) -> Mode<TestApp> { |_app| Mode::Static }
}

/// An app carrying the attract screen and the favorites overlay, with
/// the same field names as a real host so moved test bodies read the
/// same.
pub(super) struct TestApp {
    pub(super) framework:         Framework<Self>,
    pub(super) keymap:            Rc<Keymap<Self>>,
    pub(super) attract:           Attract<NoProbe>,
    pub(super) favorites_overlay: FavoritesOverlay,
    pub(super) updates:           Updates,
}

impl TestApp {
    /// Build the app with the main pane focused and the default keymap.
    pub(super) fn new_for_test() -> Result<Self, KeymapError> {
        let mut framework = Framework::new(FocusedPane::App(TestPaneId::Main));
        let keymap = build_keymap(&mut framework, None)?;
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            attract: Attract::new(),
            favorites_overlay: FavoritesOverlay::default(),
            updates: Updates::Live,
        })
    }
}

/// Assemble the test app's keymap, reading `keymap_path` when it names
/// a file, in the registration order a real host uses.
fn build_keymap(
    framework: &mut Framework<TestApp>,
    keymap_path: Option<PathBuf>,
) -> Result<Keymap<TestApp>, KeymapError> {
    let mut builder = Keymap::builder().ignore_unknown_entries();
    if let Some(path) = keymap_path {
        builder = builder.config_path(path.clone());
        if path.is_file() {
            builder = builder.load_toml(path)?;
        }
    }
    builder
        .register_navigation::<SettingsNavigation<TestApp>>()?
        .register_globals::<TestGlobalAction>()?
        .register_overlay()?
        .register_pane::<MainPane>()
        .register(MovingBandPane::<TestApp>::default())
        .register(MovingTextPane::<TestApp>::default())
        .register(PixelatePane::<TestApp>::default())
        .register(FavoritesOverlayPane::<TestApp>::default())
        .build_into(framework)
}

/// The test app's keymap with `toml` as its `keymap.toml`; empty means
/// no file, so every default holds.
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
pub(super) fn keymap_from(toml: &str) -> Keymap<TestApp> {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("keymap.toml");
    if !toml.is_empty() {
        fs::write(&path, toml).expect("test keymap should be written");
    }
    let mut framework = Framework::new(FocusedPane::App(TestPaneId::Main));
    build_keymap(&mut framework, (!toml.is_empty()).then_some(path))
        .expect("test keymap should resolve")
}

impl AppContext for TestApp {
    type AppPaneId = TestPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

impl SettingsHost for TestApp {
    fn step_setting(&mut self, _step: SettingStep) {}
}

impl KeymapUiContext for TestApp {
    fn keymap_inline_error(&self) -> Option<&str> { None }

    fn keymap_pane_display_order(&self) -> &[TestPaneId] { &TEST_PANE_DISPLAY_ORDER }
}

impl KeymapEditContext for TestApp {
    type AppGlobals = TestGlobalAction;

    const KEYMAP_TOML_HEADER: &'static str = "";

    fn keymap_file_path(&self) -> Option<PathBuf> { None }

    fn set_keymap_inline_error(&mut self, _message: String) {}

    fn clear_keymap_inline_error(&mut self) {}

    fn reload_keymap(&mut self, _content: &str) {}
}

impl TerminalApp for TestApp {
    type Identity = TestIdentity;
    type Probe = NoProbe;

    fn keymap(&self) -> Rc<Keymap<Self>> { Rc::clone(&self.keymap) }

    fn draw(&mut self, _frame: &mut Frame, _keymap: &Keymap<Self>) {}

    fn click(&mut self, _position: Position) {}

    fn visual_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        self.favorites_overlay.visual_deadline(now, frame_period)
    }

    fn modal_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        dispatch_favorites_key(self, keymap, bind)
    }

    fn attract_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        crate::dispatch_attract_key(self, keymap, bind)
    }

    fn resized(&mut self, area: Rect) { self.attract.record_terminal_resize(area); }

    fn resize_settled(&mut self) { favorites_resize_settled(self); }
}

impl AttractHost for TestApp {
    const MOVING_BAND_PANE: TestPaneId = TestPaneId::Attract(AttractMode::MovingBand);
    const MOVING_TEXT_PANE: TestPaneId = TestPaneId::Attract(AttractMode::MovingText);
    const PIXELATE_PANE: TestPaneId = TestPaneId::Attract(AttractMode::Pixelate);

    fn attract(&self) -> &Attract<NoProbe> { &self.attract }

    fn attract_mut(&mut self) -> &mut Attract<NoProbe> { &mut self.attract }
}

impl FavoritesHost for TestApp {
    const FAVORITES_PANE: TestPaneId = TestPaneId::Favorites;

    fn favorites_overlay(&self) -> &FavoritesOverlay { &self.favorites_overlay }

    fn favorites_overlay_mut(&mut self) -> &mut FavoritesOverlay { &mut self.favorites_overlay }
}
