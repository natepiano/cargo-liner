//! Reset colors follow distinct timestamp ranks in the rendered summary.

#![expect(
    clippy::expect_used,
    reason = "fixture times and rendered cells must exist"
)]
#![expect(clippy::panic, reason = "a missing rendered label fails the fixture")]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::CellPoint;
use tui_pane::Theme;

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
use crate::footer_colors::tests;
use crate::sort::AccountsColumn;
use crate::theme;
use crate::theme::SessionRowInk;

const NOW: u64 = 1_792_166_400;
const SUMMARY_AREA: Rect = Rect::new(4, 3, 160, 32);

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

pub(super) fn drawn(app: &App) -> Buffer { drawn_in(app, SUMMARY_AREA) }

fn drawn_in(app: &App, area: Rect) -> Buffer {
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
    let mut buffer = Buffer::empty(area);
    buffer.set_style(
        area,
        ratatui::style::Style::default().bg(tui_pane::pane_background(false)),
    );
    super::draw(&mut buffer, area, &frame);
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

/// Install handler reset roles for tests that otherwise inherit `tui_pane`'s empty role map.
struct ResetThemeFixture {
    previous: Arc<Theme>,
}

impl ResetThemeFixture {
    fn install() -> Self {
        let variant = theme::builtins()
            .into_iter()
            .find(|variant| variant.id.as_str() == "Default Dark")
            .expect("default reset theme");
        Self::with_theme(variant.theme)
    }

    fn with_theme(theme: Theme) -> Self {
        let previous = tui_pane::theme();
        tui_pane::set_active_theme(Arc::new(theme));
        Self { previous }
    }
}

impl Drop for ResetThemeFixture {
    fn drop(&mut self) { tui_pane::set_active_theme(Arc::clone(&self.previous)); }
}

fn reset_role(key: &str) -> Color {
    tui_pane::theme()
        .roles
        .get(key)
        .expect("reset theme role")
        .color
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
fn accounts_gradient_distinct_resets_draw_the_reset_palette_on_each_theme() {
    let previous = tui_pane::theme();
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        let nearest = reset_role("reset_nearest");
        let farthest = reset_role("reset_farthest");
        let expected = [
            nearest,
            tui_pane::blend_color(nearest, farthest, 128),
            farthest,
        ]
        .map(readable_rank);
        let app = app_with_accounts(vec![
            account("furthest", reset(7200)),
            account("nearest", reset(60)),
            account("middle", reset(120)),
        ]);
        let buffer = drawn(&app);
        let middle = reset_color(&buffer, "middle");
        assert_ne!(
            middle, expected[0],
            "middle differs from nearest: {expected:?}"
        );
        assert_ne!(
            middle, expected[2],
            "middle differs from farthest: {expected:?}"
        );
        let [first, middle_rgb, last] = [expected[0], middle, expected[2]].map(tests::terminal_rgb);
        for channel in 0..3 {
            assert!(
                (first[channel].min(last[channel])..=first[channel].max(last[channel]))
                    .contains(&middle_rgb[channel]),
                "middle lies between endpoints: {first:?} / {middle_rgb:?} / {last:?}"
            );
        }
        for (label, color) in ["nearest", "middle", "furthest"].into_iter().zip(expected) {
            assert_eq!(
                reset_color(&buffer, label),
                color,
                "{label} uses chronological rank"
            );
        }
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn reset_time_order_assigns_the_nearest_shade_to_the_earliest_instant() {
    let times = [-3600, 60, 120, 3600, 7200, 86400];
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        let _theme = ResetThemeFixture::with_theme(variant.theme);
        let nearest = reset_role("reset_nearest");
        let farthest = reset_role("reset_farthest");
        let expected: Vec<_> = (0..times.len())
            .map(|rank| {
                let alpha = u8::try_from(rank * usize::from(u8::MAX) / (times.len() - 1))
                    .expect("evenly spaced reset shade");
                readable_rank(tui_pane::blend_color(nearest, farthest, alpha))
            })
            .collect();
        for order in [[5, 1, 3, 0, 4, 2], [2, 4, 0, 3, 1, 5]] {
            let app = app_with_accounts(
                order
                    .into_iter()
                    .map(|rank| account(&format!("time-rank-{rank}"), reset(times[rank])))
                    .collect(),
            );
            let buffer = drawn(&app);
            for (rank, color) in expected.iter().enumerate() {
                assert_eq!(
                    reset_color(&buffer, &format!("time-rank-{rank}")),
                    *color,
                    "{name}: chronological rank {rank}, source order {order:?}, including a past instant"
                );
            }
        }
    }
}

#[test]
fn accounts_gradient_equal_timestamps_share_a_rank_without_changing_the_midpoint() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(vec![
        account("nearest", reset(60)),
        account("middle-one", reset(120)),
        account("furthest", reset(7200)),
        account("middle-two", reset(120)),
    ]);
    let buffer = drawn(&app);
    let midpoint = reset_color(&buffer, "middle-one");
    assert_eq!(midpoint, reset_color(&buffer, "middle-two"));
    assert_eq!(
        midpoint,
        readable_rank(tui_pane::blend_color(
            reset_role("reset_nearest"),
            reset_role("reset_farthest"),
            128
        ))
    );
}

#[test]
fn accounts_gradient_one_account_uses_the_nearest_reset_shade() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(vec![account("only", reset(60))]);
    assert_eq!(
        reset_color(&drawn(&app), "only"),
        readable_rank(reset_role("reset_nearest"))
    );
}

#[test]
fn accounts_gradient_one_shared_timestamp_uses_the_nearest_reset_shade_for_every_account() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(
        ["first", "second", "third"]
            .map(|label| account(label, reset(60)))
            .into(),
    );
    let buffer = drawn(&app);
    for label in ["first", "second", "third"] {
        assert_eq!(
            reset_color(&buffer, label),
            readable_rank(reset_role("reset_nearest"))
        );
    }
}

#[test]
fn accounts_gradient_missing_reset_keeps_plain_text_and_does_not_shift_other_ranks() {
    let _theme = ResetThemeFixture::install();
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
        readable_rank(reset_role("reset_nearest"))
    );
    assert_eq!(
        reset_color(&buffer, "furthest"),
        readable_rank(reset_role("reset_farthest"))
    );
}

fn draw_reset_ranks() -> Buffer {
    let accounts = (0..=u8::MAX)
        .map(|rank| {
            let mut row = account(&format!("rank-{rank:03}"), reset(i64::from(rank) * 60 + 60));
            row.weekly_left_percent = Reading::Known(if rank % 2 == 0 { 9.0 } else { 24.0 });
            row.read_status = AccountReadStatus::Failed;
            row
        })
        .collect();
    let app = app_with_accounts(accounts);
    let area = Rect::new(0, 0, 160, u16::from(u8::MAX) + 10);
    drawn_in(&app, area)
}

fn reset_rank_positions(buffer: &Buffer) -> HashMap<String, CellPoint> {
    let area = buffer.area;
    (area.y..area.bottom())
        .filter_map(|row| {
            let line: String = (area.x..area.right())
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            let start = line.find("rank-")?;
            Some((
                line[start..start + "rank-000".len()].to_owned(),
                CellPoint {
                    column: area.x
                        + u16::try_from(line[..start].chars().count()).expect("rank column"),
                    row,
                },
            ))
        })
        .collect()
}

fn configured_account_inks(fixture: &str) -> Vec<(&str, Color)> {
    let colors = footer_colors::FooterColors::parse(fixture);
    fixture
        .lines()
        .filter_map(|line| {
            let (key, _) = line.split_once('=')?;
            let label = key.strip_prefix("account.")?;
            Some((
                label,
                colors
                    .account(label)
                    .style(ratatui::style::Style::default())
                    .fg
                    .expect("account ink"),
            ))
        })
        .collect()
}

fn check_rendered_reset_ramp(variant_name: &str) {
    let previous = tui_pane::theme();
    let fixture = include_str!("../footer_colors/fixtures/statusline.conf");
    let mut failures = Vec::new();
    let mut reports = Vec::new();
    for variant in theme::builtins()
        .into_iter()
        .filter(|variant| variant.id.as_str() == variant_name)
    {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        let buffer = draw_reset_ranks();
        let account_inks = configured_account_inks(fixture);
        let left_right = locate(&buffer, "left").column + 4;
        let reset_right = locate(&buffer, "resets").column + 6;
        let positions = reset_rank_positions(&buffer);
        assert_eq!(positions.len(), 256, "the complete quantized ramp draws");
        let mut error_pairs = Vec::new();
        let mut warning_pairs = Vec::new();
        let mut alarm_pairs = Vec::new();
        let mut account_pairs = Vec::new();
        let alarm = buffer[(left_right - 1, positions["rank-000"].row)].fg;
        let warning = buffer[(left_right - 1, positions["rank-001"].row)].fg;
        for rank in 0..=u8::MAX {
            let label = format!("rank-{rank:03}");
            let at = positions[&label];
            let dot = (0..at.column)
                .find(|x| buffer[(*x, at.row)].symbol() == "●")
                .expect("reading dot");
            assert_eq!(
                buffer[(dot, at.row)].fg,
                tui_pane::error_color(),
                "failed reading stays error red"
            );
            let ink = buffer[(reset_right - 1, at.row)].fg;
            let reset_cells: Vec<_> = (left_right + 2..reset_right)
                .map(|x| &buffer[(x, at.row)])
                .filter(|cell| !cell.symbol().trim().is_empty())
                .collect();
            assert!(reset_cells.len() >= 5, "whole reset timestamp draws");
            for cell in reset_cells {
                assert_eq!(cell.fg, ink, "whole reset uses the rank ink");
                let contrast = tests::covered_role_contrast(cell.fg, cell.bg);
                if contrast < 4.5 {
                    failures.push(format!("{name} {label} {cell:?} covered={contrast:.6}"));
                }
            }
            for (other_name, other, pairs) in [
                ("error", tui_pane::error_color(), &mut error_pairs),
                ("left warning", warning, &mut warning_pairs),
                ("left alarm", alarm, &mut alarm_pairs),
            ] {
                let distance = tests::perceptual_distance(ink, other);
                if distance < 20.0 {
                    failures.push(format!(
                        "{name} {label} {ink:?}/{other_name} {other:?} dE={distance:.6}"
                    ));
                }
                pairs.push((label.clone(), ink, other_name.to_owned(), other));
            }
            for (account_name, other) in &account_inks {
                let gap = tests::hue_gap(ink, *other);
                if gap < 30.0 {
                    failures.push(format!(
                        "{name} {label} {ink:?}/{account_name} {other:?} hue={gap:.6}"
                    ));
                }
                account_pairs.push((label.clone(), ink, (*account_name).to_owned(), *other));
            }
        }
        let mut report = format!(
            "{name} drawn left warning={warning:?} alarm={alarm:?} error={:?}\n",
            tui_pane::error_color()
        );
        for (class, pairs) in [
            ("reset/error", error_pairs),
            ("reset/left-warning", warning_pairs),
            ("reset/left-alarm", alarm_pairs),
            ("reset/account", account_pairs),
        ] {
            report.push_str(&tests::palette_pair_report(class, &pairs));
        }
        reports.push(report);
    }
    tui_pane::set_active_theme(previous);
    assert!(
        failures.is_empty(),
        "reset semantic separation: {failures:#?}; measurements: {reports:#?}"
    );
}

#[test]
fn reset_shades_avoid_alarms_and_account_hues_with_failed_dots_red_in_default_dark() {
    check_rendered_reset_ramp("Default Dark");
}

#[test]
fn reset_shades_avoid_alarms_and_account_hues_with_failed_dots_red_in_default_light() {
    check_rendered_reset_ramp("Default Light");
}

#[test]
fn reset_shades_avoid_alarms_and_account_hues_with_failed_dots_red_in_high_contrast_dark() {
    check_rendered_reset_ramp("High Contrast Dark");
}

#[test]
fn reset_shades_avoid_alarms_and_account_hues_with_failed_dots_red_in_high_contrast_light() {
    check_rendered_reset_ramp("High Contrast Light");
}

#[test]
fn reset_endpoints_remain_perceptually_distinct_on_every_theme() {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut failures = Vec::new();
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let app = app_with_accounts(vec![
                account("nearest", reset(60)),
                account("furthest", reset(7200)),
            ]);
            let buffer = drawn(&app);
            let nearest = reset_color(&buffer, "nearest");
            let farthest = reset_color(&buffer, "furthest");
            let distance = tests::perceptual_distance(nearest, farthest);
            if distance < 12.0 {
                failures.push(format!("{name} tint={tint} nearest={nearest:?} farthest={farthest:?} dE={distance:.9} floor=12"));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "reset endpoint separation: {failures:#?}"
    );
}

#[test]
fn reset_shades_keep_saturated_chroma_and_separation_from_plain_text() {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut failures = Vec::new();
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let text = SessionRowInk::Text.style().fg.expect("plain text ink");
            let buffer = draw_reset_ranks();
            let reset_column = locate(&buffer, "resets").column + 5;
            let positions = reset_rank_positions(&buffer);
            assert_eq!(positions.len(), 256, "{name} tint={tint}: every rank draws");
            for rank in 0..=u8::MAX {
                let label = format!("rank-{rank:03}");
                let row = positions[&label].row;
                let cell = &buffer[(reset_column, row)];
                assert!(
                    !cell.symbol().trim().is_empty(),
                    "{name} {label}: reset glyph"
                );
                let ink = cell.fg;
                let chroma = footer_colors::Oklch::from_color(ink).chroma;
                let distance = tests::perceptual_distance(ink, text);
                if chroma < 0.10 || distance < 15.0 {
                    failures.push(format!(
                        "{name} tint={tint} {label} reset={ink:?} text={text:?} C={chroma:.9} floor=0.10 dE={distance:.9} floor=15"
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "reset visibility: {failures:#?}");
}
