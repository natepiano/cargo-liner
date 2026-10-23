//! Reset colors follow distinct timestamp ranks in the rendered summary.

#![expect(
    clippy::expect_used,
    reason = "fixture times and rendered cells must exist"
)]
#![expect(clippy::panic, reason = "a missing rendered label fails the fixture")]

use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::CellPoint;
use tui_pane::blend_color;

use super::SummaryFrame;
use crate::agent_cell;
use crate::app::App;
use crate::census::AgentPlacement;
use crate::census::accounts::AccountReadStatus;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditActivity;
use crate::census::accounts::CreditPermission;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::footer_colors;
use crate::sort::AccountsColumn;
use crate::theme;

const NOW: u64 = 1_792_166_400;
const SUMMARY_AREA: Rect = Rect::new(4, 3, 160, 32);
const MIDPOINT_BELOW: u8 = u8::MAX / 2;
const MIDPOINT_ABOVE: u8 = MIDPOINT_BELOW + 1;

pub(super) fn reset(seconds: i64) -> Reading<DateTime<Utc>> {
    Reading::Known(
        DateTime::from_timestamp(i64::try_from(NOW).expect("fixture second") + seconds, 0)
            .expect("fixture reset"),
    )
}

pub(super) fn account(label: &str, resets_at: Reading<DateTime<Utc>>) -> AccountUsage {
    AccountUsage {
        label: label.to_owned(),
        roles: Vec::new(),
        weekly_left_percent: Reading::Known(50.0),
        resets_at,
        credits: Credits::Amount(20.0),
        credit_permission: CreditPermission::Enabled,
        credit_activity: CreditActivity::Idle,
        read_status: AccountReadStatus::Ok,
        read_at: reset(0),
    }
}

pub(super) fn app_with_accounts(accounts: Vec<AccountUsage>) -> App {
    let mut app = App::new_for_test().expect("test app");
    app.accounts.table = AccountsTable::Ready(AccountsReading {
        accounts,
        problems: Vec::new(),
    });
    app
}

pub(super) fn drawn(app: &App) -> Buffer {
    let machines = app.census.machines(&[]);
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let frame = SummaryFrame {
        accounts:       &app.accounts.table,
        accounts_sort:  app.accounts_sort,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: agent_cell::account_column(&cells),
        footer_colors:  app.footer_colors.colors(),
        sort:           app.summary_sort,
    };
    let mut buffer = Buffer::empty(SUMMARY_AREA);
    super::draw(&mut buffer, SUMMARY_AREA, &frame);
    buffer
}

pub(super) fn locate(buffer: &Buffer, label: &str) -> CellPoint {
    for row in buffer.area.y..buffer.area.bottom() {
        let line: String = (buffer.area.x..buffer.area.right())
            .map(|column| buffer[(column, row)].symbol())
            .collect();
        if let Some(index) = line.find(label) {
            return CellPoint {
                column: buffer.area.x
                    + u16::try_from(line[..index].chars().count()).expect("column"),
                row,
            };
        }
    }
    panic!("missing rendered label {label}: {buffer:?}");
}

fn reset_color(buffer: &Buffer, label: &str) -> Color {
    let right = locate(buffer, "resets").column + 6;
    let left = locate(buffer, "left").column + 4 + 2;
    let row = locate(buffer, label).row;
    let color = buffer[(right - 1, row)].fg;
    let mut glyphs = 0;
    for column in left..right {
        if !buffer[(column, row)].symbol().trim().is_empty() {
            assert_eq!(buffer[(column, row)].fg, color, "{label}");
            glyphs += 1;
        }
    }
    assert!(glyphs >= 5, "a whole reset time draws");
    color
}

fn readable_rank(color: Color) -> Color {
    footer_colors::readable_status_style(ratatui::style::Style::default().fg(color))
        .fg
        .expect("readable reset ink")
}

#[test]
fn accounts_gradient_fixture_readings_draw_fresh_at_the_fixture_time() {
    let app = app_with_accounts(vec![account("fresh-account", reset(60))]);
    let buffer = drawn(&app);
    let label = locate(&buffer, "fresh-account");
    let roles_column = locate(&buffer, AccountsColumn::Roles.header()).column;
    let account_text: String = (label.column..roles_column)
        .map(|column| buffer[(column, label.row)].symbol())
        .collect();
    assert_eq!(account_text.trim(), "fresh-account");
}

#[test]
fn accounts_gradient_distinct_resets_draw_success_midpoint_and_error_on_each_theme() {
    let previous = tui_pane::theme();
    for variant in theme::builtins() {
        let source_success = variant.theme.semantic.success.color;
        let source_error = variant.theme.semantic.error.color;
        tui_pane::set_active_theme(Arc::new(variant.theme));
        let success = readable_rank(source_success);
        let error = readable_rank(source_error);
        let app = app_with_accounts(vec![
            account("furthest", reset(7200)),
            account("nearest", reset(60)),
            account("middle", reset(120)),
        ]);
        let buffer = drawn(&app);
        assert_eq!(reset_color(&buffer, "nearest"), success);
        assert_eq!(reset_color(&buffer, "furthest"), error);
        let midpoint = reset_color(&buffer, "middle");
        assert!(
            [MIDPOINT_BELOW, MIDPOINT_ABOVE]
                .map(|alpha| readable_rank(blend_color(source_success, source_error, alpha)))
                .contains(&midpoint),
            "the midpoint uses rank rather than elapsed time: {midpoint:?}"
        );
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn accounts_gradient_equal_timestamps_share_a_rank_without_changing_the_midpoint() {
    let app = app_with_accounts(vec![
        account("nearest", reset(60)),
        account("middle-one", reset(120)),
        account("furthest", reset(7200)),
        account("middle-two", reset(120)),
    ]);
    let buffer = drawn(&app);
    let active = tui_pane::theme();
    let midpoint = reset_color(&buffer, "middle-one");
    assert_eq!(midpoint, reset_color(&buffer, "middle-two"));
    assert!(
        [MIDPOINT_BELOW, MIDPOINT_ABOVE]
            .map(|alpha| readable_rank(blend_color(
                active.semantic.success.color,
                active.semantic.error.color,
                alpha
            )))
            .contains(&midpoint)
    );
}

#[test]
fn accounts_gradient_one_account_uses_success() {
    let app = app_with_accounts(vec![account("only", reset(60))]);
    assert_eq!(
        reset_color(&drawn(&app), "only"),
        readable_rank(tui_pane::theme().semantic.success.color)
    );
}

#[test]
fn accounts_gradient_one_shared_timestamp_uses_success_for_every_account() {
    let app = app_with_accounts(
        ["first", "second", "third"]
            .map(|label| account(label, reset(60)))
            .into(),
    );
    let buffer = drawn(&app);
    for label in ["first", "second", "third"] {
        assert_eq!(
            reset_color(&buffer, label),
            readable_rank(tui_pane::theme().semantic.success.color)
        );
    }
}

#[test]
fn accounts_gradient_missing_reset_keeps_plain_text_and_does_not_shift_other_ranks() {
    let app = app_with_accounts(vec![
        account("missing", Reading::Unavailable),
        account("nearest", reset(60)),
        account("furthest", reset(120)),
    ]);
    let buffer = drawn(&app);
    let column = locate(&buffer, "resets").column + 5;
    let row = locate(&buffer, "missing").row;
    assert_eq!(buffer[(column, row)].symbol(), " ");
    assert_eq!(
        reset_color(&buffer, "nearest"),
        readable_rank(tui_pane::theme().semantic.success.color)
    );
    assert_eq!(
        reset_color(&buffer, "furthest"),
        readable_rank(tui_pane::theme().semantic.error.color)
    );
}
