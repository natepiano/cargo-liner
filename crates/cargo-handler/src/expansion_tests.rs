#![expect(clippy::panic, reason = "invalid fixtures must fail tests")]
use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use serde_json::Value;
use tui_pane::TerminalApp;

use super::*;
use crate::agent_cell::TreeColumn;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::MachineState;
use crate::census::scan::ScanStage;
use crate::census::transcript::SubagentRole;
use crate::interaction::LegacyWorkKind;
use crate::interaction::RowLocation;
use crate::interaction::RowOwner;
use crate::render;
use crate::sort::SortDirection;
use crate::sort::TreeSort;
use crate::tiles::TileKey;

const FIXTURE_MACHINE: &str = "natedev";

fn forest() -> HandlerTree {
    HandlerTree::new(
        [
            (10, 0, RowParent::Root),
            (20, 1, RowParent::Parent(0)),
            (30, 2, RowParent::Parent(1)),
            (40, 0, RowParent::Root),
        ]
        .into_iter()
        .map(|(pid, depth, parent)| HandlerTreeRow {
            identity: SelectedRow::Shell {
                machine: "local".to_owned(),
                pid,
                started: 0,
            },
            key: ExpansionKey::process("local", pid),
            depth,
            parent,
        })
        .collect(),
    )
}

fn visible(tree: &HandlerTree, expansion: &HandlerExpansion) -> Vec<u32> {
    tree.rows
        .iter()
        .zip(tree.visibility(expansion))
        .filter(|(_, visibility)| visibility.is_visible())
        .map(|(row, _)| match &row.key {
            ExpansionKey::Process(key) => key.pid,
            ExpansionKey::Work(_) => panic!("fixture row must have a process identity"),
        })
        .collect()
}

#[test]
fn default_shows_only_root_children() {
    assert_eq!(
        visible(&forest(), &HandlerExpansion::default()),
        [10, 20, 40]
    );
}

#[test]
fn expand_toggle_and_collapse_change_visible_rows() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    let RowExpansion::Branch { key, depth } = tree.row_expansion_at(1) else {
        panic!("child must expand");
    };
    expansion.choices.expand(key.clone(), depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 30, 40]);
    expansion.choices.toggle(key.clone(), depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
    expansion.choices.toggle(key.clone(), depth);
    expansion.choices.collapse(key, depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn bulk_operations_and_reset_include_hidden_branches() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.choices.collapse_all(&tree);
    assert_eq!(visible(&tree, &expansion), [10, 40]);
    expansion.choices.expand_all(&tree);
    assert_eq!(visible(&tree, &expansion), [10, 20, 30, 40]);
    expansion.choices.reset();
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn collapse_closed_child_selects_and_closes_parent() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    let result = expansion.choices.collapse_row(&tree, 1);
    assert_eq!(result.selected, 0);
    assert_eq!(visible(&tree, &expansion), [10, 40]);
    expansion.choices.expand_all(&tree);
    let result = expansion.choices.collapse_row(&tree, 1);
    assert_eq!(result.selected, 1);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
    let result = expansion.choices.collapse_row(&tree, 2);
    assert_eq!(result.selected, 1);
}

#[test]
fn removed_process_returns_to_default() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.choices.collapse_all(&tree);
    expansion
        .choices
        .retain(|key| !matches!(key, ExpansionKey::Process(process) if process.pid == 10));
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn moved_process_keeps_override_at_new_depth() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.choices.expand_all(&tree);
    let mut rows = tree.rows;
    rows[1].depth = 0;
    rows[1].parent = RowParent::Root;
    rows[2].depth = 1;
    let moved = HandlerTree::new(rows);
    assert_eq!(visible(&moved, &expansion), [10, 20, 30, 40]);
    let RowExpansion::Branch { key, depth } = moved.row_expansion_at(1) else {
        panic!("moved branch must expand");
    };
    expansion.choices.collapse(key, depth);
    assert_eq!(visible(&moved, &expansion), [10, 20, 40]);
}

#[expect(clippy::expect_used, reason = "fixture sessions must parse")]
fn application() -> App {
    let rows = [
        (10, "root", None),
        (20, "child", Some(10)),
        (30, "grandchild", Some(20)),
        (40, "other", None),
    ]
    .into_iter()
    .map(|(pid, name, parent)| {
        serde_json::from_value(serde_json::json!({
            "agent":"claude", "pid":pid, "name":name, "started":100,
            "session_id":format!("session-{pid}"), "directory":"/fixture",
            "status":"idle", "children":[], "launched_by":parent,
        }))
        .expect("session fixture")
    })
    .collect();
    let mut app = App::new_for_test().expect("test app");
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    app
}

fn paint_buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, 200))
        .expect("frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, 200))
        .expect("settled frame");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

fn paint(app: &mut App) -> String { buffer_text(&paint_buffer(app, 180, 48)) }

fn locate(buffer: &Buffer, word: &str) -> Position {
    for y in buffer.area.y..buffer.area.bottom() {
        let line: String = (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(index) = line.find(word) {
            let Ok(column) = u16::try_from(line[..index].chars().count()) else {
                panic!("the painted word must fit the buffer width");
            };
            return Position::new(column + buffer.area.x, y);
        }
    }
    panic!("missing painted word {word}: {}", buffer_text(buffer));
}

fn marker(buffer: &Buffer, word: &str) -> Position {
    let name = locate(buffer, word);
    let Some(x) = (buffer.area.x..name.x)
        .rev()
        .find(|&x| matches!(buffer[(x, name.y)].symbol(), "▶" | "▼"))
    else {
        panic!("the branch marker must precede the name");
    };
    Position::new(x, name.y)
}

fn press(app: &mut App, key: char) {
    tui_pane::dispatch_key(
        app,
        crossterm::event::KeyEvent::new(KeyCode::Char(key), crossterm::event::KeyModifiers::NONE),
    );
}

#[test]
fn rendered_rows_follow_shared_actions_and_default_depth() {
    let mut app = application();
    let text = paint(&mut app);
    assert!(text.contains("root"));
    assert!(text.contains("child"));
    assert!(!text.contains("grandchild"));
    assert!(text.contains('▶'));
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
    );
    let text = paint(&mut app);
    assert!(text.contains("grandchild"));
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::CollapseAll),
    );
    let text = paint(&mut app);
    assert!(text.contains("child"));
    assert!(!text.contains("grandchild"));
    dispatch(&mut app, HandlerExpansionAction::Level(DepthDigit::One));
    let text = paint(&mut app);
    assert!(text.contains("child"));
    assert!(!text.contains("grandchild"));
}

#[test]
fn census_pruning_removes_choices_for_departed_processes() {
    let mut app = application();
    paint(&mut app);
    app.expansion
        .choices
        .collapse(ExpansionKey::process(FIXTURE_MACHINE, 10), 0);
    assert!(!paint(&mut app).contains("child"));
    app.census.apply(CensusUpdate::Local {
        rows:      vec![],
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    app.prune_tree_sorts();
    let fresh = application();
    app.census = fresh.census;
    assert!(paint(&mut app).contains("child"));
}

#[test]
fn row_actions_keep_selection_on_the_collapsed_parent() {
    let mut app = application();
    paint(&mut app);
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = agent_cell::tile_order_for_expansion(&placement);
    let tree = tiles[0].expansion_tree();
    let child = tree
        .rows
        .iter()
        .find(|row| matches!(&row.key, ExpansionKey::Process(key) if key.pid == 20))
        .map_or_else(|| panic!("child must exist"), |row| row.identity.clone());
    let root = tree
        .rows
        .iter()
        .find(|row| matches!(&row.key, ExpansionKey::Process(key) if key.pid == 10))
        .map_or_else(|| panic!("root must exist"), |row| row.identity.clone());
    drop(machines);
    app.tiles.focus_cell(tui_pane::TABLE_CELL + 1);
    let RowLocation::Found(surface) = app
        .selections
        .follow_focused(&SelectableSurface::EmptyCell, &child)
    else {
        panic!("child must be visible");
    };
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandRow),
    );
    assert!(paint(&mut app).contains("grandchild"));
    dispatch(&mut app, HandlerExpansionAction::Toggle);
    assert!(!paint(&mut app).contains("grandchild"));
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::CollapseRow),
    );
    assert!(!paint(&mut app).contains("child"));
    let SurfaceLookup::Current(state) = app.selections.surface(&surface) else {
        panic!("surface must exist");
    };
    assert_eq!(state.selection(), TileSelection::Row(root));
}

#[test]
fn explicit_closed_choice_follows_a_child_to_root_depth() {
    let mut expansion = HandlerExpansion::default();
    let tree = forest();
    let RowExpansion::Branch { key, depth } = tree.row_expansion_at(1) else {
        panic!("child must expand");
    };
    expansion.choices.collapse(key, depth);
    let mut rows = tree.rows;
    rows[1].depth = 0;
    rows[1].parent = RowParent::Root;
    rows[2].depth = 1;
    let moved = HandlerTree::new(rows);
    assert_eq!(visible(&moved, &expansion), [10, 20, 40]);
    expansion.choices.reset();
    assert_eq!(visible(&moved, &expansion), [10, 20, 30, 40]);
}

#[test]
fn machine_identity_keeps_equal_pids_independent() {
    let mut expansion = HandlerExpansion::default();
    let ExpansionKey::Process(local) = ExpansionKey::process("local", 10) else {
        panic!("process must have an identity");
    };
    let ExpansionKey::Process(remote) = ExpansionKey::process("remote", 10) else {
        panic!("process must have an identity");
    };
    let local = ExpansionKey::Process(local);
    let remote = ExpansionKey::Process(remote);
    expansion.choices.collapse(local.clone(), 0);
    assert!(!expansion.is_open(&local, 0));
    assert!(expansion.is_open(&remote, 0));
}

#[expect(clippy::expect_used, reason = "invalid work fixtures must fail tests")]
fn application_with_work(children: &Value) -> App {
    let mut app = application();
    let root = serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":10, "name":"root", "started":100,
        "session_id":"session-10", "directory":"/fixture",
        "status":"idle", "children":children,
    }))
    .expect("work fixture");
    app.census.apply(CensusUpdate::Local {
        rows:      vec![root],
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    app
}

fn select_work(app: &mut App, identity: &SelectedRow) {
    app.tiles.focus_cell(tui_pane::TABLE_CELL + 1);
    assert!(matches!(
        app.selections
            .follow_focused(&SelectableSurface::EmptyCell, identity),
        RowLocation::Found(_)
    ));
}

#[test]
fn recorded_pidless_subagent_hides_children_and_enter_toggles_them() {
    let mut app = application_with_work(&serde_json::json!([
        {"depth":0,"kind":"subagent","work_id":{"Recorded":"task-a"},
         "name":"pidless worker","started":110},
        {"depth":1,"kind":"shell","pid":201,"name":"nested command","started":120}
    ]));
    let text = paint(&mut app);
    assert!(text.contains("pidless worker"));
    assert!(!text.contains("nested command"));
    select_work(
        &mut app,
        &SelectedRow::Subagent {
            machine: FIXTURE_MACHINE.to_owned(),
            owner:   RowOwner {
                pid:     10,
                started: 100,
            },
            id:      "task-a".into(),
        },
    );
    let enter =
        crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
    tui_pane::dispatch_key(&mut app, enter);
    assert!(paint(&mut app).contains("nested command"));
    tui_pane::dispatch_key(&mut app, enter);
    assert!(!paint(&mut app).contains("nested command"));
}

#[test]
fn duplicate_legacy_branches_keep_ordinals_before_visibility_filtering() {
    let mut app = application_with_work(&serde_json::json!([
        {"depth":0,"kind":"subagent","pid":101,"name":"first worker","started":110},
        {"depth":1,"kind":"subagent","pid":103,"name":"hidden duplicate","started":110},
        {"depth":0,"kind":"subagent","pid":102,"name":"second worker","started":110},
        {"depth":1,"kind":"shell","pid":202,"name":"second command","started":120}
    ]));
    let text = paint(&mut app);
    assert!(!text.contains("hidden duplicate"));
    select_work(
        &mut app,
        &SelectedRow::LegacyWork {
            machine: FIXTURE_MACHINE.to_owned(),
            owner:   RowOwner {
                pid:     10,
                started: 100,
            },
            kind:    LegacyWorkKind::Subagent(SubagentRole::Unrecorded),
            started: 110,
            ordinal: 2,
        },
    );
    dispatch(&mut app, HandlerExpansionAction::Toggle);
    assert!(paint(&mut app).contains("second command"));
    dispatch(&mut app, HandlerExpansionAction::Toggle);
    assert!(!paint(&mut app).contains("second command"));
}

#[test]
fn legacy_branch_expansion_and_selection_follow_census_identity_after_sorting() {
    let mut app = application_with_work(&serde_json::json!([
        {"depth":0,"kind":"subagent","name":"branch A","started":110},
        {"depth":1,"kind":"shell","pid":201,"name":"command A","started":120},
        {"depth":0,"kind":"subagent","name":"branch B","started":110},
        {"depth":1,"kind":"shell","pid":202,"name":"command B","started":120}
    ]));
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let Some(tile) = agent_cell::tile_keys(&placement).into_iter().next() else {
        panic!("the fixture must have a tile");
    };
    app.tree_sorts.insert(
        tile.clone(),
        TreeSort {
            column:    TreeColumn::Session,
            direction: SortDirection::Ascending,
        },
    );
    paint(&mut app);
    let branch_a = SelectedRow::LegacyWork {
        machine: FIXTURE_MACHINE.to_owned(),
        owner:   RowOwner {
            pid:     10,
            started: 100,
        },
        kind:    LegacyWorkKind::Subagent(SubagentRole::Unrecorded),
        started: 110,
        ordinal: 0,
    };
    select_work(&mut app, &branch_a);
    dispatch(&mut app, HandlerExpansionAction::Toggle);
    let text = paint(&mut app);
    assert!(text.contains("command A"));
    assert!(!text.contains("command B"));

    app.tree_sorts
        .entry(tile.clone())
        .or_default()
        .select(TreeColumn::Session);
    let text = paint(&mut app);
    assert!(text.contains("command A"), "branch A must stay open");
    assert!(!text.contains("command B"), "branch B must stay closed");
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = agent_cell::tile_order_sorted(&placement, &app.tree_sorts, 200);
    let named_rows = tiles[0].named_row_identities();
    let Some((_, branch_a_after_sort)) = named_rows.iter().find(|(name, _)| name == "branch A")
    else {
        panic!("the sorted tile must contain branch A");
    };
    assert_eq!(branch_a_after_sort, &branch_a);
    let SurfaceLookup::Current(state) = app.selections.surface(&SelectionSurface::Tile(tile))
    else {
        panic!("the sorted tile must retain its selection");
    };
    assert_eq!(
        state.selection(),
        TileSelection::Row(branch_a_after_sort.clone())
    );
}

#[test]
fn marker_clicks_open_close_and_select_the_painted_branch() {
    let mut app = application();
    let before = paint_buffer(&mut app, 180, 48);
    let point = marker(&before, "child");
    assert_eq!(before[(point.x, point.y)].symbol(), "▶");
    app.click(Position::new(point.x + 1, point.y));
    let open = paint_buffer(&mut app, 180, 48);
    assert!(buffer_text(&open).contains("grandchild"));
    let SelectableSurface::Surface(surface) = focused_surface(&app) else {
        panic!("the click must focus the tile");
    };
    assert!(matches!(
        app.selections
            .surface(&surface)
            .expect("clicked surface")
            .selection(),
        TileSelection::Row(SelectedRow::Session { pid: 20, .. })
    ));
    let point = marker(&open, "child");
    assert_eq!(open[(point.x, point.y)].symbol(), "▼");
    app.click(point);
    assert!(!paint(&mut app).contains("grandchild"));
}

#[test]
fn name_and_leaf_lead_clicks_only_select_the_row() {
    let mut app = application();
    let before = paint_buffer(&mut app, 180, 48);
    app.click(locate(&before, "child"));
    assert!(!paint(&mut app).contains("grandchild"));
    let SelectableSurface::Surface(surface) = focused_surface(&app) else {
        panic!("the name must focus its tile");
    };
    assert!(matches!(
        app.selections
            .surface(&surface)
            .expect("selected tile")
            .selection(),
        TileSelection::Row(SelectedRow::Session { pid: 20, .. })
    ));
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
    );
    let open = paint_buffer(&mut app, 180, 48);
    let leaf = locate(&open, "grandchild");
    app.click(Position::new(leaf.x - 1, leaf.y));
    assert!(paint(&mut app).contains("grandchild"));
    assert!(matches!(
        app.selections
            .surface(&surface)
            .expect("leaf tile")
            .selection(),
        TileSelection::Row(SelectedRow::Session { pid: 30, .. })
    ));
}

#[test]
fn a_scrolled_marker_toggles_the_row_in_the_painted_viewport() {
    let children: Vec<_> = (0..30)
        .flat_map(|index| {
            [
                serde_json::json!({"depth":0,"kind":"subagent","pid":100+index,
            "name":format!("branch-{index:02}"),"started":110}),
                serde_json::json!({"depth":1,"kind":"shell","pid":200+index,
            "name":format!("nested-{index:02}"),"started":120}),
            ]
        })
        .collect();
    let mut app = application_with_work(&Value::Array(children));
    let before = paint_buffer(&mut app, 100, 14);
    app.click(locate(&before, "branch-00"));
    tui_pane::dispatch_key(
        &mut app,
        crossterm::event::KeyEvent::new(KeyCode::End, crossterm::event::KeyModifiers::NONE),
    );
    let scrolled = paint_buffer(&mut app, 100, 14);
    assert!(!buffer_text(&scrolled).contains("branch-00"));
    app.click(marker(&scrolled, "branch-29"));
    let after = paint_buffer(&mut app, 100, 48);
    assert!(buffer_text(&after).contains("nested-29"));
    assert!(!buffer_text(&after).contains("nested-28"));
}

#[test]
fn a_narrow_tile_keeps_its_marker_click_target() {
    let mut app = application_with_work(&serde_json::json!([
        {"depth":0,"kind":"subagent","pid":101,"name":"a-long-branch-name","started":110},
        {"depth":1,"kind":"shell","pid":201,"name":"opened-work","started":120}
    ]));
    let before = paint_buffer(&mut app, 32, 24);
    let point = marker(&before, "a-long");
    assert_eq!(before[(point.x, point.y)].symbol(), "▶");
    app.click(point);
    assert!(paint(&mut app).contains("opened-work"));
}

#[test]
fn a_marker_for_a_departed_row_does_not_toggle_its_replacement() {
    let mut app = application();
    let before = paint_buffer(&mut app, 180, 48);
    let point = marker(&before, "child");
    let replacement = application_with_work(&serde_json::json!([
        {"depth":0,"kind":"subagent","pid":101,"name":"replacement","started":110},
        {"depth":1,"kind":"shell","pid":201,"name":"replacement-work","started":120}
    ]));
    app.census = replacement.census;
    app.click(point);
    let after = paint(&mut app);
    assert!(after.contains("replacement"));
    assert!(!after.contains("replacement-work"));
    app.census = application().census;
    assert!(!paint(&mut app).contains("grandchild"));
}

fn depth_rows(prefix: &str, count: usize) -> Vec<AgentRow> {
    let children: Vec<_> = (0..count)
        .map(|depth| {
            serde_json::json!({
                "depth":depth,"kind":"shell","pid":100+depth,
                "name":format!("{prefix}-level-{:02}",depth+1),"started":120,
            })
        })
        .collect();
    let Ok(row) = serde_json::from_value(serde_json::json!({
        "agent":"claude","pid":10,"name":format!("{prefix}-root"),"started":100,
        "session_id":format!("{prefix}-session"),"directory":"/fixture",
        "status":"idle","children":children,
    })) else {
        panic!("the depth fixture must decode as an agent row");
    };
    vec![row]
}

fn two_depth_tiles() -> App {
    let mut app = application();
    app.loaded_config.config.machines.remote = vec!["depth-remote".into()];
    app.census.apply(CensusUpdate::Local {
        rows:      depth_rows("near", 10),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    app.census.apply(CensusUpdate::Remote {
        host:  "depth-remote".into(),
        state: MachineState::Answered(depth_rows("far", 10), HierarchyEdges::default().into()),
    });
    app
}

#[test]
fn digit_depths_change_only_the_focused_tile() {
    let mut app = two_depth_tiles();
    let before = paint_buffer(&mut app, 180, 60);
    app.click(locate(&before, "near-level-01"));
    for digit in ['2', '9', '1', '0'] {
        press(&mut app, digit);
        let after = buffer_text(&paint_buffer(&mut app, 180, 60));
        let Some(depth) = digit.to_digit(10) else {
            panic!("the fixture key must be a depth digit");
        };
        let depth = depth as usize;
        for level in 1..=10 {
            assert_eq!(
                after.contains(&format!("near-level-{level:02}")),
                level <= depth,
                "digit {digit}, level {level}"
            );
            assert_eq!(
                after.contains(&format!("far-level-{level:02}")),
                level == 1,
                "the other tile must retain its default after {digit}"
            );
        }
        assert!(after.contains("near-root"));
    }
}

#[test]
fn minus_restores_the_first_level_in_every_tile() {
    let mut app = two_depth_tiles();
    paint(&mut app);
    press(&mut app, '=');
    let all = paint(&mut app);
    assert!(all.contains("near-level-10"));
    assert!(all.contains("far-level-10"));
    press(&mut app, '-');
    let first = paint(&mut app);
    for prefix in ["near", "far"] {
        assert!(first.contains(&format!("{prefix}-root")));
        assert!(first.contains(&format!("{prefix}-level-01")));
        assert!(!first.contains(&format!("{prefix}-level-02")));
    }
}

#[test]
fn new_rows_follow_the_tile_level_including_expand_all() {
    let mut app = two_depth_tiles();
    paint(&mut app);
    press(&mut app, '=');
    app.census.apply(CensusUpdate::Local {
        rows:      depth_rows("near", 12),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    let all = paint_buffer(&mut app, 180, 70);
    assert!(buffer_text(&all).contains("near-level-12"));
    app.click(locate(&all, "near-level-01"));
    press(&mut app, '2');
    app.census.apply(CensusUpdate::Local {
        rows:      depth_rows("near", 13),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    let bounded = buffer_text(&paint_buffer(&mut app, 180, 70));
    assert!(bounded.contains("near-level-02"));
    assert!(!bounded.contains("near-level-03"));
    assert!(bounded.contains("far-level-10"));
}

#[test]
fn row_choices_override_the_level_until_the_next_digit() {
    let mut app = two_depth_tiles();
    let before = paint_buffer(&mut app, 180, 60);
    app.click(locate(&before, "near-level-01"));
    press(&mut app, '2');
    let two = paint_buffer(&mut app, 180, 60);
    app.click(marker(&two, "near-level-02"));
    assert!(paint(&mut app).contains("near-level-03"));
    press(&mut app, '2');
    assert!(!paint(&mut app).contains("near-level-03"));
    let two = paint_buffer(&mut app, 180, 60);
    app.click(marker(&two, "near-level-01"));
    assert!(!paint(&mut app).contains("near-level-02"));
    press(&mut app, '2');
    assert!(paint(&mut app).contains("near-level-02"));
}

#[test]
fn depth_digits_do_not_change_a_tile_covered_by_the_attract_screen() {
    let mut app = two_depth_tiles();
    let before = paint_buffer(&mut app, 180, 60);
    app.click(locate(&before, "near-level-01"));
    app.attract.request_show();
    press(&mut app, '9');
    app.attract.toggle();
    app.tiles.settle_for_test();
    let key = TileKey::Unassigned {
        machine: FIXTURE_MACHINE.into(),
    };
    assert_eq!(
        app.expansion.level(&key),
        DepthLevel::Through(DepthDigit::One)
    );
    assert!(
        !app.expansion
            .choices
            .is_open(&ExpansionKey::process(FIXTURE_MACHINE, 101), 1)
    );
}
