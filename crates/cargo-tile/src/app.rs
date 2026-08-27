//! `App`: the state the framework borrows itself back through, per
//! [`AppContext`].

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Keymap;
use tui_pane::KeymapEditContext;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::NoToastAction;
use tui_pane::ToastId;

use crate::attract::Attract;
use crate::attract::AttractMode;
use crate::config;
use crate::config::LoadedConfig;
use crate::constants::KEYMAP_TOML_HEADER;
use crate::favorites_overlay::FavoritesOverlay;
use crate::favorites_overlay::FavoritesOverlayContent;
use crate::globals::AppGlobalAction;
use crate::keymap;
use crate::roster::Roster;
use crate::sccache::SccacheStats;
use crate::terminal::ToastVisualSchedule;
use crate::terminal::VisualDeadline;
use crate::terminal::VisualFrameRequest;
use crate::tiles::TileGrid;

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
    /// having a rectangle -- [`crate::attract`] draws over the whole
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
    /// The favorites modal is open with this complete content state.
    Favorites(FavoritesOverlayContent),
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

/// Top-level application state.
pub(crate) struct App {
    /// Framework state — overlays, panes, toasts, settings pane.
    pub(crate) framework:         Framework<Self>,
    /// Timed-toast transitions that still require event-loop wakes.
    toast_visual_schedule:        ToastVisualSchedule,
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
        Self::new_with_keymap_path(loaded_config, startup_note, config::keymap_path())
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
            toast_visual_schedule: ToastVisualSchedule::default(),
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

    /// Record the rendered lifecycle of a newly-pushed timed toast.
    pub(crate) fn schedule_timed_toast(
        &mut self,
        toast_id: ToastId,
        pushed_at: Instant,
        visible_duration: Duration,
        body_text: &str,
        min_interior_lines: usize,
    ) {
        self.toast_visual_schedule.record_timed_toast(
            toast_id,
            pushed_at,
            visible_duration,
            body_text,
            min_interior_lines,
            self.framework.toasts.settings(),
        );
    }

    /// Earliest timed-toast transition that can require a frame.
    pub(crate) fn toast_visual_deadline(
        &self,
        now: Instant,
        frame_period: Duration,
    ) -> VisualDeadline {
        self.toast_visual_schedule.next_deadline(now, frame_period)
    }

    /// Advance timed-toast transitions and report whether to draw.
    pub(crate) fn toast_visual_frame_request(&mut self, now: Instant) -> VisualFrameRequest {
        self.toast_visual_schedule.request_frame(now)
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
