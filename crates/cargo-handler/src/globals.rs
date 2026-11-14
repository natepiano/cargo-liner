//! The app-globals scope: this app's global shortcuts, the ones the
//! framework does not already own.
//!
//! [`tui_pane::GlobalAction`] owns quit, restart, pane cycling, and the
//! settings / keymap / shortcut overlays — those need no registration
//! here. This scope is for the shortcuts *this* app adds on top: the two
//! that open and close cells, the horizontal arrows that move the focus
//! ring between them, and Enter, which shows the focused tile on the full
//! screen. The framework picks up the rest from the
//! registration in [`crate::keymap`]: TOML loading, the status-line
//! slots, and the rows in the keymap overlay.
//!
//! The rest belong to the attract screen: `a` draws it over the grid,
//! `r` draws a fresh one at random, and `u` restores the configuration
//! that replacement displaced. Three more belong to its saved
//! favorites: `ctrl-s` saves the parameters on screen now, `ctrl-o`
//! opens the saved list, and `m` shows one of them at random.
//!
//! To add another, give the enum a variant, bind a default key in
//! [`Globals::defaults`], and handle it in [`dispatch`].

use std::time::Instant;

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::ExpansionAction;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::NavAction;
use tui_pane::TileAction;
use tui_pane::TileGrowth;

use crate::app::App;
use crate::app::expansion::DepthDigit;
use crate::app::expansion::HandlerExpansionAction;
use crate::app::expansion::{self};
use crate::constants::APP_GLOBALS_SECTION;
use crate::details;
use crate::render;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum AppGlobalAction {
        OpenDetails => ("open_details", "Show the selected row details");
        ClearWord => ("clear_word", "Clear the word highlight");
        ExpandAll => ("expand_all", "Expand all rows");
        CollapseAll => ("collapse_all", "Collapse all to first level");
        ExpandRow => ("expand_row", "Expand the selected row");
        CollapseRow => ("collapse_row", "Collapse the selected row");
        ToggleRow => ("toggle_row", "Toggle the selected row");
        Expand => ("expand", "Expand the focused tile");
        Depth0 => ("depth_0", "Show focused tile depth 0");
        Depth1 => ("depth_1", "Show focused tile depth 1");
        Depth2 => ("depth_2", "Show focused tile depth 2");
        Depth3 => ("depth_3", "Show focused tile depth 3");
        Depth4 => ("depth_4", "Show focused tile depth 4");
        Depth5 => ("depth_5", "Show focused tile depth 5");
        Depth6 => ("depth_6", "Show focused tile depth 6");
        Depth7 => ("depth_7", "Show focused tile depth 7");
        Depth8 => ("depth_8", "Show focused tile depth 8");
        Depth9 => ("depth_9", "Show focused tile depth 9");
        AddTile    => ("add_tile",    "Add a tile");
        RemoveTile => ("remove_tile", "Remove an empty tile");
        FocusLeft  => ("focus_left",  "Focus the tile to the left");
        FocusRight => ("focus_right", "Focus the tile to the right");
        FocusUp    => ("focus_up",    "Focus the tile above");
        FocusDown  => ("focus_down",  "Focus the tile below");
        RowUp => ("row_up", "Select the previous row");
        RowDown => ("row_down", "Select the next row");
        RowPageUp => ("row_page_up", "Select one page above");
        RowPageDown => ("row_page_down", "Select one page below");
        RowHome => ("row_home", "Select the first row");
        RowEnd => ("row_end", "Select the last row");
        Attract    => ("attract",     "Show the attract screen");
        RandomizeAttract => ("randomize_attract", "Randomize the attract screen");
        UndoAttractReplacement => ("undo_attract_replacement", "Undo attract replacement");
        SaveFavorite => ("save_favorite", "Save attract parameters");
        OpenFavorites => ("open_favorites", "Open attract favorites");
        RandomFavorite => ("random_favorite", "Show a random favorite");
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    const SECTION_NAME: &'static str = APP_GLOBALS_SECTION;

    fn render_order() -> &'static [Self::Actions] { <Self as tui_pane::Action>::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        let mut bindings = tui_pane::bindings! {
            KeyCode::Esc => Self::ClearWord,
            'd' => Self::OpenDetails,
            '+' => Self::AddTile,
            KeyBind::shift(KeyCode::Delete) => Self::RemoveTile,
            KeyBind::shift(KeyCode::Left) => Self::FocusLeft,
            KeyBind::shift(KeyCode::Right) => Self::FocusRight,
            KeyBind::shift(KeyCode::Up) => Self::FocusUp,
            KeyBind::shift(KeyCode::Down) => Self::FocusDown,
            KeyCode::Up => Self::RowUp,
            KeyCode::Down => Self::RowDown,
            KeyCode::PageUp => Self::RowPageUp,
            KeyCode::PageDown => Self::RowPageDown,
            KeyCode::Home => Self::RowHome,
            KeyCode::End => Self::RowEnd,
            'a' => Self::Attract,
            'r' => Self::RandomizeAttract,
            'u' => Self::UndoAttractReplacement,
            KeyBind::ctrl('s') => Self::SaveFavorite,
            KeyBind::ctrl('o') => Self::OpenFavorites,
            'm' => Self::RandomFavorite,
            KeyCode::Enter => Self::Expand,
            '0' => Self::Depth0,
            '1' => Self::Depth1,
            '2' => Self::Depth2,
            '3' => Self::Depth3,
            '4' => Self::Depth4,
            '5' => Self::Depth5,
            '6' => Self::Depth6,
            '7' => Self::Depth7,
            '8' => Self::Depth8,
            '9' => Self::Depth9,
        };
        for (key, action) in ExpansionAction::key_bindings() {
            let action = match action {
                ExpansionAction::ExpandAll => Self::ExpandAll,
                ExpansionAction::CollapseAll => Self::CollapseAll,
                ExpansionAction::ExpandRow => Self::ExpandRow,
                ExpansionAction::CollapseRow => Self::CollapseRow,
            };
            bindings.bind(key, action);
        }
        bindings
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Move the focus ring. A full-screen tile keeps the focus, so row keys stay on the shown tile.
fn move_focus(app: &mut App, action: TileAction, growth: TileGrowth) {
    if !app.full_screen.is_open() {
        app.tiles.apply(action, growth);
    }
}

/// Run one app-global action.
fn dispatch(action: AppGlobalAction, app: &mut App) {
    let growth = app.loaded_config.config.tiles.growth();
    match action {
        AppGlobalAction::OpenDetails => details::open_selected(app),
        AppGlobalAction::ClearWord => {
            app.word_highlight.clear();
            app.painted_words = crate::word_highlight::PaintedWords::default();
        },
        AppGlobalAction::ExpandAll => expansion::dispatch(
            app,
            HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
        ),
        AppGlobalAction::CollapseAll => expansion::dispatch(
            app,
            HandlerExpansionAction::Shared(ExpansionAction::CollapseAll),
        ),
        AppGlobalAction::ExpandRow => expansion::dispatch(
            app,
            HandlerExpansionAction::Shared(ExpansionAction::ExpandRow),
        ),
        AppGlobalAction::CollapseRow => expansion::dispatch(
            app,
            HandlerExpansionAction::Shared(ExpansionAction::CollapseRow),
        ),
        AppGlobalAction::ToggleRow => expansion::dispatch(app, HandlerExpansionAction::Toggle),
        AppGlobalAction::Expand => app.full_screen.expand(Instant::now()),
        AppGlobalAction::Depth0 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Zero));
        },
        AppGlobalAction::Depth1 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::One));
        },
        AppGlobalAction::Depth2 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Two));
        },
        AppGlobalAction::Depth3 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Three));
        },
        AppGlobalAction::Depth4 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Four));
        },
        AppGlobalAction::Depth5 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Five));
        },
        AppGlobalAction::Depth6 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Six));
        },
        AppGlobalAction::Depth7 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Seven));
        },
        AppGlobalAction::Depth8 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Eight));
        },
        AppGlobalAction::Depth9 => {
            expansion::dispatch(app, HandlerExpansionAction::Level(DepthDigit::Nine));
        },
        AppGlobalAction::AddTile => app.tiles.apply(TileAction::Add, growth),
        AppGlobalAction::RemoveTile => app.tiles.apply(TileAction::Remove, growth),
        AppGlobalAction::FocusLeft => move_focus(app, TileAction::FocusLeft, growth),
        AppGlobalAction::FocusRight => move_focus(app, TileAction::FocusRight, growth),
        AppGlobalAction::FocusUp => move_focus(app, TileAction::FocusUp, growth),
        AppGlobalAction::FocusDown => move_focus(app, TileAction::FocusDown, growth),
        AppGlobalAction::RowUp => render::navigate_selection(app, NavAction::Up),
        AppGlobalAction::RowDown => render::navigate_selection(app, NavAction::Down),
        AppGlobalAction::RowPageUp => render::navigate_selection(app, NavAction::PageUp),
        AppGlobalAction::RowPageDown => render::navigate_selection(app, NavAction::PageDown),
        AppGlobalAction::RowHome => render::navigate_selection(app, NavAction::Home),
        AppGlobalAction::RowEnd => render::navigate_selection(app, NavAction::End),
        AppGlobalAction::Attract => app.attract.toggle(),
        AppGlobalAction::RandomizeAttract => app.attract.randomize(),
        AppGlobalAction::UndoAttractReplacement => tui_pane::undo_attract_replacement(app),
        AppGlobalAction::SaveFavorite => tui_pane::save_favorite(app),
        AppGlobalAction::OpenFavorites => tui_pane::open_favorites(app),
        AppGlobalAction::RandomFavorite => tui_pane::show_random_favorite(app),
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Instant;

    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use tui_pane::FocusLocation;
    use tui_pane::KeyBind;
    use tui_pane::TerminalApp;

    use super::*;
    use crate::census::AgentRow;
    use crate::census::CensusUpdate;
    use crate::census::HierarchyEdges;
    use crate::census::scan::ScanStage;
    use crate::tiles::TileContent;
    use crate::tiles::TileKey;

    fn paint_focus(app: &mut App) {
        let mut terminal = Terminal::new(TestBackend::new(180, 48)).expect("test terminal");
        let keymap = app.keymap();
        terminal
            .draw(|frame| app.draw(frame, &keymap))
            .expect("focus frame");
    }

    fn focus_application(initial_rows: usize) -> App {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.initial_rows =
            serde_json::from_value(serde_json::json!(initial_rows)).expect("tile rows");
        app.loaded_config.config.tiles.widen_summary = false;
        let rows = [10, 20, 30]
            .into_iter()
            .map(|pid| {
                serde_json::from_value::<AgentRow>(serde_json::json!({
                    "agent":"claude", "pid":pid, "name":format!("owner-{pid}"),
                    "started":100, "session_id":format!("session-{pid}"),
                    "directory":"/fixture", "status":"idle",
                    "children":[{
                        "kind":"shell", "depth":0, "pid":9000+pid,
                        "name":format!("build-for-{pid}"), "started":100
                    }]
                }))
                .expect("session fixture")
            })
            .collect();
        app.census.apply(CensusUpdate::Local {
            rows,
            hierarchy: HierarchyEdges::default().into(),
            stage: ScanStage::Completed,
        });
        paint_focus(&mut app);
        app.tiles.settle_for_test();
        paint_focus(&mut app);
        app
    }

    fn owner_position(app: &App, pid: u32) -> Position {
        let area = app.tiles
            .placements(app.selection_area, app.loaded_config.config.tiles.growth())
            .into_iter()
            .find(|placement| {
                matches!(&placement.content, TileContent::Group(TileKey::Agent(cell)) if cell.pid == pid)
            })
            .expect("owner tile")
            .frame
            .rect();
        Position::new(area.x + area.width / 2, area.y + area.height / 2)
    }

    fn click_owner(app: &mut App, pid: u32) {
        app.click(owner_position(app, pid));
        paint_focus(app);
    }

    fn assert_shift_arrow_edge(app: &mut App, key: KeyCode, target: Position) {
        let expected = FocusLocation::Cell(app.tiles.cell_at(target).expect("drawn target tile"));
        assert_ne!(
            app.tiles.focused_cell(),
            expected,
            "focus starts on another tile"
        );
        tui_pane::dispatch_key(app, KeyEvent::new(key, KeyModifiers::SHIFT));
        assert_eq!(
            app.tiles.focused_cell(),
            expected,
            "shift arrow moves one tile"
        );
        tui_pane::dispatch_key(app, KeyEvent::new(key, KeyModifiers::SHIFT));
        assert_eq!(
            app.tiles.focused_cell(),
            expected,
            "focus stays at the edge"
        );
    }

    fn horizontal_focus_application() -> App {
        let mut app = focus_application(1);
        for pid in [20, 30, 10] {
            click_owner(&mut app, pid);
        }
        app
    }

    #[test]
    fn shift_arrow_right_moves_one_tile_and_stays_at_the_edge() {
        let mut app = horizontal_focus_application();
        let target = owner_position(&app, 30);
        assert!(owner_position(&app, 10).x < target.x);
        assert_shift_arrow_edge(&mut app, KeyCode::Right, target);
    }

    #[test]
    fn shift_arrow_left_moves_one_tile_and_stays_at_the_edge() {
        let mut app = horizontal_focus_application();
        click_owner(&mut app, 30);
        let target = owner_position(&app, 10);
        assert!(target.x < owner_position(&app, 30).x);
        assert_shift_arrow_edge(&mut app, KeyCode::Left, target);
    }

    #[test]
    fn shift_arrow_up_moves_one_tile_and_stays_at_the_edge() {
        let mut app = focus_application(8);
        click_owner(&mut app, 10);
        let area = app
            .tiles
            .placements(app.selection_area, app.loaded_config.config.tiles.growth())
            .into_iter()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("summary tile")
            .frame
            .rect();
        let target = Position::new(area.x + area.width / 2, area.y + area.height / 2);
        assert!(target.y < owner_position(&app, 10).y);
        assert_shift_arrow_edge(&mut app, KeyCode::Up, target);
    }

    #[test]
    fn shift_arrow_down_moves_one_tile_and_stays_at_the_edge() {
        let mut app = focus_application(8);
        click_owner(&mut app, 20);
        let target = owner_position(&app, 30);
        assert!(owner_position(&app, 20).y < target.y);
        assert_shift_arrow_edge(&mut app, KeyCode::Down, target);
    }

    /// Every default key reaches its action, read out of the table the
    /// keymap is built from. The arrows and `+` `-` share the scope with
    /// the attract and favorites keys, so a key added over one of them
    /// would take a tile away instead.
    #[test]
    fn each_default_key_maps_to_its_action() {
        let scope = AppGlobalAction::defaults().into_scope_map();
        let expected = [
            (KeyBind::from('d'), AppGlobalAction::OpenDetails),
            (KeyBind::from('+'), AppGlobalAction::AddTile),
            (KeyBind::from(KeyCode::Esc), AppGlobalAction::ClearWord),
            (KeyBind::from('='), AppGlobalAction::ExpandAll),
            (KeyBind::from('-'), AppGlobalAction::CollapseAll),
            (KeyBind::from(KeyCode::Left), AppGlobalAction::CollapseRow),
            (KeyBind::from(KeyCode::Right), AppGlobalAction::ExpandRow),
            (KeyBind::from(KeyCode::Enter), AppGlobalAction::Expand),
            (KeyBind::from('0'), AppGlobalAction::Depth0),
            (KeyBind::from('1'), AppGlobalAction::Depth1),
            (KeyBind::from('2'), AppGlobalAction::Depth2),
            (KeyBind::from('3'), AppGlobalAction::Depth3),
            (KeyBind::from('4'), AppGlobalAction::Depth4),
            (KeyBind::from('5'), AppGlobalAction::Depth5),
            (KeyBind::from('6'), AppGlobalAction::Depth6),
            (KeyBind::from('7'), AppGlobalAction::Depth7),
            (KeyBind::from('8'), AppGlobalAction::Depth8),
            (KeyBind::from('9'), AppGlobalAction::Depth9),
            (KeyBind::shift(KeyCode::Delete), AppGlobalAction::RemoveTile),
            (KeyBind::shift(KeyCode::Left), AppGlobalAction::FocusLeft),
            (KeyBind::shift(KeyCode::Right), AppGlobalAction::FocusRight),
            (KeyBind::shift(KeyCode::Up), AppGlobalAction::FocusUp),
            (KeyBind::shift(KeyCode::Down), AppGlobalAction::FocusDown),
            (KeyBind::from(KeyCode::Up), AppGlobalAction::RowUp),
            (KeyBind::from(KeyCode::Down), AppGlobalAction::RowDown),
            (KeyBind::from(KeyCode::PageUp), AppGlobalAction::RowPageUp),
            (
                KeyBind::from(KeyCode::PageDown),
                AppGlobalAction::RowPageDown,
            ),
            (KeyBind::from(KeyCode::Home), AppGlobalAction::RowHome),
            (KeyBind::from(KeyCode::End), AppGlobalAction::RowEnd),
            (KeyBind::from('a'), AppGlobalAction::Attract),
            (KeyBind::from('r'), AppGlobalAction::RandomizeAttract),
            (KeyBind::from('u'), AppGlobalAction::UndoAttractReplacement),
            (KeyBind::ctrl('s'), AppGlobalAction::SaveFavorite),
            (KeyBind::ctrl('o'), AppGlobalAction::OpenFavorites),
            (KeyBind::from('m'), AppGlobalAction::RandomFavorite),
        ];

        for (bind, action) in expected {
            assert_eq!(scope.action_for(&bind), Some(action), "{bind:?}");
        }
        // `toggle_row` is the one action with no default key.
        assert_eq!(scope.key_for(AppGlobalAction::ToggleRow), None);
        assert_eq!(
            expected.len() + 1,
            <AppGlobalAction as tui_pane::Action>::ALL.len()
        );
    }

    /// `r` asks for the attract screen as well as drawing a new one, so
    /// the draw is seen.
    #[test]
    fn randomizing_requests_the_attract_screen() {
        let mut app = App::new_for_test().expect("test app should build");
        assert!(!app.attract.asked_for());

        dispatch(AppGlobalAction::RandomizeAttract, &mut app);

        assert!(app.attract.asked_for());
    }

    /// With no replacement to undo, `u` says so rather than doing
    /// nothing silently.
    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let mut app = App::new_for_test().expect("test app should build");

        dispatch(AppGlobalAction::UndoAttractReplacement, &mut app);

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Nothing to undo");
    }
}
