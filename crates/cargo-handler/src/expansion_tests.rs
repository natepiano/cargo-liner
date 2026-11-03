#![expect(clippy::panic, reason = "invalid fixtures must fail tests")]
use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::Value;
use tui_pane::TerminalApp;

use super::*;
use crate::agent_cell::TreeColumn;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::scan::ScanStage;
use crate::census::transcript::SubagentRole;
use crate::interaction::LegacyWorkKind;
use crate::interaction::RowLocation;
use crate::interaction::RowOwner;
use crate::render;
use crate::sort::SortDirection;
use crate::sort::TreeSort;

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
    expansion.0.expand(key.clone(), depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 30, 40]);
    expansion.0.toggle(key.clone(), depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
    expansion.0.toggle(key.clone(), depth);
    expansion.0.collapse(key, depth);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn bulk_operations_and_reset_include_hidden_branches() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.0.collapse_all(&tree);
    assert_eq!(visible(&tree, &expansion), [10, 40]);
    expansion.0.expand_all(&tree);
    assert_eq!(visible(&tree, &expansion), [10, 20, 30, 40]);
    expansion.0.reset();
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn collapse_closed_child_selects_and_closes_parent() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    let result = expansion.0.collapse_row(&tree, 1);
    assert_eq!(result.selected, 0);
    assert_eq!(visible(&tree, &expansion), [10, 40]);
    expansion.0.expand_all(&tree);
    let result = expansion.0.collapse_row(&tree, 1);
    assert_eq!(result.selected, 1);
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
    let result = expansion.0.collapse_row(&tree, 2);
    assert_eq!(result.selected, 1);
}

#[test]
fn removed_process_returns_to_default() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.0.collapse_all(&tree);
    expansion
        .0
        .retain(|key| !matches!(key, ExpansionKey::Process(process) if process.pid == 10));
    assert_eq!(visible(&tree, &expansion), [10, 20, 40]);
}

#[test]
fn moved_process_keeps_override_at_new_depth() {
    let tree = forest();
    let mut expansion = HandlerExpansion::default();
    expansion.0.expand_all(&tree);
    let mut rows = tree.rows;
    rows[1].depth = 0;
    rows[1].parent = RowParent::Root;
    rows[2].depth = 1;
    let moved = HandlerTree::new(rows);
    assert_eq!(visible(&moved, &expansion), [10, 20, 30, 40]);
    let RowExpansion::Branch { key, depth } = moved.row_expansion_at(1) else {
        panic!("moved branch must expand");
    };
    expansion.0.collapse(key, depth);
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

fn paint(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(180, 48)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, 200))
        .expect("frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, 200))
        .expect("settled frame");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
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
    assert!(!text.contains("child"));
    dispatch(&mut app, HandlerExpansionAction::Reset);
    let text = paint(&mut app);
    assert!(text.contains("child"));
    assert!(!text.contains("grandchild"));
}

#[test]
fn census_pruning_removes_choices_for_departed_processes() {
    let mut app = application();
    paint(&mut app);
    dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::CollapseAll),
    );
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
    expansion.0.collapse(key, depth);
    let mut rows = tree.rows;
    rows[1].depth = 0;
    rows[1].parent = RowParent::Root;
    rows[2].depth = 1;
    let moved = HandlerTree::new(rows);
    assert_eq!(visible(&moved, &expansion), [10, 20, 40]);
    expansion.0.reset();
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
    expansion.0.collapse(local.clone(), 0);
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
            machine: census::local_machine_name(),
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
            machine: census::local_machine_name(),
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
        machine: census::local_machine_name(),
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
