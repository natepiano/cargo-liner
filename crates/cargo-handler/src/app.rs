//! `App`: the state the framework borrows itself back through, per
//! [`AppContext`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::AppContext;
use tui_pane::AppIdentity;
use tui_pane::Appearance;
use tui_pane::AttractHost;
use tui_pane::AttractMode;
use tui_pane::CellClick;
use tui_pane::FavoritesHost;
use tui_pane::FavoritesOverlay;
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
use tui_pane::TileGridHost;
use tui_pane::VisualDeadline;

use crate::agent_cell;
use crate::census;
use crate::census::AgentPlacement;
use crate::census::Census;
#[cfg(test)]
use crate::census::CensusUpdate;
use crate::census::HierarchyCommand;
#[cfg(test)]
use crate::census::HierarchyEdges;
use crate::census::HierarchySource;
use crate::census::RemoteMachines;
use crate::census::accounts::AccountsMonitor;
#[cfg(test)]
use crate::census::scan::ScanStage;
use crate::config::CargoHandler;
use crate::config::LoadedConfig;
use crate::constants::KEYMAP_TOML_HEADER;
use crate::footer_colors::FooterColorFile;
use crate::globals::AppGlobalAction;
use crate::keymap;
use crate::render;
use crate::settings;
use crate::sort::AccountsSort;
use crate::sort::SummarySort;
use crate::sort::TreeSort;
use crate::startup_log;
use crate::startup_log::StartupProbe;
#[cfg(test)]
use crate::tiles::AgentCell;
use crate::tiles::TileGrid;
use crate::tiles::TileKey;

/// The attract screen, sharing the handler's optional startup timing log.
pub(crate) type Attract = tui_pane::Attract<StartupProbe>;

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
/// app-side pane.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppPaneId {
    /// The tile grid, which is the whole body.
    Main,
    /// One per attract-screen animation. Not a pane in the sense of
    /// having a rectangle -- [`tui_pane::Attract`] draws over the whole
    /// terminal -- but a scope of its own, so each animation binds its
    /// own keys and `keymap.toml` keeps a table for each.
    Attract(AttractMode),
    /// The favorites modal and its local keymap scope.
    Favorites,
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
    /// The tile grid: how many cells the pane holds and the motion
    /// between one arrangement and the next.
    pub(crate) tiles:             TileGrid,
    /// Per-root preferences, discarded when that root leaves the census.
    pub(crate) tree_sorts:        HashMap<TileKey, TreeSort>,
    /// One preference shared by the summary machine groups.
    pub(crate) summary_sort:      SummarySort,
    /// Account ordering is independent of the machine-group agent ordering.
    pub(crate) accounts_sort:     AccountsSort,
    /// Message shown on the keymap overlay's selected row after a
    /// rejected capture.
    inline_error:                 Option<String>,
    /// When the app started, for the status line's uptime segment.
    pub(crate) started:           Instant,
    /// The attract screen, which comes on over the grid after a quiet
    /// spell and on `a`.
    pub(crate) attract:           Attract,
    /// Modal for browsing attract-screen favorites.
    pub(crate) favorites_overlay: FavoritesOverlay,
    /// The agents on this machine and each remote, as of each one's
    /// latest answer.
    pub(crate) census:            Census,
    /// Account readings refreshed independently of the frame and census scan.
    pub(crate) accounts:          AccountsMonitor,
    /// Footer colors cached between census scans.
    pub(crate) footer_colors:     FooterColorFile,
    /// `machines.remote` as the census scheduler reads it.
    pub(crate) remote_machines:   RemoteMachines,
    /// `census.hierarchy_command` as the census scheduler reads it.
    pub(crate) hierarchy_command: HierarchyCommand,
}

impl App {
    /// Discard preferences for session or machine-pool tiles no longer present.
    pub(crate) fn prune_tree_sorts(&mut self) {
        let machines = self
            .census
            .machines(&self.loaded_config.config.machines.remote);
        let placement = AgentPlacement::from_machines(&machines);
        let keys = agent_cell::tile_keys(&placement);
        self.tree_sorts.retain(|id, _| keys.contains(id));
    }

    /// Build the app with the grid focused and the keymap loaded.
    pub(crate) fn new(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
    ) -> Result<Self, KeymapError> {
        let census = Census::new(census::local_machine_name());
        let mut app = Self::new_with_keymap_path(
            loaded_config,
            startup_note,
            CargoHandler::keymap_path(),
            census,
        )?;
        app.footer_colors = FooterColorFile::load();
        Ok(app)
    }

    /// Apply a system appearance reported by the background watcher.
    pub(crate) fn apply_system_appearance(&mut self, appearance: Appearance) {
        self.startup_note =
            tui_pane::apply_system_appearance(appearance, &self.loaded_config.config.appearance);
    }

    /// Build the app with the keymap read from `keymap_path`, or from
    /// the defaults alone when there is none, starting from `census`.
    fn new_with_keymap_path(
        loaded_config: LoadedConfig,
        startup_note: Option<String>,
        keymap_path: Option<PathBuf>,
        census: Census,
    ) -> Result<Self, KeymapError> {
        let remote_machines = RemoteMachines::new(loaded_config.config.machines.remote.clone());
        let hierarchy_command = HierarchyCommand::new(HierarchySource::from(
            loaded_config.config.census.hierarchy_command.clone(),
        ));
        let accounts = census::accounts::AccountsMonitor::new(
            loaded_config.config.census.accounts_command.clone(),
        );
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        let keymap = keymap::build_keymap(&mut framework, keymap_path)?;
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            loaded_config,
            startup_note,
            tiles: TileGrid::new(),
            tree_sorts: HashMap::new(),
            summary_sort: SummarySort::default(),
            accounts_sort: AccountsSort::SourceOrder,
            inline_error: None,
            started: Instant::now(),
            attract: Attract::new(),
            favorites_overlay: FavoritesOverlay::default(),
            census,
            accounts,
            remote_machines,
            hierarchy_command,
            footer_colors: FooterColorFile::default(),
        })
    }

    /// An app on the default config and the default keymap, reading
    /// and writing no file, whose census is this machine as `natedev`,
    /// scanned and holding no agents, with no remotes.
    #[cfg(test)]
    pub(crate) fn new_for_test() -> Result<Self, KeymapError> {
        Self::new_with_keymap_path(
            LoadedConfig {
                config: crate::config::Config::default(),
                error:  None,
            },
            None,
            None,
            {
                let mut census = Census::new("natedev".to_string());
                census.apply(CensusUpdate::Local {
                    rows:      Vec::new(),
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Classified,
                });
                census
            },
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

    fn setting_text(&self) -> Option<String> { settings::selected_text(self) }

    fn commit_setting_text(&mut self, text: &str) { settings::commit_text(self, text); }
}

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> { self.inline_error.as_deref() }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { &APP_PANE_DISPLAY_ORDER }
}

impl KeymapEditContext for App {
    type AppGlobals = AppGlobalAction;

    const KEYMAP_TOML_HEADER: &'static str = KEYMAP_TOML_HEADER;

    fn keymap_file_path(&self) -> Option<PathBuf> { CargoHandler::keymap_path() }

    fn set_keymap_inline_error(&mut self, message: String) { self.inline_error = Some(message); }

    fn clear_keymap_inline_error(&mut self) { self.inline_error = None; }

    /// Rebuild from the file the editor just wrote rather than from
    /// `content`: the builder is the one place that knows how a scope
    /// resolves, and re-running it is what keeps a rebind and a
    /// hand-edited `keymap.toml` on the same path.
    fn reload_keymap(&mut self, _content: &str) {
        match keymap::build_keymap(&mut self.framework, CargoHandler::keymap_path()) {
            Ok(keymap) => self.keymap = Rc::new(keymap),
            Err(error) => self.inline_error = Some(format!("keymap reload failed: {error}")),
        }
    }
}

impl TerminalApp for App {
    type Identity = CargoHandler;
    type Probe = StartupProbe;

    fn keymap(&self) -> Rc<Keymap<Self>> { Rc::clone(&self.keymap) }

    fn draw(&mut self, frame: &mut Frame, keymap: &Keymap<Self>) {
        render::draw(frame, self, keymap);
    }

    fn click(&mut self, position: Position) { tui_pane::handle_tile_click(self, position); }

    /// Settling the window costs several round trips to the window
    /// server, which is far longer than a frame, and `terminal.draw` is
    /// no place to spend them.
    fn before_draw(&mut self) {
        if self.attract.showing() {
            startup_log::startup_event("identify_start");
            self.attract.identify();
            startup_log::startup_event("identify_end");
        }
    }

    fn visual_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        self.favorites_overlay.visual_deadline(now, frame_period)
    }

    /// The favorites modal owns every key while it is open: a key its
    /// scope does not bind is still its own, and disarms a pending
    /// delete rather than reaching a global.
    fn modal_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        tui_pane::dispatch_favorites_key(self, keymap, bind)
    }

    /// An attract screen that is what the display is showing owns the
    /// keys it binds, ahead of the grid's: the arrows and `+` `-` steer
    /// it rather than moving a focus ring nobody can see. Only the keys
    /// it binds are taken -- `q` still quits, and `a` gives the grid
    /// back.
    fn attract_key(&mut self, keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
        tui_pane::dispatch_attract_key(self, keymap, bind)
    }

    fn resized(&mut self, area: Rect) { self.attract.record_terminal_resize(area); }

    /// The attract screen reclamps its parameters to the new size, so
    /// an open favorites table re-marks which row matches them.
    fn resize_settled(&mut self) { tui_pane::favorites_resize_settled(self); }

    /// Never while the attract screen is up: the strip already paints
    /// every cell it covers, and a full repaint inside one frame of it
    /// shows as a tear.
    fn holds_full_repaint(&self) -> bool { self.attract.showing() }

    fn before_exit(&mut self) {
        self.attract
            .record_completed_backdrop_attempts_before_exit();
    }
}

/// The tile grid is the whole body, registered under [`AppPaneId::Main`].
impl TileGridHost for App {
    type TileId = TileKey;

    const TILE_GRID_PANE: AppPaneId = AppPaneId::Main;

    fn tile_grid_mut(&mut self) -> &mut TileGrid { &mut self.tiles }

    fn cell_clicked(&mut self, click: CellClick<TileKey>) { render::cell_clicked(self, click); }
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

/// The favorites modal's keys hang off [`AppPaneId::Favorites`].
impl FavoritesHost for App {
    const FAVORITES_PANE: AppPaneId = AppPaneId::Favorites;

    fn favorites_overlay(&self) -> &FavoritesOverlay { &self.favorites_overlay }

    fn favorites_overlay_mut(&mut self) -> &mut FavoritesOverlay { &mut self.favorites_overlay }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::agent_cell::TreeColumn;
    use crate::census::AgentRow;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyRole;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::ReviewerState;
    use crate::census::scan::ScanStage;
    use crate::config::Config;
    use crate::sort::SummaryColumn;

    /// A saved preference belongs to one root incarnation, rather than its slot or pid alone.
    #[test]
    fn tree_sorts_follow_root_identity_across_census_changes() {
        let row = |pid, started| {
            serde_json::from_value::<AgentRow>(serde_json::json!({
                "agent":"claude", "name":format!("agent-{pid}"), "pid":pid,
                "started":started, "process_started":started, "directory":"/fixture",
                "desktop":null, "branch":null, "status":"idle", "launched_by":null, "children":[]
            }))
            .expect("a fixture agent")
        };
        let completed = |roots: Vec<AgentRow>| {
            let mut rows = Vec::new();
            let mut edges = Vec::new();
            for mut root in roots {
                let manager = format!("root-{}", root.pid);
                let member = format!("member-{}", root.pid);
                root.session_id = SessionIdentity::Claude(manager.clone());
                let mut child = row(root.pid + 1000, root.started + 1);
                child.session_id = SessionIdentity::Claude(member.clone());
                edges.push(HierarchyEdge {
                    reviewer: ReviewerState::NotPool,
                    manager,
                    member,
                    role: HierarchyRole::Impl,
                });
                rows.extend([root, child]);
            }
            CensusUpdate::Local {
                rows,
                hierarchy: HierarchyEdges(edges).into(),
                stage: ScanStage::Completed,
            }
        };
        let mut app = App::new_for_test().expect("a test app");
        let first = row(10, 100);
        let second = row(20, 200);
        app.census
            .apply(completed(vec![first.clone(), second.clone()]));
        let first_id = TileKey::Agent(AgentCell {
            machine:         "natedev".to_string(),
            pid:             10,
            process_started: 100,
        });
        let second_id = TileKey::Agent(AgentCell {
            machine:         "natedev".to_string(),
            pid:             20,
            process_started: 200,
        });
        let mut first_sort = TreeSort::default();
        first_sort.select(TreeColumn::Role);
        let mut second_sort = TreeSort::default();
        second_sort.select(TreeColumn::Session);
        app.tree_sorts.insert(first_id.clone(), first_sort);
        app.tree_sorts.insert(second_id.clone(), second_sort);
        app.summary_sort.select(SummaryColumn::Status);
        let summary_sort = app.summary_sort;

        app.census.apply(completed(vec![second, first.clone()]));
        app.prune_tree_sorts();
        assert_eq!(app.tree_sorts.get(&first_id), Some(&first_sort));
        assert_eq!(app.tree_sorts.get(&second_id), Some(&second_sort));

        app.census.apply(completed(vec![first, row(20, 300)]));
        app.prune_tree_sorts();
        assert_eq!(
            app.tree_sorts.len(),
            1,
            "a recycled pid starts with the default sort"
        );
        assert_eq!(app.tree_sorts.get(&first_id), Some(&first_sort));
        assert_eq!(app.summary_sort, summary_sort);

        app.census.apply(CensusUpdate::Local {
            rows:      Vec::new(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        app.prune_tree_sorts();
        assert!(app.tree_sorts.is_empty());
        assert_eq!(
            app.summary_sort, summary_sort,
            "machine scans keep the summary preference"
        );
    }

    /// Startup gives the scheduler the configured command before the
    /// settings overlay has been opened or edited.
    #[test]
    fn the_hierarchy_handle_starts_with_the_loaded_command() {
        let mut config = Config::default();
        config.census.hierarchy_command = "production_tree --json".to_string();
        let app = App::new_with_keymap_path(
            LoadedConfig {
                config,
                error: None,
            },
            None,
            None,
            Census::new("natedev".to_string()),
        )
        .expect("test app should build");

        assert_eq!(
            app.hierarchy_command.snapshot(),
            HierarchySource::Command("production_tree --json".to_string())
        );
    }
}
