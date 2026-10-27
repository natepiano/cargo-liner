//! Account header presses reorder the rendered account table independently.

#![expect(
    clippy::expect_used,
    reason = "fixture readings and rendered headers must exist"
)]

use ratatui::buffer::Buffer;
use tui_pane::CellClick;
use tui_pane::CellPoint;
use tui_pane::TileContent;

use super::accounts_gradient_tests;
use crate::app::App;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::census::scan::ScanStage;
use crate::render;
use crate::sort::AccountsColumn;

const DAY_SECONDS: i64 = 86_400;
const SOURCE_ORDER: [&str; 4] = ["gamma", "alpha", "beta", "delta"];

fn app_with_accounts(mut accounts: Vec<AccountUsage>) -> App {
    // Unread fixtures keep header widths equal at fixture time and click time.
    for account in &mut accounts {
        account.read_at = Reading::Unavailable;
    }
    accounts_gradient_tests::app_with_accounts(accounts)
}

fn accounts() -> Vec<AccountUsage> {
    [
        ("gamma", AccountRole::Workers, 10.0, 3),
        ("alpha", AccountRole::Units, 2.0, 1),
        ("beta", AccountRole::Showrunners, 10.0, 2),
        ("delta", AccountRole::Spare, 100.0, 5),
    ]
    .map(|(label, role, amount, days)| AccountUsage {
        roles: vec![role],
        weekly_left_percent: Reading::Known(amount),
        credits: Credits::Amount(amount),
        ..accounts_gradient_tests::account(
            label,
            accounts_gradient_tests::reset(days * DAY_SECONDS),
        )
    })
    .into()
}

fn with_agents(mut app: App) -> App {
    app.census.apply(CensusUpdate::Local {
        rows:      [(10, "session-first", 100), (20, "session-second", 200)]
            .map(|(pid, name, started)| {
                serde_json::from_value(serde_json::json!({
                    "agent":"claude","name":name,"pid":pid,"started":started,
                    "directory":"/fixture","children":[{"depth":0,"kind":"shell","pid":9999,"name":"fixture command","started":100}]
                }))
                .expect("agent fixture")
            })
            .into(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    app
}

fn header_point(buffer: &Buffer, column: AccountsColumn) -> CellPoint {
    let row = accounts_gradient_tests::locate(buffer, "left").row;
    let line: String = (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, row)].symbol())
        .collect();
    let offset = line.find(column.header()).expect("drawn account header");
    CellPoint {
        column: u16::try_from(line[..offset].chars().count()).expect("header column"),
        row:    row - buffer.area.y,
    }
}

fn click_header(app: &mut App, buffer: &Buffer, column: AccountsColumn) {
    render::cell_clicked(
        app,
        CellClick {
            index:   0,
            content: TileContent::Summary,
            area:    buffer.area,
            point:   header_point(buffer, column),
        },
    );
}

fn assert_order(buffer: &Buffer, labels: &[&str]) {
    let rows: Vec<_> = labels
        .iter()
        .map(|label| accounts_gradient_tests::locate(buffer, label).row)
        .collect();
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "expected {labels:?}, rows {rows:?}"
    );
}

#[test]
fn accounts_header_sort_each_column_orders_and_reverses_only_accounts() {
    let cases = [
        (
            AccountsColumn::Account,
            ["alpha", "beta", "delta", "gamma"],
            ["gamma", "delta", "beta", "alpha"],
        ),
        (
            AccountsColumn::Roles,
            ["beta", "delta", "alpha", "gamma"],
            ["gamma", "alpha", "delta", "beta"],
        ),
        (
            AccountsColumn::WeeklyLeft,
            ["alpha", "gamma", "beta", "delta"],
            ["delta", "gamma", "beta", "alpha"],
        ),
        (
            AccountsColumn::Resets,
            ["alpha", "beta", "gamma", "delta"],
            ["delta", "gamma", "beta", "alpha"],
        ),
        (
            AccountsColumn::Credits,
            ["alpha", "gamma", "beta", "delta"],
            ["delta", "gamma", "beta", "alpha"],
        ),
    ];
    for (column, ascending, descending) in cases {
        let mut app = with_agents(app_with_accounts(accounts()));
        let original_sort = app.summary_sort;
        let before = accounts_gradient_tests::drawn(&app);
        assert_order(&before, &SOURCE_ORDER);
        let agents_before = ["session-first", "session-second"]
            .map(|label| accounts_gradient_tests::locate(&before, label).row);
        click_header(&mut app, &before, column);
        let first = accounts_gradient_tests::drawn(&app);
        assert_order(&first, &ascending);
        assert_eq!(app.summary_sort, original_sort);
        assert_eq!(
            ["session-first", "session-second"]
                .map(|label| accounts_gradient_tests::locate(&first, label).row),
            agents_before,
        );
        click_header(&mut app, &first, column);
        let second = accounts_gradient_tests::drawn(&app);
        assert_order(&second, &descending);
        assert_eq!(app.summary_sort, original_sort);
        let agent_header = accounts_gradient_tests::locate(&second, "session");
        render::cell_clicked(
            &mut app,
            CellClick {
                index:   0,
                content: TileContent::Summary,
                area:    second.area,
                point:   CellPoint {
                    column: agent_header.column - second.area.x,
                    row:    agent_header.row - second.area.y,
                },
            },
        );
        assert_ne!(app.summary_sort, original_sort);
        assert_order(&accounts_gradient_tests::drawn(&app), &descending);
    }
}

#[test]
fn accounts_header_sort_signed_zero_values_keep_source_order() {
    for column in [AccountsColumn::WeeklyLeft, AccountsColumn::Credits] {
        let rows = [("A", 0.0), ("B", -0.0)]
            .map(|(label, amount)| AccountUsage {
                weekly_left_percent: Reading::Known(amount),
                credits: Credits::Amount(amount),
                ..accounts_gradient_tests::account(label, accounts_gradient_tests::reset(60))
            })
            .into();
        let mut app = app_with_accounts(rows);
        let before = accounts_gradient_tests::drawn(&app);
        click_header(&mut app, &before, column);
        assert_order(&accounts_gradient_tests::drawn(&app), &["A", "B"]);
    }
}

#[test]
fn accounts_header_sort_unknown_values_stay_after_numeric_and_chronological_values() {
    for column in [
        AccountsColumn::WeeklyLeft,
        AccountsColumn::Credits,
        AccountsColumn::Resets,
    ] {
        let mut rows = accounts();
        let mut unlimited =
            accounts_gradient_tests::account("unlimited-account", Reading::Unavailable);
        unlimited.weekly_left_percent = Reading::Unavailable;
        unlimited.credits = Credits::Unlimited;
        let mut unavailable =
            accounts_gradient_tests::account("unavailable-account", Reading::Unavailable);
        unavailable.weekly_left_percent = Reading::Unavailable;
        unavailable.credits = Credits::Unavailable;
        rows.insert(0, unlimited);
        rows.insert(2, unavailable);
        let mut app = app_with_accounts(rows);
        for _ in 0..2 {
            let before = accounts_gradient_tests::drawn(&app);
            click_header(&mut app, &before, column);
            let after = accounts_gradient_tests::drawn(&app);
            let first_unknown = ["unlimited-account", "unavailable-account"]
                .map(|label| accounts_gradient_tests::locate(&after, label).row)
                .into_iter()
                .min()
                .expect("unknown rows");
            for label in SOURCE_ORDER {
                assert!(
                    accounts_gradient_tests::locate(&after, label).row < first_unknown,
                    "{column:?}: {label}"
                );
            }
        }
    }
}

#[test]
fn accounts_header_sort_data_rows_and_column_gaps_keep_source_order() {
    let mut app = app_with_accounts(accounts());
    let before = accounts_gradient_tests::drawn(&app);
    let header = header_point(&before, AccountsColumn::Credits);
    for point in [
        CellPoint {
            row: header.row + 1,
            ..header
        },
        CellPoint {
            column: header_point(&before, AccountsColumn::Roles).column - 1,
            row:    header.row,
        },
    ] {
        render::cell_clicked(
            &mut app,
            CellClick {
                index: 0,
                content: TileContent::Summary,
                area: before.area,
                point,
            },
        );
        assert_order(&accounts_gradient_tests::drawn(&app), &SOURCE_ORDER);
    }
    click_header(&mut app, &before, AccountsColumn::Credits);
    assert_order(
        &accounts_gradient_tests::drawn(&app),
        &["alpha", "gamma", "beta", "delta"],
    );
}

#[test]
fn numeric_account_cells_align_their_right_edges_with_their_headers() {
    let app = app_with_accounts(accounts());
    let buffer = accounts_gradient_tests::drawn(&app);
    for (column, previous_column, values) in [
        (
            AccountsColumn::WeeklyLeft,
            AccountsColumn::Roles,
            ["10%", "2%", "10%", "100%"],
        ),
        (
            AccountsColumn::Credits,
            AccountsColumn::Resets,
            ["10", "2", "10", "100"],
        ),
    ] {
        let header = header_point(&buffer, column);
        let right = buffer.area.x + header.column + super::cell_width(column.header());
        let left = buffer.area.x + header_point(&buffer, previous_column).column;
        for (label, value) in SOURCE_ORDER.into_iter().zip(values) {
            let y = accounts_gradient_tests::locate(&buffer, label).row;
            let line: String = (left..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let index = if column == AccountsColumn::Credits {
                line.rfind(value)
            } else {
                line.find(value)
            }
            .expect("numeric account value");
            let value_right = left + super::cell_width(&line[..index]) + super::cell_width(value);
            assert_eq!(value_right, right, "{label} {column:?}: {line}");
        }
    }
}

#[test]
fn new_account_headers_toggle_and_keep_blank_read_and_credit_states_last() {
    let table=crate::census::accounts::AccountsReading::parse(&serde_json::to_vec(&serde_json::json!({"accounts":[
        {"label":"older","roles":[],"weekly_left_percent":50,"credits":null,"use_credits":true,"credits_in_use":false,"read_ok":true,"read_at":"2026-10-16T00:00:00Z","resets_at":null},
        {"label":"newer","roles":[],"weekly_left_percent":50,"credits":null,"use_credits":false,"credits_in_use":true,"read_ok":true,"read_at":"2026-10-17T00:00:00Z","resets_at":null},
        {"label":"blank","roles":[],"weekly_left_percent":null,"credits":null,"use_credits":false,"credits_in_use":false,"read_at":null,"resets_at":null}
    ],"problems":[]})).expect("wire JSON")).expect("reading");
    for (header, first, second) in [
        (
            "enabled",
            ["older", "newer", "blank"],
            ["older", "newer", "blank"],
        ),
        (
            "in use",
            ["newer", "older", "blank"],
            ["newer", "older", "blank"],
        ),
        (
            "read",
            ["newer", "older", "blank"],
            ["older", "newer", "blank"],
        ),
    ] {
        let mut app = accounts_gradient_tests::app_with_accounts(table.accounts.clone());
        for (marker, order) in [("▲", first), ("▼", second)] {
            let before = accounts_gradient_tests::drawn(&app);
            let point = accounts_gradient_tests::locate(&before, header);
            render::cell_clicked(
                &mut app,
                CellClick {
                    index:   0,
                    content: TileContent::Summary,
                    area:    before.area,
                    point:   CellPoint {
                        column: point.column - before.area.x,
                        row:    point.row - before.area.y,
                    },
                },
            );
            let after = accounts_gradient_tests::drawn(&app);
            assert_order(&after, &order);
            accounts_gradient_tests::locate(&after, &format!("{header} {marker}"));
        }
    }
}
