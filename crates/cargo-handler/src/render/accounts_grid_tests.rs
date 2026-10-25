//! Account demand and independent header sorts reach the actual grid.

#![expect(
    clippy::expect_used,
    reason = "grid fixture rows and header cells must exist"
)]
#![expect(clippy::panic, reason = "missing rendered headers fail the test")]

use tui_pane::AttractWork;
use tui_pane::CellPoint;
use tui_pane::TileView;
use tui_pane::Updates;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::scan::ScanStage;
use crate::sort::SortDirection;
use crate::sort::SummaryColumn;

const NOW: u64 = 1_792_166_400;

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
    panic!("missing grid label {label}");
}

fn draw_grid(app: &mut App) -> Buffer {
    // Header sorting is exercised in the summary, with whole session names.
    app.tiles.set_view(TileView::Summary);
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
        // A header click can add a sort arrow; the terminal keeps its width.
        80,
        demand
            .summary
            .required()
            .get()
            .checked_add(3)
            .expect("small summary"),
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
    buffer
}

#[test]
fn accounts_grid_measured_tables_sort_account_headers_without_changing_agent_order() {
    let mut app = App::new_for_test().expect("test app");
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
    let account_header = locate(&before, "weekly left");
    let original_sort = app.summary_sort;
    cell_clicked(
        &mut app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    Rect::new(1, 1, before.area.width - 2, before.area.height - 2),
            point:   CellPoint {
                column: account_header.column - 1,
                row:    account_header.row - 1,
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
    let account_header = locate(&after, "weekly left ▲");
    cell_clicked(
        &mut app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    Rect::new(1, 1, after.area.width - 2, after.area.height - 2),
            point:   CellPoint {
                column: account_header.column - 1,
                row:    account_header.row - 1,
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
            area:    Rect::new(1, 1, reversed.area.width - 2, reversed.area.height - 2),
            point:   CellPoint {
                column: header.column - 1,
                row:    header.row - 1,
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
