//! Account demand and independent header sorts reach the actual grid.

#![expect(
    clippy::expect_used,
    reason = "grid fixture rows and header cells must exist"
)]
#![expect(clippy::panic, reason = "missing rendered headers fail the test")]

use std::ops::Range;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::layout::Position;
use tui_pane::AttractWork;
use tui_pane::CellPoint;
use tui_pane::PaneFocusState;
use tui_pane::TerminalApp;
use tui_pane::Updates;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::scan::ScanStage;
use crate::interaction::SelectionSurface;
use crate::sort::AccountsSort;
use crate::sort::SortDirection;
use crate::sort::SummaryColumn;

const NOW: u64 = 1_792_166_400;
// Optional columns leave before layout selection. These widths force the kept
// columns onto two lines or into stacked facts, including the pane frame.
const TWO_LINE_ACCOUNTS_WIDTH: u16 = 36;
const STACKED_ACCOUNTS_WIDTH: u16 = 18;
// The final account and its heading must fit the five-row viewport together.
const COMPLETE_STACKED_ACCOUNT_WIDTH: u16 = 26;

fn locate(buffer: &Buffer, label: &str) -> CellPoint {
    for y in buffer.area.y..buffer.area.bottom() {
        let line: String = (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(index) = line.find(label) {
            return CellPoint {
                column: buffer.area.x
                    + u16::try_from(line[..index].chars().count()).expect("column"),
                row:    y,
            };
        }
    }
    panic!(
        "missing grid label {label}; area={:?} cells={:?}",
        buffer.area, buffer.content
    );
}

fn draw_grid(app: &mut App) -> Buffer {
    let cells = Cells::new(
        app.census.machines(&[]),
        NOW,
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
    )
    .with_accounts(&app.accounts.table)
    .with_accounts_sort(app.accounts_sort);
    let demand = cells.demands(&[]);
    let area = Rect::new(
        0,
        0,
        // An 80-cell terminal must retain all rows even when a sort arrow adds demand.
        80,
        u16::try_from(demand.summary + 3).expect("small summary"),
    );
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        app.loaded_config.config.tiles.growth(),
        TileGridContents::Shown,
        &cells,
    );
    // Demand synchronization can queue layout motion. This test checks final sorting,
    // so complete that transition explicitly instead of sampling its wall clock.
    app.tiles.settle_for_test();
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        app.loaded_config.config.tiles.growth(),
        TileGridContents::Shown,
        &cells,
    );
    // Session labels also occur in tile titles. Read the actual summary surface,
    // rather than selecting whichever copy appears first in the complete grid.
    let summary = app.tiles.painted_summary_frames();
    assert_eq!(summary.len(), 1, "the settled grid paints one summary");
    let inner = summary[0].inner(ratatui::layout::Margin::new(1, 1));
    let mut content = Buffer::empty(inner);
    for y in inner.y..inner.bottom() {
        for x in inner.x..inner.right() {
            content[(x, y)] = buffer[(x, y)].clone();
        }
    }
    *app.painted_summary.borrow_mut() = cells.painted_summary.into_inner();
    content
}

#[test]
fn accounts_grid_measured_tables_sort_account_headers_without_changing_agent_order() {
    let mut app = App::new_for_test().expect("test app");
    // Keep the three fixture tiles in one screen column: a 39-cell summary
    // intentionally omits session names and stacks accounts without headers.
    // The terminal remains 80 cells, so this still exercises narrow demand.
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("single screen column");
    app.loaded_config.config.tiles.widen_summary = false;
    let rows: Vec<AgentRow> = [(10, "alpha-session", 100), (20, "zeta-session", 200)]
        .map(|(pid, name, started)| {
            serde_json::from_value(serde_json::json!({
                "agent":"claude","name":name,"pid":pid,"started":started,"directory":"/fixture",
                "desktop":null,"branch":null,"launched_by":null,"children":[{"depth":0,"kind":"shell","pid":9999,"name":"fixture command","started":100}]
            }))
            .expect("agent fixture")
        })
        .into();
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    app.summary_sort = SummarySort {
        column:    SummaryColumn::Name,
        direction: SortDirection::Descending,
    };
    app.accounts.table = AccountsTable::Ready(AccountsReading::parse(&serde_json::to_vec(&serde_json::json!({
        "accounts":[
            {"label":"codex2","tool":"codex","roles":["workers"],"weekly_left_percent":22,"resets_at":null,"credits":4,"use_credits":true,"read_at":null},
            {"label":"claude1","tool":"claude","roles":["units"],"weekly_left_percent":55,"resets_at":null,"credits":null,"use_credits":false,"read_at":null}
        ],"problems":["fixture stored reading problem"]
    })).expect("reading encodes")).expect("source reading"));
    let before = draw_grid(&mut app);
    assert!(locate(&before, "zeta-session").row < locate(&before, "alpha-session").row);
    assert!(locate(&before, "codex2").row < locate(&before, "claude1").row);
    assert!(
        locate(&before, "fixture stored reading problem").row < before.area.bottom() - 1,
        "the measured demand includes the final note"
    );
    let account_header = locate(&before, "left");
    let original_sort = app.summary_sort;
    cell_clicked(
        &mut app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    before.area,
            point:   CellPoint {
                column: account_header.column - before.area.x,
                row:    account_header.row - before.area.y,
            },
        },
    );
    assert_eq!(app.summary_sort, original_sort);
    let after = draw_grid(&mut app);
    for name in ["zeta-session", "alpha-session", "codex2", "claude1"] {
        assert_eq!(
            locate(&before, name).row,
            locate(&after, name).row,
            "{name}"
        );
    }
    let account_header = locate(&after, "left ▲");
    cell_clicked(
        &mut app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    after.area,
            point:   CellPoint {
                column: account_header.column - after.area.x,
                row:    account_header.row - after.area.y,
            },
        },
    );
    let reversed = draw_grid(&mut app);
    assert!(locate(&reversed, "claude1").row < locate(&reversed, "codex2").row);
    assert!(locate(&reversed, "zeta-session").row < locate(&reversed, "alpha-session").row);
    assert_eq!(app.summary_sort, original_sort);
    // An actual agents header still changes ordering through the same handler.
    let header = locate(&reversed, "session ▼");
    cell_clicked(
        &mut app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    reversed.area,
            point:   CellPoint {
                column: header.column - reversed.area.x,
                row:    header.row - reversed.area.y,
            },
        },
    );
    assert_eq!(app.summary_sort.direction, SortDirection::Ascending);
    let sorted = draw_grid(&mut app);
    assert!(locate(&sorted, "alpha-session").row < locate(&sorted, "zeta-session").row);
    assert!(locate(&sorted, "claude1").row < locate(&sorted, "codex2").row);
}

#[test]
fn failed_accounts_without_agents_keep_the_summary_visible() {
    let mut app = App::new_for_test().expect("test app");
    app.census.apply(CensusUpdate::Local {
        rows:      Vec::new(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    app.accounts.table = AccountsTable::Failed(AccountsFailure::InvalidReading);
    let keymap = std::rc::Rc::clone(&app.keymap);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).expect("test terminal");
    // Start with the attract screen fully visible: the failed source must bring the grid back.
    let area = Rect::new(0, 0, 100, 23);
    let now = std::time::Instant::now();
    for _ in 0..90 {
        app.attract
            .advance(area, AttractWork::Idle, Updates::Live, now);
    }
    for _ in 0..90 {
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("render failure");
    }
    let buffer = terminal.backend().buffer();
    let position = locate(buffer, "accounts: invalid JSON reading");
    assert!(
        buffer[(position.column, position.row)]
            .modifier
            .contains(ratatui::style::Modifier::DIM)
    );
}

fn interactive_accounts() -> App {
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("one screen column");
    app.loaded_config.config.tiles.widen_summary = false;
    app.census.apply(CensusUpdate::Local {
        rows:      Vec::new(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    let accounts: Vec<_> = (0..6)
        .map(|index| {
            serde_json::json!({
                "label":format!("account-{index}"), "tool":"codex",
                "roles":["showrunners", "units"], "weekly_left_percent":60,
                "credits":13_446, "use_credits":true, "credits_in_use":true,
                "read_ok":true, "read_at":"2026-10-16T15:57:53Z",
                "resets_at":"2026-10-17T14:00:00Z"
            })
        })
        .collect();
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({"accounts":accounts,"problems":[]}))
                .expect("account JSON"),
        )
        .expect("account reading"),
    );
    app
}

fn paint_accounts_viewport(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, NOW))
        .expect("initial frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, NOW))
        .expect("settled frame");
    let summaries = app.tiles.painted_summary_frames();
    assert_eq!(summaries.len(), 1, "one summary surface");
    let inner = app
        .selections
        .surface(&SelectionSurface::Summary)
        .expect("summary selection")
        .viewport()
        .content_area();
    let mut summary = Buffer::empty(inner);
    for y in inner.y..inner.bottom() {
        for x in inner.x..inner.right() {
            summary[(x, y)] = terminal.backend().buffer()[(x, y)].clone();
        }
    }
    summary
}

fn account_line(buffer: &Buffer, y: u16) -> String {
    (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn account_key(app: &mut App, code: KeyCode) {
    tui_pane::dispatch_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

fn assert_account_selected(app: &App, buffer: &Buffer, label: &str) {
    assert_eq!(
        app.selections
            .surface(&crate::interaction::SelectionSurface::Summary)
            .expect("summary selection")
            .selection(),
        crate::interaction::TileSelection::Row(crate::interaction::RowIdentity::SummaryAccount {
            account: label.to_owned(),
        }),
        "clicked account identity"
    );
    let point = locate(buffer, label);
    assert_eq!(
        buffer[(point.column, point.row)].bg,
        tui_pane::active_focus_color(),
        "the account heading is visibly selected"
    );
}

fn assert_account_tail_reachable(width: u16, continuation: &str) {
    let mut app = interactive_accounts();
    let complete = paint_accounts_viewport(&mut app, width, 64);
    let final_heading = locate(&complete, "account-5");
    let last_y = (complete.area.y..complete.area.bottom())
        .rev()
        .find(|&y| !account_line(&complete, y).trim().is_empty())
        .expect("final painted line");
    assert!(last_y > final_heading.row, "fixture has a continuation");
    let final_line = account_line(&complete, last_y);
    assert!(
        final_line.contains(continuation),
        "final continuation: {final_line}"
    );
    let first = locate(&complete, "account-0");
    app.click(Position::new(first.column, first.row));
    let short = paint_accounts_viewport(&mut app, width, 9);
    assert!(
        last_y - complete.area.y >= short.area.height,
        "viewport really clips the tail"
    );
    account_key(&mut app, KeyCode::End);
    let end = paint_accounts_viewport(&mut app, width, 9);
    assert_eq!(
        account_line(&end, end.area.bottom() - 1),
        final_line,
        "End must reveal the final account's complete last line"
    );
    assert_account_selected(&app, &end, "account-5");
}

fn assert_account_continuations_select(width: u16) {
    let mut app = interactive_accounts();
    let complete = paint_accounts_viewport(&mut app, width, 64);
    let heading = locate(&complete, "account-1");
    let next = locate(&complete, "account-2");
    assert!(
        next.row > heading.row + 1,
        "fixture has account continuations"
    );
    for y in heading.row + 1..next.row {
        let line = account_line(&complete, y);
        assert!(!line.trim().is_empty(), "each continuation carries a fact");
        account_key(&mut app, KeyCode::Home);
        let home = paint_accounts_viewport(&mut app, width, 64);
        let first = locate(&home, "account-0");
        app.click(Position::new(first.column, first.row));
        let before = paint_accounts_viewport(&mut app, width, 64);
        assert_account_selected(&app, &before, "account-0");
        let x = (before.area.x..before.area.right())
            .find(|&x| !before[(x, y)].symbol().trim().is_empty())
            .expect("painted continuation cell");
        app.click(Position::new(x, y));
        let after = paint_accounts_viewport(&mut app, width, 64);
        assert_account_selected(&app, &after, "account-1");
    }
}

#[test]
fn two_line_accounts_end_reveals_the_final_continuation() {
    // Spec 1 drops read age before TwoLine; reset is the final kept fact.
    assert_account_tail_reachable(TWO_LINE_ACCOUNTS_WIDTH, "Sat 10:00");
}

#[test]
fn stacked_accounts_end_reveals_the_final_fact() {
    // Spec 1 drops read age before Stacked; reset is the final kept fact.
    assert_account_tail_reachable(COMPLETE_STACKED_ACCOUNT_WIDTH, "10:00");
}

#[test]
fn two_line_account_continuation_clicks_select_the_account() {
    assert_account_continuations_select(TWO_LINE_ACCOUNTS_WIDTH);
}

#[test]
fn stacked_account_fact_clicks_select_the_account() {
    assert_account_continuations_select(STACKED_ACCOUNTS_WIDTH);
}

#[test]
fn full_accounts_keep_one_selectable_line_per_account() {
    let mut app = interactive_accounts();
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines);
    let width = summary::width(
        &machines,
        &placement,
        NOW,
        AccountColumn::Hidden,
        SummarySort::default(),
        &app.accounts.table,
        AccountsSort::SourceOrder,
    ) + 2;
    let before = paint_accounts_viewport(&mut app, width, 20);
    let first = locate(&before, "account-0");
    for index in 0..6 {
        let point = locate(&before, &format!("account-{index}"));
        assert_eq!(point.row, first.row + index, "one line per full account");
        let line = account_line(&before, point.row);
        for value in ["60%", "13,446", "2m"] {
            assert!(line.contains(value), "complete full row: {line}");
        }
    }
    let second = locate(&before, "account-1");
    app.click(Position::new(second.column, second.row));
    let clicked = paint_accounts_viewport(&mut app, width, 20);
    assert_account_selected(&app, &clicked, "account-1");
    account_key(&mut app, KeyCode::Down);
    let after = paint_accounts_viewport(&mut app, width, 20);
    assert_account_selected(&app, &after, "account-2");
}

#[test]
fn stacked_account_taller_than_viewport_exposes_every_fact_with_arrows_and_end() {
    let mut app = interactive_accounts();
    let width = STACKED_ACCOUNTS_WIDTH;
    let complete = paint_accounts_viewport(&mut app, width, 64);
    let first = locate(&complete, "account-0");
    let second = locate(&complete, "account-1");
    let facts: Vec<_> = (first.row + 1..second.row)
        .map(|y| account_line(&complete, y))
        .collect();
    app.click(Position::new(first.column, first.row));
    let short = paint_accounts_viewport(&mut app, width, 6);
    assert!(
        facts.len() + 1 > usize::from(short.area.height),
        "one selected account must exceed the viewport"
    );
    let mut observed = Vec::new();
    for _ in 0..facts.len() + 2 {
        let current = paint_accounts_viewport(&mut app, width, 6);
        observed.extend((current.area.y..current.area.bottom()).map(|y| account_line(&current, y)));
        account_key(&mut app, KeyCode::Down);
    }
    for fact in facts {
        assert!(
            observed.contains(&fact),
            "arrow navigation skipped fact {fact:?}: {observed:?}"
        );
    }
    let final_y = (complete.area.y..complete.area.bottom())
        .rev()
        .find(|&y| !account_line(&complete, y).trim().is_empty())
        .expect("final account fact");
    let final_line = account_line(&complete, final_y);
    account_key(&mut app, KeyCode::End);
    let end = paint_accounts_viewport(&mut app, width, 6);
    assert_eq!(
        account_line(&end, end.area.bottom() - 1),
        final_line,
        "End reveals the final fact even when its heading is above the viewport"
    );
    assert_eq!(
        app.selections
            .surface(&crate::interaction::SelectionSurface::Summary)
            .expect("summary selection")
            .selection(),
        crate::interaction::TileSelection::Row(crate::interaction::RowIdentity::SummaryAccount {
            account: "account-5".to_owned(),
        })
    );
}

fn assert_account_band(buffer: &Buffer, lines: Range<u16>) {
    let ground = tui_pane::selection_style(PaneFocusState::Active)
        .bg
        .expect("active selection ground");
    for y in buffer.area.y..buffer.area.bottom() {
        for x in buffer.area.x..buffer.area.right() {
            assert_eq!(
                buffer[(x, y)].bg == ground,
                lines.contains(&y),
                "selection band at ({x}, {y}); selected lines {lines:?}"
            );
        }
    }
}

fn assert_middle_account_band(width: u16) -> u16 {
    let mut app = interactive_accounts();
    let before = paint_accounts_viewport(&mut app, width, 64);
    let selected = locate(&before, "account-2");
    let next = locate(&before, "account-3");
    assert!(selected.row > before.area.y, "preceding row is visible");
    assert!(next.row < before.area.bottom(), "following row is visible");
    app.click(Position::new(selected.column, selected.row));
    let after = paint_accounts_viewport(&mut app, width, 64);
    assert_account_selected(&app, &after, "account-2");
    assert_account_band(&after, selected.row..next.row);
    next.row - selected.row
}

#[test]
fn two_line_account_selection_bands_cover_both_lines_and_stop_at_neighbors() {
    assert_eq!(assert_middle_account_band(TWO_LINE_ACCOUNTS_WIDTH), 2);
}

#[test]
fn stacked_account_selection_bands_cover_every_fact_and_stop_at_neighbors() {
    assert!(assert_middle_account_band(STACKED_ACCOUNTS_WIDTH) > 2);
}

#[test]
fn full_account_selection_bands_cover_exactly_one_line() {
    let app = interactive_accounts();
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines);
    let width = summary::width(
        &machines,
        &placement,
        NOW,
        AccountColumn::Hidden,
        SummarySort::default(),
        &app.accounts.table,
        AccountsSort::SourceOrder,
    ) + 2;
    assert_eq!(assert_middle_account_band(width), 1);
}

#[test]
fn clipped_stacked_account_selection_bands_cover_facts_without_the_heading() {
    let mut app = interactive_accounts();
    let complete = paint_accounts_viewport(&mut app, STACKED_ACCOUNTS_WIDTH, 64);
    let heading = locate(&complete, "account-5");
    let last = (heading.row..complete.area.bottom())
        .rev()
        .find(|&y| !account_line(&complete, y).trim().is_empty())
        .expect("last account fact");
    let facts: Vec<_> = (heading.row + 1..=last)
        .map(|y| account_line(&complete, y))
        .collect();
    let first = locate(&complete, "account-0");
    app.click(Position::new(first.column, first.row));
    let short = paint_accounts_viewport(&mut app, STACKED_ACCOUNTS_WIDTH, 6);
    assert!(facts.len() >= usize::from(short.area.height));
    account_key(&mut app, KeyCode::End);
    let end = paint_accounts_viewport(&mut app, STACKED_ACCOUNTS_WIDTH, 6);
    for y in end.area.y..end.area.bottom() {
        let line = account_line(&end, y);
        assert!(facts.contains(&line), "visible account fact: {line:?}");
        assert!(!line.contains("account-5"), "heading is scrolled off");
    }
    assert_account_band(&end, end.area.y..end.area.bottom());
}

fn first_account_glyph(buffer: &Buffer, y: u16) -> u16 {
    (buffer.area.x..buffer.area.right())
        .find(|&x| !buffer[(x, y)].symbol().trim().is_empty())
        .expect("nonblank drawn line")
}

#[test]
fn two_line_account_continuations_and_headers_align_with_account_names() {
    let mut app = interactive_accounts();
    let buffer = paint_accounts_viewport(&mut app, TWO_LINE_ACCOUNTS_WIDTH, 64);
    // Spec 1 drops credits and read age before choosing TwoLine.
    for y in buffer.area.y..buffer.area.bottom() {
        let line = account_line(&buffer, y);
        for dropped in ["13,446", "2m"] {
            assert!(!line.contains(dropped), "dropped {dropped}: {line}");
        }
    }
    let header = locate(&buffer, "account  ");
    let resets = locate(&buffer, "resets");
    assert_eq!(resets.row, header.row + 1, "two header lines");
    assert_eq!(resets.column, header.column, "resets starts under account");
    assert_eq!(first_account_glyph(&buffer, resets.row), header.column);
    for index in 0..6 {
        let name = locate(&buffer, &format!("account-{index}"));
        let continuation = name.row + 1;
        assert!(
            account_line(&buffer, continuation).contains("Sat 10:00"),
            "the second line contains the kept reset fact"
        );
        assert_eq!(
            first_account_glyph(&buffer, continuation),
            name.column,
            "account-{index} continuation starts under its name"
        );
    }
}
