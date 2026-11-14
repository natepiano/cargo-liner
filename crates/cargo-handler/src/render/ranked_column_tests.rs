//! Full tree counts choose columns before visible rows share their height.

use std::collections::HashMap;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use serde_json::json;
use tui_pane::CellIndex;
use tui_pane::ExpansionAction;
use tui_pane::ScrollDirection;
use tui_pane::TerminalApp;
use tui_pane::TileCells;
use tui_pane::TileColumnOrder;
use tui_pane::TileView;

use super::Cells;
use super::PaintedSurfaceContent;
use super::draw_at_for_test;
use super::word_click_tests;
use crate::agent_cell::TreeColumn;
use crate::app::App;
use crate::app::expansion;
use crate::app::expansion::HandlerExpansionAction;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::scan::ScanStage;
use crate::interaction::SelectionSurface;
use crate::interaction::SurfaceLookup;
use crate::tiles::TileContent;
use crate::tiles::TileKey;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const NOW: u64 = 1_794_096_000;

fn rows() -> TestResult<Vec<AgentRow>> {
    let mut rows = Vec::new();
    for (pid, name, count) in [
        (10, "natedev", 42),
        (20, "hana", 42),
        (30, "cargo-liner", 11),
    ] {
        let children: Vec<_> = (1..count)
            .map(|i| {
                json!({
                    "depth":0,"kind":"shell","pid":pid * 1000 + i,
                    "name":format!("task-{pid}-{i:02}"),"started":NOW-10
                })
            })
            .collect();
        rows.push(serde_json::from_value(json!({
            "agent":"claude","pid":pid,"session_id":name,"name":name,
            "started":NOW-1000,"directory":"/fixture","children":children,
            "account":{"home":"~/.claude1","label":"claude1"}
        }))?);
    }
    for pid in 40..43 {
        rows.push(serde_json::from_value(json!({
            "agent":"claude","pid":pid,"session_id":format!("free-{pid}"),
            "name":format!("free-{pid}"),"started":NOW-100,"directory":"/fixture","children":[]
        }))?);
    }
    Ok(rows)
}

pub(super) fn publish(app: &mut App, rows: Vec<AgentRow>) {
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
}

fn application() -> TestResult<(App, Vec<AgentRow>)> {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.view = TileView::Cells;
    app.loaded_config.config.tiles.initial_rows = serde_json::from_value(json!(3))?;
    app.loaded_config.config.tiles.widen_summary = false;
    let rows = rows()?;
    publish(&mut app, rows.clone());
    expansion::dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
    );
    Ok((app, rows))
}

pub(super) fn paint(app: &mut App, width: u16, height: u16) -> TestResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    let keymap = app.keymap();
    for _ in 0..3 {
        app.tiles.settle_for_test();
        terminal.draw(|frame| draw_at_for_test(frame, app, &keymap, NOW))?;
    }
    Ok(terminal.backend().buffer().clone())
}

fn first_frame(app: &mut App, width: u16, height: u16) -> TestResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    let keymap = app.keymap();
    terminal.draw(|frame| draw_at_for_test(frame, app, &keymap, NOW))?;
    Ok(terminal.backend().buffer().clone())
}

pub(super) fn ranks(app: &App) -> TileColumnOrder<TileKey> {
    Cells::new(
        app.census.machines(&[]),
        NOW,
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
        &app.accounts.table,
    )
    .with_expansion(&app.expansion)
    .column_order()
}

pub(super) fn positions(app: &App) -> HashMap<TileKey, Rect> {
    app.tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .filter_map(|p| match p.content {
            TileContent::Group(id) => Some((id, p.frame.inner())),
            _ => None,
        })
        .collect()
}

#[test]
fn ranked_columns_use_complete_handler_trees_before_expansion() -> TestResult {
    let (mut app, _) = application()?;
    let expected = ranks(&app);
    let TileColumnOrder::LongestWithShortest(counts) = &expected else {
        return Err("ranked columns required".into());
    };
    assert_eq!(
        counts.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
        [42, 42, 11, 3]
    );
    let before = paint(&mut app, 181, 70)?;
    let placed = positions(&app);
    let roots: Vec<_> = counts.iter().map(|(key, _)| placed[key]).collect();
    assert_eq!(roots[0].x, roots[3].x);
    assert_eq!(roots[1].x, roots[2].x);
    assert_ne!(roots[0].x, roots[1].x);
    for (rect, marker) in [(roots[0], "task-10-41"), (roots[1], "task-20-41")] {
        assert!(
            rect_text(&before, rect).contains(marker),
            "each long tile draws its last work row"
        );
    }
    app.scroll(
        Position::new(roots[0].x, roots[0].y + 2),
        ScrollDirection::Down,
    );
    assert_eq!(ranks(&app), expected);
    app.click(Position::new(roots[0].x + 3, roots[0].y + 1));
    expansion::dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::CollapseRow),
    );
    let collapsed = paint(&mut app, 181, 70)?;
    assert!(!rect_text(&collapsed, positions(&app)[&counts[0].0]).contains("task-10-41"));
    assert_eq!(ranks(&app), expected);
    paint(&mut app, 181, 52)?;
    assert_eq!(ranks(&app), expected);
    let long = positions(&app)[&counts[1].0];
    for _ in 0..42 {
        app.scroll(Position::new(long.x + 3, long.y + 2), ScrollDirection::Down);
    }
    let surface = SelectionSurface::Tile(counts[1].0.clone());
    let SurfaceLookup::Current(state) = app.selections.surface(&surface) else {
        return Err("long tile selection".into());
    };
    assert!(
        state.viewport().scroll_offset() > 0,
        "the long tile scrolls at the shorter height"
    );
    assert_eq!(ranks(&app), expected);
    Ok(())
}

pub(super) fn rect_text(buffer: &Buffer, rect: Rect) -> String {
    (rect.y..rect.bottom())
        .map(|y| {
            (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_drawn_headers_click(app: &mut App, buffer: &Buffer) -> TestResult {
    let mut starts: HashMap<u16, Vec<u16>> = HashMap::new();
    let mut clicks = Vec::new();
    for surface in &app.painted_surfaces {
        if let PaintedSurfaceContent::Tree { key, spans, .. } = &surface.content {
            let core = [TreeColumn::Role, TreeColumn::Activity, TreeColumn::Status]
                .into_iter()
                .map(|column| {
                    spans
                        .iter()
                        .find(|span| span.column == column)
                        .map(|span| surface.area.x + span.columns.start)
                        .ok_or("core header")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(previous) = starts.insert(surface.area.x, core.clone()) {
                assert_eq!(previous, core);
            }
            let status = spans
                .iter()
                .find(|span| span.column == TreeColumn::Status)
                .ok_or("status header")?;
            let point = Position::new(
                surface.area.x + status.columns.start,
                surface.area.y + status.row,
            );
            assert_eq!(buffer[point].symbol(), "s");
            clicks.push((key.clone(), point));
        }
    }
    assert_ne!(clicks, [] as [(TileKey, Position); 0]);
    for (key, point) in clicks {
        app.click(point);
        assert_eq!(
            app.tree_sorts.get(&key).ok_or("clicked sort")?.column,
            TreeColumn::Status
        );
    }
    Ok(())
}

/// Moving pieces can shift or hide the source header; use the visible buffer positions.
fn assert_visible_headers_click(app: &mut App, buffer: &Buffer) -> TestResult {
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let clicks: Vec<_> = word_click_tests::positions(buffer, "status")
        .into_iter()
        .filter_map(|point| {
            let index = app.tiles.cell_at(point)?;
            placements
                .iter()
                .find_map(|placement| match &placement.content {
                    TileContent::Group(key)
                        if app.tiles.cell_index(&placement.content)
                            == CellIndex::Current(index) =>
                    {
                        Some((key.clone(), point))
                    },
                    _ => None,
                })
        })
        .collect();
    assert!(
        !clicks.is_empty(),
        "the next frame draws clickable tree headers"
    );
    for (key, point) in clicks {
        app.tree_sorts.remove(&key);
        app.click(point);
        assert_eq!(
            app.tree_sorts
                .get(&key)
                .ok_or("clicked visible header")?
                .column,
            TreeColumn::Status
        );
    }
    Ok(())
}

#[test]
fn ranked_columns_refresh_header_positions_after_rank_membership_and_width_changes() -> TestResult {
    let (mut app, mut rows) = application()?;
    paint(&mut app, 181, 70)?;
    let before = positions(&app);
    rows[0].children.truncate(1);
    publish(&mut app, rows.clone());
    let next_frame = first_frame(&mut app, 181, 70)?;
    assert_visible_headers_click(&mut app, &next_frame)?;
    let buffer = paint(&mut app, 181, 70)?;
    assert_ne!(before, positions(&app), "a changed rank reassigns tiles");
    assert_drawn_headers_click(&mut app, &buffer)?;
    rows.remove(2);
    publish(&mut app, rows);
    let next_frame = first_frame(&mut app, 181, 70)?;
    assert_visible_headers_click(&mut app, &next_frame)?;
    let buffer = paint(&mut app, 181, 70)?;
    assert_eq!(positions(&app).len(), 3);
    assert_drawn_headers_click(&mut app, &buffer)?;
    let next_frame = first_frame(&mut app, 40, 70)?;
    assert_visible_headers_click(&mut app, &next_frame)?;
    let buffer = paint(&mut app, 40, 70)?;
    assert!(positions(&app).values().all(|area| area.x == 1));
    assert_drawn_headers_click(&mut app, &buffer)?;
    Ok(())
}
