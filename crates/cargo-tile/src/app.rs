//! `App`: the state the framework borrows itself back through, per
//! [`AppContext`].

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Keymap;
use tui_pane::KeymapEditContext;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::NoToastAction;

use crate::attract::Attract;
use crate::config;
use crate::config::LoadedConfig;
use crate::constants::KEYMAP_TOML_HEADER;
use crate::globals::AppGlobalAction;
use crate::keymap;
use crate::roster::Roster;
use crate::sccache::SccacheStats;
use crate::tiles::TileGrid;

/// App-pane sections the keymap overlay walks, in display order. Every
/// [`AppPaneId`] belongs here or its pane-local shortcuts go unlisted.
const APP_PANE_DISPLAY_ORDER: [AppPaneId; 1] = [AppPaneId::Main];

/// The panes this app supplies to the framework; one variant per
/// app-side pane. A new TUI grows by adding variants here, giving each
/// a `Pane<App>` host in [`crate::keymap`], and laying them out in
/// [`crate::render`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppPaneId {
    /// The one content pane this template starts with.
    Main,
}

/// Whether the display takes new work in as it arrives or is being
/// held still.
///
/// Held, nothing is folded in: no scan, no fade, no cell in motion.
/// Reading a screen that repaints four times a second is what this is
/// for -- a pid pairs off with its parent far more easily when neither
/// of them is about to move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Updates {
    /// Every scan, fade and step is folded in as it reaches the loop.
    Live,
    /// Nothing is folded in until the reader lets go.
    Frozen,
}

impl Updates {
    /// The other of the two, which is all the key that holds the
    /// display asks for.
    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Live => Self::Frozen,
            Self::Frozen => Self::Live,
        }
    }
}

/// Top-level application state.
pub(crate) struct App {
    /// Framework state — overlays, panes, toasts, settings pane.
    pub(crate) framework:     Framework<Self>,
    /// Resolved bindings.
    ///
    /// Behind an [`Rc`] because dispatch needs `&Keymap<App>` and
    /// `&mut App` at once, and because rebinding a key in the keymap
    /// overlay replaces this whole map mid-dispatch.
    pub(crate) keymap:        Rc<Keymap<Self>>,
    /// Parsed `config.toml` and any parse error, surfaced in the
    /// settings overlay.
    pub(crate) loaded_config: LoadedConfig,
    /// Theme-resolution note from startup (a configured theme id that
    /// no file or built-in supplies), surfaced in the settings overlay.
    pub(crate) startup_note:  Option<String>,
    /// The commands the display is holding: what the last scan found,
    /// plus whatever has finished and is still fading out of it.
    pub(crate) roster:        Roster,
    /// The tile grid: how many cells the pane holds and the motion
    /// between one arrangement and the next.
    pub(crate) tiles:         TileGrid,
    /// What sccache last reported, for the summary cell's top border,
    /// and where the poll that refreshes it stands.
    pub(crate) sccache:       SccacheStats,
    /// Message shown on the keymap overlay's selected row after a
    /// rejected capture.
    inline_error:             Option<String>,
    /// When the app started, for the status line's uptime segment.
    pub(crate) started:       Instant,
    /// Whether the display is taking new work in or being held still.
    pub(crate) updates:       Updates,
    /// The attract screen shown over the grid while nothing is running.
    pub(crate) attract:       Attract,
}

impl App {
    /// Build the app with the main pane focused and the keymap loaded.
    pub(crate) fn new(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
    ) -> Result<Self, KeymapError> {
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        let keymap = keymap::build_keymap(&mut framework, config::keymap_path())?;
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            loaded_config,
            startup_note,
            roster: Roster::new(),
            tiles: TileGrid::new(),
            sccache: SccacheStats::new(),
            inline_error: None,
            started: Instant::now(),
            updates: Updates::Live,
            attract: Attract::new(),
        })
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> { self.inline_error.as_deref() }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { &APP_PANE_DISPLAY_ORDER }
}

impl KeymapEditContext for App {
    type AppGlobals = AppGlobalAction;

    const KEYMAP_TOML_HEADER: &'static str = KEYMAP_TOML_HEADER;

    fn keymap_file_path(&self) -> Option<PathBuf> { config::keymap_path() }

    fn set_keymap_inline_error(&mut self, message: String) { self.inline_error = Some(message); }

    fn clear_keymap_inline_error(&mut self) { self.inline_error = None; }

    /// Rebuild from the file the editor just wrote rather than from
    /// `content`: the builder is the one place that knows how a scope
    /// resolves, and re-running it is what keeps a rebind and a
    /// hand-edited `keymap.toml` on the same path.
    fn reload_keymap(&mut self, _content: &str) {
        match keymap::build_keymap(&mut self.framework, config::keymap_path()) {
            Ok(keymap) => self.keymap = Rc::new(keymap),
            Err(error) => self.inline_error = Some(format!("keymap reload failed: {error}")),
        }
    }
}
