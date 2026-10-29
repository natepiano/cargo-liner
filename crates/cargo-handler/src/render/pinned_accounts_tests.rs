//! Account headers and their body share drawing, navigation and click coordinates.

#![expect(
    clippy::expect_used,
    reason = "drawn fixture rows and surfaces must exist"
)]

use crossterm::event::KeyCode;
use ratatui::layout::Position;
use tui_pane::FocusLocation;
use tui_pane::NavAction;
use tui_pane::TerminalApp;
use tui_pane::TileView;

use super::accounts_grid_tests;
use super::*;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::scan::ScanStage;
use crate::interaction::RowClick;
use crate::interaction::RowIdentity;
use crate::interaction::RowLocation;
use crate::interaction::RowSpan;
use crate::interaction::SelectableSurface;
use crate::interaction::SelectedSpan;
use crate::interaction::Selections;
use crate::interaction::SurfaceLookup;
use crate::interaction::TileSelection;

const NOW: u64 = 1_792_166_400;
const WIDTHS: [u16; 3] = [200, 36, 18];

#[test]
fn pinned_accounts_departing_tile_click_does_not_focus_or_select_its_replacement() {
    let mut app = fixture();
    app.loaded_config.config.tiles.view = TileView::Cells;
    app.loaded_config.config.tiles.widen_summary = false;
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(4)).expect("four tiles in one column");
    paint(&mut app, 200, 80);
    let growth = app.loaded_config.config.tiles.growth();
    let area = app.selection_area;
    let departing = app
        .tiles
        .placements(area, growth)
        .into_iter()
        .find(|placement| matches!(&placement.content, TileContent::Group(_)))
        .expect("first group tile")
        .content;
    let rows = app.census.machines(&[])[0].state.rows()[1..].to_vec();
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 80))
        .expect("motion terminal");
    terminal
        .draw(|frame| {
            draw_panes_at(frame, &mut app, area, TileGridContents::Shown, NOW);
        })
        .expect("removal frame");
    let placements = app.tiles.placements(area, growth);
    let departed = placements
        .iter()
        .find(|placement| placement.content == departing)
        .expect("departing tile is still drawn during removal");
    let body = departed.frame.inner().intersection(departed.frame.clip());
    assert!(body.height > 2, "departure still has visible body rows");
    let focused = app.tiles.focused_cell();
    let point = body
        .positions()
        .find(|point| {
            app.tiles
                .cell_at(*point)
                .is_some_and(|index| FocusLocation::Cell(index) != focused)
        })
        .expect("the drawn departure overlaps a destination replacement");
    let selections: Vec<_> = placements
        .iter()
        .filter_map(|placement| match selection_surface(&placement.content) {
            SelectableSurface::EmptyCell => None,
            SelectableSurface::Surface(surface) => app
                .selections
                .surface(&surface)
                .map_or(None, |state| Some((surface, state.selection()))),
        })
        .collect();
    app.click(point);
    assert_eq!(
        app.tiles.focused_cell(),
        focused,
        "a click on the drawn departure must not focus its destination replacement"
    );
    for (surface, before) in selections {
        assert_eq!(
            app.selections
                .surface(&surface)
                .expect("retained surface")
                .selection(),
            before,
            "a click on the departure must not select a replacement row"
        );
    }
}

fn fixture() -> App {
    let mut app = accounts_grid_tests::interactive_accounts();
    app.loaded_config.config.tiles.view = TileView::Summary;
    let rows = (1..=3)
        .map(|pid| {
            serde_json::from_value(serde_json::json!({
                "agent":"claude", "pid":pid, "name":format!("boundary-agent-{pid}"),
                "session_id":format!("boundary-{pid}"), "started":100,
                "directory":"/fixture", "children":[{
                    "depth":0,"kind":"shell","pid":1000+pid,"name":"busy","started":100
                }]
            }))
            .expect("boundary session")
        })
        .collect();
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    app
}

fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("summary terminal");
    for settled in [false, true] {
        if settled {
            app.tiles.settle_for_test();
        }
        terminal
            .draw(|frame| {
                draw_panes_at(
                    frame,
                    app,
                    Rect::new(0, 0, width, height),
                    TileGridContents::Shown,
                    NOW,
                );
            })
            .expect("summary frame");
    }
    let area = app
        .selections
        .surface(&SelectionSurface::Summary)
        .expect("summary surface")
        .viewport()
        .content_area();
    let mut content = Buffer::empty(area);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            content[(x, y)] = terminal.backend().buffer()[(x, y)].clone();
        }
    }
    content
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| accounts_grid_tests::account_line(buffer, y))
        .collect()
}

fn headers(buffer: &Buffer) -> Vec<String> {
    let text = lines(buffer);
    let heading = text
        .iter()
        .position(|line| line.trim() == "accounts")
        .expect("accounts heading");
    let first = text
        .iter()
        .position(|line| line.contains("account-0"))
        .expect("first account");
    text[heading..first].to_vec()
}

fn select_first(app: &mut App, buffer: &Buffer) {
    let first = accounts_grid_tests::locate(buffer, "account-0");
    app.click(Position::new(first.column, first.row));
}

fn selected(app: &App) -> TileSelection {
    app.selections
        .surface(&SelectionSurface::Summary)
        .expect("summary selection")
        .selection()
}

fn assert_pinned(buffer: &Buffer, expected: &[String]) {
    let text = lines(buffer);
    assert_eq!(
        &text[..expected.len()],
        expected,
        "pinned heading and fitted headers: {text:#?}"
    );
    assert_eq!(
        text.iter().filter(|line| line.trim() == "accounts").count(),
        1,
        "the section heading draws once"
    );
    for (index, header) in expected.iter().enumerate() {
        assert_eq!(
            text.iter().filter(|line| *line == header).count(),
            1,
            "header draws once"
        );
        let y = buffer.area.y + u16::try_from(index).expect("header row");
        for x in buffer.area.x..buffer.area.right() {
            assert_ne!(
                buffer[(x, y)].bg,
                tui_pane::active_focus_color(),
                "pinned lines never take the band"
            );
        }
    }
}

fn pinned_end(width: u16) {
    let mut app = fixture();
    let complete = paint(&mut app, width, 64);
    let expected = headers(&complete);
    select_first(&mut app, &complete);
    paint(&mut app, width, 10);
    accounts_grid_tests::account_key(&mut app, KeyCode::End);
    let end = paint(&mut app, width, 10);
    assert_pinned(&end, &expected);
    assert_eq!(
        selected(&app),
        TileSelection::Row(RowIdentity::SummaryAccount {
            account: "account-5".into(),
        })
    );
    let before = selected(&app);
    app.click(Position::new(end.area.x, end.area.y));
    assert_eq!(
        selected(&app),
        before,
        "pinned heading does not select a row"
    );
}

#[test]
fn pinned_accounts_full_headers_survive_end() { pinned_end(200); }
#[test]
fn pinned_accounts_two_line_headers_survive_end() { pinned_end(36); }
#[test]
fn pinned_accounts_stacked_heading_survives_end() { pinned_end(18); }

#[test]
fn pinned_accounts_column_header_sorts_after_end() {
    let mut app = fixture();
    let complete = paint(&mut app, 200, 64);
    let expected = headers(&complete);
    select_first(&mut app, &complete);
    paint(&mut app, 200, 10);
    accounts_grid_tests::account_key(&mut app, KeyCode::End);
    let end = paint(&mut app, 200, 10);
    assert_pinned(&end, &expected);
    let before = app.accounts_sort;
    let selection = selected(&app);
    let header = accounts_grid_tests::locate(&end, "left");
    app.click(Position::new(header.column, header.row));
    assert_ne!(
        app.accounts_sort, before,
        "pinned column header changes account ordering"
    );
    assert_eq!(
        selected(&app),
        selection,
        "sorting header does not select a body row"
    );
}

#[test]
fn pinned_accounts_continuation_click_uses_the_drawn_body_offset() {
    let mut app = fixture();
    let complete = paint(&mut app, 36, 64);
    let expected = headers(&complete);
    select_first(&mut app, &complete);
    paint(&mut app, 36, 12);
    accounts_grid_tests::account_key(&mut app, KeyCode::End);
    let end = paint(&mut app, 36, 12);
    assert_pinned(&end, &expected);
    let preceding = accounts_grid_tests::locate(&end, "account-4");
    app.click(Position::new(preceding.column, preceding.row + 1));
    assert_eq!(
        selected(&app),
        TileSelection::Row(RowIdentity::SummaryAccount {
            account: "account-4".into(),
        })
    );
}

#[test]
fn pinned_accounts_resize_uses_each_fitted_layout_header_once() {
    let mut app = fixture();
    let initial = paint(&mut app, 200, 24);
    let full = headers(&initial);
    let mut reference = fixture();
    let two_line = headers(&paint(&mut reference, 36, 32));
    let stacked = headers(&paint(&mut reference, 18, 64));
    select_first(&mut app, &initial);
    paint(&mut app, 200, 10);
    accounts_grid_tests::account_key(&mut app, KeyCode::End);
    for (width, expected) in [
        (200, &full),
        (36, &two_line),
        (18, &stacked),
        (36, &two_line),
        (200, &full),
    ] {
        let resized = paint(&mut app, width, 10);
        assert_pinned(&resized, expected);
        assert_eq!(
            selected(&app),
            TileSelection::Row(RowIdentity::SummaryAccount {
                account: "account-5".into(),
            }),
            "resize retains the selected identity"
        );
    }
}

#[test]
fn pinned_accounts_require_headers_and_one_body_line() {
    for width in WIDTHS {
        let mut app = fixture();
        let complete = paint(&mut app, width, 64);
        let expected = headers(&complete);
        select_first(&mut app, &complete);
        let pins = u16::try_from(expected.len()).expect("pin height");
        let chrome = 64 - complete.area.height;
        // At the lower height the pinned lines would leave no body.
        paint(&mut app, width, pins + chrome);
        accounts_grid_tests::account_key(&mut app, KeyCode::End);
        let below = paint(&mut app, width, pins + chrome);
        assert!(
            !lines(&below).iter().any(|line| line.trim() == "accounts"),
            "too short to pin"
        );
        let at = paint(&mut app, width, pins + chrome + 1);
        assert_pinned(&at, &expected);
        assert!(
            !accounts_grid_tests::account_line(&at, at.area.bottom() - 1)
                .trim()
                .is_empty(),
            "one body line remains"
        );
    }
}

#[test]
fn pinned_accounts_cross_the_agent_boundary_down_and_up_without_lost_rows() {
    let mut app = fixture();
    let complete = paint(&mut app, 200, 24);
    let expected = headers(&complete);
    let last_agent = accounts_grid_tests::locate(&complete, "boundary-agent-3");
    app.click(Position::new(last_agent.column, last_agent.row));
    let before = paint(&mut app, 200, 9);
    assert!(
        lines(&before)
            .iter()
            .any(|line| line.contains("boundary-agent-3")),
        "last agent remains above the natural heading"
    );
    accounts_grid_tests::account_key(&mut app, KeyCode::Down);
    let natural = paint(&mut app, 200, 9);
    let agent = accounts_grid_tests::locate(&natural, "boundary-agent-3");
    let heading = accounts_grid_tests::locate(&natural, "accounts");
    assert!(
        heading.row > agent.row,
        "the natural heading stays below the final agent"
    );
    assert_eq!(
        lines(&natural)
            .iter()
            .filter(|line| line.trim() == "accounts")
            .count(),
        1
    );
    for _ in 0..5 {
        accounts_grid_tests::account_key(&mut app, KeyCode::Down);
    }
    assert_pinned(&paint(&mut app, 200, 9), &expected);
    for _ in 0..5 {
        accounts_grid_tests::account_key(&mut app, KeyCode::Up);
    }
    let natural_again = paint(&mut app, 200, 9);
    assert!(
        accounts_grid_tests::locate(&natural_again, "accounts").row
            > accounts_grid_tests::locate(&natural_again, "boundary-agent-3").row
    );
    accounts_grid_tests::account_key(&mut app, KeyCode::Up);
    let returned = paint(&mut app, 200, 9);
    assert_eq!(
        lines(&returned)
            .iter()
            .filter(|line| line.contains("boundary-agent-3"))
            .count(),
        1,
        "last agent returns once"
    );
    assert!(
        lines(&returned)
            .iter()
            .filter(|line| line.trim() == "accounts")
            .count()
            <= 1,
        "natural heading never duplicates"
    );
}

#[test]
fn pinned_accounts_page_navigation_crosses_the_section_boundary() {
    let mut app = fixture();
    let complete = paint(&mut app, 200, 64);
    let expected = headers(&complete);
    select_first(&mut app, &complete);
    paint(&mut app, 200, 9);
    accounts_grid_tests::account_key(&mut app, KeyCode::PageDown);
    assert_pinned(&paint(&mut app, 200, 9), &expected);
    let mut saw_agent = false;
    for _ in 0..3 {
        accounts_grid_tests::account_key(&mut app, KeyCode::PageUp);
        let up = paint(&mut app, 200, 9);
        saw_agent |= lines(&up)
            .iter()
            .any(|line| line.contains("boundary-agent"));
        assert!(
            lines(&up)
                .iter()
                .filter(|line| line.trim() == "accounts")
                .count()
                <= 1
        );
    }
    assert!(saw_agent, "page up exposes agent rows across the boundary");
}

#[test]
fn pinned_accounts_missed_click_keeps_selection_and_retired_surface_stays_gone() {
    let mut selections = Selections::default();
    let identity = RowIdentity::SummaryAccount {
        account: "retired-account".into(),
    };
    selections.resolve(vec![(
        SelectionSurface::Summary,
        vec![(identity.clone(), RowSpan::single(2))],
    )]);
    let state = selections
        .surface_mut(&SelectionSurface::Summary)
        .expect("live surface");
    state.set_area(Rect::new(5, 5, 20, 4));
    assert_eq!(state.click(Position::new(0, 0)), RowClick::Missed);
    assert_eq!(state.selection(), TileSelection::Row(identity.clone()));
    selections.resolve(Vec::new());
    assert!(
        matches!(
            selections.surface(&SelectionSurface::Summary),
            crate::interaction::SurfaceLookup::Retired
        ),
        "retired content has no selectable rows"
    );
    assert!(
        matches!(
            selections.follow_focused(&identity),
            crate::interaction::RowLocation::Gone
        ),
        "retired row cannot focus replacement content"
    );
    let mut empty = crate::interaction::SurfaceSelection::default();
    empty.navigate(NavAction::End);
    assert_eq!(empty.selection(), TileSelection::NoRows);
}

#[test]
fn pinned_accounts_named_states_distinguish_empty_cells_rows_and_retired_content() {
    let mut selections = Selections::default();
    assert!(matches!(
        selection_surface(&TileContent::Summary),
        SelectableSurface::Surface(SelectionSurface::Summary)
    ));
    assert!(matches!(
        selection_surface(&TileContent::Empty(1)),
        SelectableSurface::EmptyCell
    ));
    let empty = crate::interaction::SurfaceSelection::default();
    assert!(matches!(empty.selected_span(), SelectedSpan::NoRows));
    let old = RowIdentity::SummaryAccount {
        account: "old".into(),
    };
    let replacement = RowIdentity::SummaryAccount {
        account: "replacement".into(),
    };
    selections.resolve(vec![(
        SelectionSurface::Summary,
        vec![(old.clone(), RowSpan::single(0))],
    )]);
    assert!(matches!(
        selections.containing(&old),
        RowLocation::Found(SelectionSurface::Summary)
    ));
    assert!(matches!(
        selections.surface(&SelectionSurface::Summary),
        SurfaceLookup::Current(_)
    ));
    selections.resolve(vec![(
        SelectionSurface::Summary,
        vec![(replacement.clone(), RowSpan::single(0))],
    )]);
    assert!(matches!(selections.follow_focused(&old), RowLocation::Gone));
    assert_eq!(
        selections
            .surface(&SelectionSurface::Summary)
            .expect("replacement surface")
            .selection(),
        TileSelection::Row(replacement)
    );
}
