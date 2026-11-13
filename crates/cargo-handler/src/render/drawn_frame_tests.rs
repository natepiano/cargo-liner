//! Pointer actions use the headers and rows from the painted frame.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::CellIndex;
use tui_pane::CellPoint;
use tui_pane::TerminalApp;
use tui_pane::TileAction;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsTable;
use crate::census::model::ModelName;
use crate::census::model::ReasoningEffort;
use crate::census::scan::ScanStage;
use crate::census::transcript::SubagentRole;
use crate::interaction::RowIdentity;
use crate::interaction::SurfaceSelection;
use crate::sort::SummaryColumn;
use crate::word_highlight::WordHighlight;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn rows() -> TestResult<Vec<AgentRow>> {
    let mut result: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","pid":10,"name":"owner","started":1,"session_id":"owner","directory":"/fixture","children":[
            {"depth":0,"kind":"shell","pid":11,"name":"first-work","started":1},
            {"depth":1,"kind":"shell","pid":12,"name":"nested-work","started":1}
        ]}
    ]))?;
    result[0].children[0].kind = ChildKind::Subagent(SubagentRole::Reviewer);
    Ok(result)
}

fn install(app: &mut App, rows: Vec<AgentRow>) {
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
}

fn paint(app: &mut App, width: u16) -> TestResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(width, 60))?;
    let keymap = app.keymap();
    terminal.draw(|frame| app.draw(frame, &keymap))?;
    app.tiles.settle_for_test();
    terminal.draw(|frame| app.draw(frame, &keymap))?;
    Ok(terminal.backend().buffer().clone())
}

fn group_header(app: &App, buffer: &Buffer, text: &str) -> TestResult<Position> {
    word_click_tests::positions(buffer, text)
        .into_iter()
        .find(|point| {
            app.tiles
                .placements(app.selection_area, app.loaded_config.config.tiles.growth())
                .iter()
                .any(|placement| {
                    matches!(placement.content, TileContent::Group(_))
                        && placement.frame.inner().contains(*point)
                })
        })
        .ok_or_else(|| format!("missing group header {text}").into())
}

fn model_rows() -> TestResult<Vec<AgentRow>> {
    let mut owners = rows()?;
    owners[0].model = ModelName::Opus;
    owners[0].effort = ReasoningEffort::Max;
    for child in &mut owners[0].children {
        child.depth = 0;
        child.kind = ChildKind::Subagent(SubagentRole::Reviewer);
        child.model = ModelName::Sol;
        child.effort = ReasoningEffort::XHigh;
    }
    let mut blank = owners[0].children[0].clone();
    blank.pid = Some(13);
    blank.name = "blank-work".into();
    blank.model = ModelName::Unrecorded;
    blank.effort = ReasoningEffort::Unrecorded;
    owners[0].children.push(blank);
    Ok(owners)
}

#[test]
fn drawn_model_header_sorts_changed_values_and_blanks() -> TestResult {
    assert_changed_metadata_sort("model", TreeColumn::Model)
}

#[test]
fn drawn_effort_header_sorts_changed_values_and_blanks() -> TestResult {
    assert_changed_metadata_sort("effort", TreeColumn::Effort)
}

fn assert_changed_metadata_sort(header: &str, column: TreeColumn) -> TestResult {
    let mut app = App::new_for_test()?;
    let mut owners = model_rows()?;
    owners[0].children[0].model = ModelName::Haiku;
    owners[0].children[0].effort = ReasoningEffort::High;
    owners[0].children[1].model = ModelName::Sol;
    owners[0].children[1].effort = ReasoningEffort::Low;
    install(&mut app, owners.clone());
    let before = paint(&mut app, 260)?;
    assert!(
        group_header(&app, &before, "first-work")?.y
            < group_header(&app, &before, "nested-work")?.y
    );
    let point = group_header(&app, &before, header)?;
    owners[0].children[0].model = ModelName::Sol;
    owners[0].children[0].effort = ReasoningEffort::Low;
    owners[0].children[1].model = ModelName::Haiku;
    owners[0].children[1].effort = ReasoningEffort::High;
    install(&mut app, owners);
    app.click(point);
    assert!(app.tree_sorts.values().any(|sort| sort.column == column));
    let after = paint(&mut app, 260)?;
    let blank = group_header(&app, &after, "blank-work")?;
    let first = group_header(&app, &after, "first-work")?;
    let second = group_header(&app, &after, "nested-work")?;
    assert!(
        blank.y < second.y && second.y < first.y,
        "{header}: {blank:?}, {first:?}, {second:?}"
    );
    Ok(())
}

#[test]
fn drawn_model_and_effort_headers_survive_removed_values() -> TestResult {
    for (header, column) in [("model", "Model"), ("effort", "Effort")] {
        let mut app = App::new_for_test()?;
        let mut owners = model_rows()?;
        install(&mut app, owners.clone());
        let before = paint(&mut app, 260)?;
        let point = group_header(&app, &before, header)?;
        owners[0].model = ModelName::Unrecorded;
        owners[0].effort = ReasoningEffort::Unrecorded;
        for child in &mut owners[0].children {
            child.model = ModelName::Unrecorded;
            child.effort = ReasoningEffort::Unrecorded;
        }
        install(&mut app, owners);
        app.click(point);
        assert!(
            app.tree_sorts
                .values()
                .any(|sort| format!("{:?}", sort.column) == column)
        );
    }
    Ok(())
}

#[test]
fn moving_model_and_effort_headers_use_the_painted_spans() -> TestResult {
    for (header, column) in [("model", "Model"), ("effort", "Effort")] {
        let mut app = App::new_for_test()?;
        let mut owners = model_rows()?;
        let mut other = word_click_tests::session(20, "second-owner", "second-work")?;
        other.children[0].pid = Some(21);
        owners.push(other);
        install(&mut app, owners);
        let area = Rect::new(0, 0, 300, 60);
        body(&mut app, area)?;
        app.tiles.settle_for_test();
        body(&mut app, area)?;
        app.tiles
            .apply(TileAction::Add, app.loaded_config.config.tiles.growth());
        app.tiles.hold_motion_at_half_for_test();
        let moving = body(&mut app, area)?;
        assert!(app.tiles.has_pending_motion_for_test());
        app.click(group_header(&app, &moving, header)?);
        assert!(
            app.tree_sorts
                .values()
                .any(|sort| format!("{:?}", sort.column) == column)
        );
    }
    Ok(())
}

#[test]
fn a_drawn_role_header_survives_empty_current_columns() -> TestResult {
    for width in [220, 60] {
        let mut app = App::new_for_test()?;
        install(&mut app, rows()?);
        let buffer = paint(&mut app, width)?;
        let point = group_header(&app, &buffer, "role")?;
        let mut changed = rows()?;
        changed[0].children[0].kind = ChildKind::Subagent(SubagentRole::Unrecorded);
        let shell = changed[0].children[1].clone();
        changed[0].children.extend((100..200).map(|pid| {
            let mut row = shell.clone();
            row.depth = 0;
            row.pid = Some(pid);
            row.name = format!("new-work-{pid}");
            row
        }));
        install(&mut app, changed);
        app.click(point);
        assert!(
            app.tree_sorts
                .values()
                .any(|sort| sort.column == TreeColumn::Role)
        );
    }
    Ok(())
}

#[test]
fn two_word_presses_and_a_wheel_step_use_one_painted_frame() -> TestResult {
    let mut app = word_click_tests::application()?;
    let mut renderer = word_click_tests::Renderer::new(&mut app, 200, 70)?;
    let buffer = renderer.paint(&mut app)?;
    let first = word_click_tests::locate(&buffer, "first-work")?;
    let second = word_click_tests::locate(&buffer, "second-work")?;
    app.click(first);
    let mut changed = word_click_tests::session(20, "second-owner", "new-unpainted")?;
    changed.children[0].pid = Some(21);
    install(
        &mut app,
        vec![
            word_click_tests::session(10, "first-owner", "first-work")?,
            changed,
        ],
    );
    app.scroll(first, ScrollDirection::Down);
    app.click(second);
    let WordHighlight::Word(word) = &app.word_highlight else {
        return Err("the second painted word did not resolve".into());
    };
    assert_eq!(word.text(), "second-work");
    app.click(second);
    assert_eq!(
        app.word_highlight,
        crate::word_highlight::WordHighlight::Off
    );
    Ok(())
}

#[test]
fn a_retired_painted_marker_cannot_change_expansion_or_selection() -> TestResult {
    let mut app = App::new_for_test()?;
    install(&mut app, rows()?);
    let buffer = paint(&mut app, 200)?;
    let work = word_click_tests::locate(&buffer, "first-work")?;
    let marker = (buffer.area.x..work.x)
        .rev()
        .find(|x| matches!(buffer[(*x, work.y)].symbol(), "▶" | "▼"))
        .ok_or("missing branch marker")?;
    let before: Vec<_> = app
        .painted_surfaces
        .iter()
        .map(|surface| (surface.surface(), surface.rows.selection()))
        .collect();
    let mut changed = rows()?;
    changed[0].children.remove(0);
    changed[0].children[0].depth = 0;
    install(&mut app, changed);
    app.click(Position::new(marker, work.y));
    for (surface, selection) in before {
        assert_eq!(
            app.selections.surface(&surface).map_or(
                TileSelection::NoRows,
                super::super::interaction::SurfaceSelection::selection
            ),
            selection
        );
    }
    let after = paint(&mut app, 200)?;
    assert_eq!(word_click_tests::positions(&after, "nested-work").len(), 1);
    Ok(())
}

#[test]
fn a_removed_tile_header_cannot_restore_a_pruned_sort() -> TestResult {
    let mut app = App::new_for_test()?;
    install(&mut app, rows()?);
    let buffer = paint(&mut app, 200)?;
    let point = group_header(&app, &buffer, "role")?;
    if let CellIndex::Current(index) = app.tiles.cell_index(
        &app.tiles
            .placements(app.selection_area, app.loaded_config.config.tiles.growth())
            .into_iter()
            .find(|placement| matches!(placement.content, TileContent::Group(_)))
            .ok_or("missing group")?
            .content,
    ) {
        app.tiles.focus_cell(index);
    }
    let focus = app.tiles.focused_cell();
    install(&mut app, Vec::new());
    app.prune_tree_sorts();
    app.click(point);
    assert!(app.tree_sorts.is_empty());
    assert_eq!(app.tiles.focused_cell(), focus);
    Ok(())
}

#[test]
fn a_pool_header_uses_its_drawn_columns_after_new_accounts_arrive() -> TestResult {
    for width in [220, 60] {
        let mut app = word_click_tests::application()?;
        let buffer = paint(&mut app, width)?;
        let pool = app
            .painted_surfaces
            .iter()
            .find(|surface| {
                matches!(
                    &surface.surface(),
                    SelectionSurface::Tile(TileKey::Unassigned { .. })
                )
            })
            .ok_or("missing pool")?;
        let PaintedSurfaceContent::Tree { spans, .. } = &pool.content else {
            return Err("missing pool headers".into());
        };
        let header = spans
            .iter()
            .find(|span| span.column == TreeColumn::Session)
            .ok_or("missing session")?;
        let point = header
            .columns
            .clone()
            .map(|column| Position::new(pool.area.x + column, pool.area.y + header.row))
            .find(|point| buffer[*point].symbol().chars().any(char::is_alphabetic))
            .ok_or("missing painted session header")?;
        let mut changed = word_click_tests::session(30, "changed-pool", "")?;
        changed.account = serde_json::from_value(
            serde_json::json!({"home":"~/.claude99","label":"a-new-account-with-a-long-name"}),
        )?;
        install(&mut app, vec![changed]);
        app.click(point);
        assert!(
            app.tree_sorts
                .iter()
                .any(|(key, sort)| matches!(key, TileKey::Unassigned { .. })
                    && sort.column == TreeColumn::Session)
        );
    }
    Ok(())
}

fn body(app: &mut App, area: Rect) -> TestResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| draw_panes_at(frame, app, area, TileGridContents::Shown, 100))?;
    Ok(terminal.backend().buffer().clone())
}

#[test]
fn a_moving_marker_toggles_the_row_painted_under_the_pointer() -> TestResult {
    let mut app = App::new_for_test()?;
    let mut owners = rows()?;
    let mut other = word_click_tests::session(20, "second-owner", "second-work")?;
    other.children[0].pid = Some(21);
    owners.push(other);
    install(&mut app, owners);
    let area = Rect::new(0, 0, 200, 60);
    body(&mut app, area)?;
    app.tiles.settle_for_test();
    body(&mut app, area)?;
    app.tiles
        .apply(TileAction::Add, app.loaded_config.config.tiles.growth());
    app.tiles.hold_motion_at_half_for_test();
    let moving = body(&mut app, area)?;
    assert!(app.tiles.has_pending_motion_for_test());
    let work = word_click_tests::locate(&moving, "first-work")?;
    let x = (0..work.x)
        .rev()
        .find(|x| matches!(moving[(*x, work.y)].symbol(), "▶" | "▼"))
        .ok_or("missing moving marker")?;
    app.click(Position::new(x, work.y));
    app.tiles.settle_for_test();
    let after = body(&mut app, area)?;
    assert_ne!(
        word_click_tests::positions(&after, "nested-work"),
        [] as [ratatui::prelude::Position; 0]
    );
    Ok(())
}

#[test]
fn a_scrolled_marker_keeps_its_painted_row_after_a_wheel_step() -> TestResult {
    let mut app = App::new_for_test()?;
    let mut owners = rows()?;
    let leaf = owners[0].children[1].clone();
    owners[0].children.extend((100..180).map(|pid| {
        let mut row = leaf.clone();
        row.depth = 0;
        row.started = 0;
        row.pid = Some(pid);
        row.name = format!("tail-{pid}");
        row
    }));
    owners[0].children.extend((200..205).map(|pid| {
        let mut row = leaf.clone();
        row.depth = 0;
        row.started = 2;
        row.pid = Some(pid);
        row.name = format!("after-{pid}");
        row
    }));
    install(&mut app, owners);
    let area = Rect::new(0, 0, 200, 60);
    body(&mut app, area)?;
    app.tiles.settle_for_test();
    body(&mut app, area)?;
    let (surface, identity) = {
        let cells = Cells::new(
            app.census.machines(&[]),
            100,
            app.footer_colors.colors(),
            &app.tree_sorts,
            app.summary_sort,
            &AccountsTable::NotConfigured,
        )
        .with_expansion(&app.expansion);
        let tile = cells.tiles.first().ok_or("missing scroll tile")?;
        let (_, identity) = tile
            .named_row_identities()
            .into_iter()
            .find(|(name, _)| name == "after-204")
            .ok_or("missing scroll row")?;
        (SelectionSurface::Tile(tile.key()), identity)
    };
    let SurfaceLookup::Current(state) = app.selections.surface_mut(&surface) else {
        return Err("missing scroll selection".into());
    };
    state.select(&identity);
    let before = body(&mut app, area)?;
    let work = group_header(&app, &before, "first-work")?;
    let surface = app
        .painted_surfaces
        .iter()
        .find(|surface| surface.area.contains(work))
        .ok_or("missing scrolled surface")?;
    assert!(surface.rows.viewport().scroll_offset() > 0);
    let x = (0..work.x)
        .rev()
        .find(|x| matches!(before[(*x, work.y)].symbol(), "▶" | "▼"))
        .ok_or("missing scrolled marker")?;
    app.scroll(work, ScrollDirection::Down);
    app.click(Position::new(x, work.y));
    app.tiles.settle_for_test();
    let after = body(&mut app, area)?;
    assert_ne!(
        word_click_tests::positions(&after, "nested-work"),
        [] as [ratatui::prelude::Position; 0]
    );
    Ok(())
}

#[test]
fn a_later_machine_header_keeps_its_row_after_an_earlier_notice_changes() -> TestResult {
    use crate::census::HierarchyNotice;
    use crate::census::HierarchySnapshot;
    use crate::census::MachineState;
    use crate::census::hierarchy::HierarchyError;
    let mut app = App::new_for_test()?;
    app.loaded_config.config.machines.remote = vec!["remote".into()];
    install(
        &mut app,
        vec![word_click_tests::session(10, "local-owner", "local-work")?],
    );
    app.census.apply(CensusUpdate::Remote {
        host:  "remote".into(),
        state: MachineState::Answered(
            vec![word_click_tests::session(
                20,
                "remote-owner",
                "remote-work",
            )?],
            HierarchyEdges::default().into(),
        ),
    });
    let buffer = paint(&mut app, 200)?;
    let summary = app.painted_summary.borrow();
    let PaintedSummary::Drawn(layout) = &*summary else {
        return Err("missing summary".into());
    };
    let area = app
        .painted_surfaces
        .iter()
        .find(|surface| surface.surface() == SelectionSurface::Summary)
        .ok_or("missing summary surface")?
        .area;
    let spans = layout.header_spans(Rect::new(0, 0, area.width, u16::try_from(layout.height())?));
    let span = spans
        .iter()
        .rev()
        .find(|span| span.column == SummaryColumn::Name)
        .ok_or("missing remote header")?;
    let point = Position::new(area.x + span.columns.start, area.y + span.row);
    assert!(buffer[point].symbol().chars().any(char::is_alphabetic));
    drop(summary);
    app.census.apply(CensusUpdate::Local {
        rows:      vec![word_click_tests::session(10, "local-owner", "local-work")?],
        hierarchy: HierarchySnapshot {
            edges:  HierarchyEdges::default(),
            notice: HierarchyNotice::Failed(HierarchyError::TimedOut),
        },
        stage:     ScanStage::Completed,
    });
    app.click(point);
    assert_eq!(app.summary_sort.column, SummaryColumn::Name);
    Ok(())
}

#[test]
fn a_session_that_moves_between_tiles_keeps_focus_during_motion() -> TestResult {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.widen_summary = false;
    app.loaded_config.config.tiles.initial_rows = serde_json::from_value(serde_json::json!(4))?;
    let mut owners = rows()?;
    owners.push(word_click_tests::session(
        20,
        "second-owner",
        "second-work",
    )?);
    owners.push(word_click_tests::session(40, "third-owner", "third-work")?);
    let mut moving = word_click_tests::session(30, "moving-session", "")?;
    moving.launched_by = Some(10);
    owners.push(moving);
    install(&mut app, owners.clone());
    let before = paint(&mut app, 200)?;
    let area = app.selection_area;
    app.click(group_header(&app, &before, "moving-session")?);
    let identity = RowIdentity::session("natedev", &owners[3]);
    assert!(
        app.painted_surfaces.iter().any(|surface| app
            .selections
            .surface(&surface.surface())
            .map_or(false, |state| state.selection()
                == TileSelection::Row(identity.clone()))),
        "initial selected identity missing: {:?}; selections {:?}",
        identity,
        app.selections
    );
    app.tiles.settle_for_test();
    body(&mut app, area)?;
    owners[3].launched_by = Some(40);
    install(&mut app, owners);
    app.tiles
        .apply(TileAction::Add, app.loaded_config.config.tiles.growth());
    app.tiles.hold_motion_at_half_for_test();
    body(&mut app, area)?;
    assert!(app.tiles.has_pending_motion_for_test());
    let destination = app
        .painted_surfaces
        .iter()
        .find(|surface| surface.rows.selection() == TileSelection::Row(identity.clone()))
        .ok_or_else(|| {
            format!(
                "missing moved session selection: {:?}; focus {:?}; painted {:?}",
                app.selections,
                app.tiles.focused_cell(),
                app.painted_surfaces
                    .iter()
                    .map(|surface| (surface.surface(), surface.rows.selection()))
                    .collect::<Vec<_>>()
            )
        })?;
    let SelectionSurface::Tile(key) = &destination.surface() else {
        return Err("session did not move to a tree tile".into());
    };
    let CellIndex::Current(index) = app.tiles.cell_index(&TileContent::Group(key.clone())) else {
        return Err("missing destination tile".into());
    };
    assert_eq!(
        app.tiles.focused_cell(),
        tui_pane::FocusLocation::Cell(index)
    );
    check_covered_destination(&mut app, area, &TileContent::Group(key.clone()), index)?;
    Ok(())
}

fn check_covered_destination(
    app: &mut App,
    area: Rect,
    content: &TileContent,
    index: usize,
) -> TestResult {
    let moving_placements = app
        .tiles
        .placements(area, app.loaded_config.config.tiles.growth());
    app.tiles.settle_for_test();
    let destination = app
        .tiles
        .placements(area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| &placement.content == content)
        .ok_or("missing destination placement")?
        .frame
        .rect();
    let centre = Position::new(
        destination.x + destination.width / 2,
        destination.y + destination.height / 2,
    );
    let covered = app
        .tiles
        .cell_at(centre)
        .ok_or("destination centre has no painted piece")?;
    assert_ne!(
        covered, index,
        "another moving piece must cover the destination centre {centre:?}; placements {moving_placements:?}"
    );
    let covering = moving_placements
        .iter()
        .find(|placement| app.tiles.cell_index(&placement.content) == CellIndex::Current(covered))
        .ok_or("missing covering piece")?;
    let settled = app
        .tiles
        .placements(area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| placement.content == covering.content)
        .ok_or("missing covering destination")?;
    assert_ne!(
        covering.frame.rect(),
        settled.frame.rect(),
        "the covering piece must move"
    );
    Ok(())
}

#[test]
fn a_tree_header_keeps_shared_widths_when_another_tile_changes() -> TestResult {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.initial_rows = serde_json::from_value(serde_json::json!(8))?;
    app.loaded_config.config.tiles.widen_summary = false;
    let mut owners = rows()?;
    owners[0].started = census::unix_now().saturating_sub(60);
    let mut neighbor = word_click_tests::session(20, "neighbor", "neighbor-work")?;
    neighbor.started = owners[0].started;
    neighbor.children[0].kind = ChildKind::Subagent(SubagentRole::Impl);
    owners.push(neighbor);
    install(&mut app, owners.clone());
    let buffer = paint(&mut app, 200)?;
    let point = group_header(&app, &buffer, "role")?;
    owners[1].started = 1;
    owners[1].children[0].kind = ChildKind::Subagent(SubagentRole::Showrunner);
    install(&mut app, owners);
    app.click(point);
    assert!(
        app.tree_sorts
            .values()
            .any(|sort| sort.column == TreeColumn::Role)
    );
    Ok(())
}

fn marker(buffer: &Buffer, row: Position) -> TestResult<Position> {
    (0..row.x)
        .rev()
        .find(|x| matches!(buffer[(*x, row.y)].symbol(), "▶" | "▼"))
        .map(|x| Position::new(x, row.y))
        .ok_or_else(|| "missing painted marker".into())
}

#[test]
fn pressing_the_root_row_replaces_the_child_selection() -> TestResult {
    let mut app = App::new_for_test()?;
    let owners = rows()?;
    let root = RowIdentity::session("natedev", &owners[0]);
    install(&mut app, owners);
    let buffer = paint(&mut app, 200)?;
    let child = group_header(&app, &buffer, "first-work")?;
    let parent = group_header(&app, &buffer, "owner")?;
    let child_identity = {
        let cells = Cells::new(
            app.census.machines(&[]),
            100,
            app.footer_colors.colors(),
            &app.tree_sorts,
            app.summary_sort,
            &AccountsTable::NotConfigured,
        )
        .with_expansion(&app.expansion);
        cells
            .tiles
            .iter()
            .flat_map(AgentTile::named_row_identities)
            .find(|(name, _)| name == "first-work")
            .ok_or("missing child identity")?
            .1
    };
    app.click(child);
    let surface = app
        .painted_surfaces
        .iter()
        .find(|surface| surface.area.contains(parent))
        .ok_or("missing tree surface")?
        .surface();
    assert_eq!(
        app.selections
            .surface(&surface)
            .map_or(TileSelection::NoRows, SurfaceSelection::selection),
        TileSelection::Row(child_identity)
    );
    app.click(parent);
    assert_eq!(
        app.selections
            .surface(&surface)
            .map_or(TileSelection::NoRows, SurfaceSelection::selection),
        TileSelection::Row(root)
    );
    Ok(())
}

#[test]
fn pressing_the_root_marker_collapses_its_children() -> TestResult {
    let mut app = App::new_for_test()?;
    install(&mut app, rows()?);
    let buffer = paint(&mut app, 200)?;
    let parent = group_header(&app, &buffer, "owner")?;
    let point = marker(&buffer, parent)?;
    assert_eq!(buffer[point].symbol(), "▼");
    app.click(point);
    let after = paint(&mut app, 200)?;
    let parent = group_header(&app, &after, "owner")?;
    assert_eq!(after[marker(&after, parent)?].symbol(), "▶");
    assert_eq!(
        word_click_tests::positions(&after, "first-work"),
        [] as [Position; 0]
    );
    Ok(())
}

/// `SummaryHeight` changes demand without changing the grid rules.
struct SummaryHeight<'a> {
    cells: Cells<'a>,
    rows:  u16,
}

impl TileCells<TileKey> for SummaryHeight<'_> {
    fn summary_title(&self) -> &'static str { "summary" }
    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
        self.cells.prepare_frame(allocations);
    }
    fn prepare_moving_frame(
        &self,
        destination: impl FnOnce() -> Vec<(TileContent, Rect)>,
        bounds: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) {
        self.cells.prepare_moving_frame(destination, bounds);
    }
    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        let mut demands = self.cells.demands(widths);
        demands.summary = usize::from(self.rows);
        demands
    }
    fn rows_drawn(&self, content: &TileContent, area: Rect) -> u16 {
        self.cells.rows_drawn(content, area)
    }
    fn draw(&self, buffer: &mut Buffer, content: &TileContent, area: Rect, color: Color) {
        self.cells.draw(buffer, content, area, color);
    }
}

fn demand_frame(app: &mut App, area: Rect, rows: u16) -> Buffer {
    let cells = SummaryHeight {
        cells: Cells::new(
            app.census.machines(&[]),
            100,
            app.footer_colors.colors(),
            &app.tree_sorts,
            app.summary_sort,
            &AccountsTable::NotConfigured,
        )
        .with_expansion(&app.expansion)
        .with_selections(app.selections.clone(), SelectableSurface::EmptyCell),
        rows,
    };
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        app.loaded_config.config.tiles.growth(),
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    app.selection_area = area;
    app.painted_surfaces = cells.cells.painted_surfaces.into_inner();
    app.selections = cells.cells.selections.into_inner();
    buffer
}

#[test]
fn a_shrinking_tile_header_sorts_the_column_painted_during_motion() -> TestResult {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.initial_rows = serde_json::from_value(serde_json::json!(8))?;
    app.loaded_config.config.tiles.widen_summary = false;
    install(&mut app, rows()?);
    let area = Rect::new(0, 0, 180, 22);
    demand_frame(&mut app, area, 1);
    app.tiles.settle_for_test();
    demand_frame(&mut app, area, 1);
    let before = app
        .painted_surfaces
        .iter()
        .find(|surface| matches!(surface.content, PaintedSurfaceContent::Tree { .. }))
        .ok_or("missing original tile")?
        .area
        .height;
    app.tiles.hold_motion_at_half_for_test();
    demand_frame(&mut app, area, 12);
    let moving = demand_frame(&mut app, area, 12);
    assert!(app.tiles.has_pending_motion_for_test());
    let point = group_header(&app, &moving, "role")?;
    app.click(point);
    assert!(
        app.tree_sorts
            .values()
            .any(|sort| sort.column == TreeColumn::Role)
    );
    app.tiles.settle_for_test();
    demand_frame(&mut app, area, 12);
    let after = app
        .painted_surfaces
        .iter()
        .find(|surface| matches!(surface.content, PaintedSurfaceContent::Tree { .. }))
        .ok_or("missing smaller tile")?
        .area
        .height;
    assert!(after < before, "the destination tile must shrink");
    Ok(())
}

#[test]
fn a_live_marker_outside_the_painted_width_does_not_toggle() -> TestResult {
    let mut app = App::new_for_test()?;
    install(&mut app, rows()?);
    let buffer = paint(&mut app, 200)?;
    let parent = group_header(&app, &buffer, "owner")?;
    let point = marker(&buffer, parent)?;
    let surface = app
        .painted_surfaces
        .iter()
        .find(|surface| surface.area.contains(parent))
        .ok_or("missing tree")?;
    let PaintedSurfaceContent::Tree { key, .. } = &surface.content else {
        return Err("missing tree identity".into());
    };
    let key = key.clone();
    let area = Rect::new(
        surface.area.x,
        surface.area.y,
        point.x - surface.area.x + 1,
        surface.area.height,
    );
    let cells = Cells::new(
        app.census.machines(&[]),
        100,
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
        &AccountsTable::NotConfigured,
    )
    .with_expansion(&app.expansion)
    .with_selections(app.selections.clone(), SelectableSurface::EmptyCell);
    cells.prepare_frame(|| vec![(TileContent::Group(key.clone()), area)]);
    let mut clipped = Buffer::empty(buffer.area);
    cells.draw(
        &mut clipped,
        &TileContent::Group(key.clone()),
        area,
        Color::Reset,
    );
    app.painted_surfaces = cells.painted_surfaces.into_inner();
    app.selections = cells.selections.into_inner();
    assert!(area.contains(point));
    assert_ne!(clipped[point].symbol(), "▼");
    cell_clicked(
        &mut app,
        CellPointer {
            index: 2,
            content: TileContent::Group(key),
            area,
            point: CellPoint {
                column: point.x - area.x,
                row:    point.y - area.y,
            },
        },
    );
    let after = paint(&mut app, 200)?;
    let parent = group_header(&app, &after, "owner")?;
    assert_eq!(after[marker(&after, parent)?].symbol(), "▼");
    Ok(())
}
