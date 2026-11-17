//! Reset colors follow distinct timestamp ranks in the rendered summary.

#![expect(
    clippy::expect_used,
    reason = "fixture times and rendered cells must exist"
)]
#![expect(clippy::panic, reason = "a missing rendered label fails the fixture")]

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::CellPoint;

use super::PaintedSummary;
use super::SummaryFrame;
use crate::agent_cell;
use crate::app::App;
use crate::census::AgentPlacement;
use crate::census::HierarchyRole;
use crate::census::accounts::AccountReadStatus;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountRoleUsage;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditPermission;
use crate::census::accounts::Credits;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::census::accounts::Reading;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_HC_DARK_THEME;
use crate::constants::DEFAULT_HC_LIGHT_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::RESET_LATEST_ROLE;
use crate::constants::RESET_MIDDLE_ROLE;
use crate::constants::RESET_SOONEST_ROLE;
use crate::footer_colors;
use crate::footer_colors::tests;
use crate::sort::AccountsColumn;
use crate::theme;
use crate::theme::MINIMUM_RESET_STEP_DISTANCE;
use crate::theme::ResetThemeFixture;
use crate::theme::SessionRowInk;

pub(super) fn role_usage(role: AccountRole) -> AccountRoleUsage {
    AccountRoleUsage {
        role,
        counts: Reading::Unavailable,
    }
}

const NOW: u64 = 1_792_166_400;
// A reset ink keeps this CIEDE2000 distance from error red and the low-quota inks.
const MINIMUM_RESET_ALERT_DISTANCE: f64 = 20.0;
// The user chose green, blue and violet resets (2026-11-14). On light themes, no readable green
// keeps 20 from the olive low-quota warning and 15 from the Astra reviewer green.
const MINIMUM_LIGHT_RESET_WARNING_DISTANCE: f64 = 17.0;
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
        login_ends_at: Reading::Unavailable,
        credits: Credits::Amount(20.0),
        credit_permission: CreditPermission::Enabled,
        read_status: AccountReadStatus::Ok,
        read_at: reset(0),
    }
}

pub(super) fn app_with_accounts(accounts: Vec<AccountUsage>) -> App {
    let mut app = App::new_for_test().expect("test app");
    app.accounts.table = AccountsTable::Ready(AccountsReading {
        received_at: NOW,
        sessions: crate::census::accounts::SessionStates::default(),
        accounts,
        problems: Vec::new(),
    });
    app
}

pub(super) fn drawn(app: &App) -> Buffer { drawn_in(app, SUMMARY_AREA) }

fn drawn_in(app: &App, area: Rect) -> Buffer {
    let machines = app.census.machines(&[]);
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    let frame = SummaryFrame {
        accounts:       &app.accounts.table,
        accounts_sort:  app.accounts_sort,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: agent_cell::account_column(&cells),
        footer_colors:  &app.footer_colors,
        sort:           app.summary_sort,
    };
    let mut buffer = Buffer::empty(area);
    buffer.set_style(
        area,
        ratatui::style::Style::default().bg(tui_pane::pane_background(false)),
    );
    let layout = Rc::new(super::SummaryLayout::new(&frame, area.width));
    super::draw_measured(&mut buffer, area, &frame, &layout);
    *app.painted_summary.borrow_mut() = PaintedSummary::Drawn(layout);
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
    let left = right.saturating_sub(9);
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
fn chronological_resets_draw_three_steps_far_apart_in_every_theme() {
    for variant in theme::builtins() {
        let theme_name = variant.id.to_string();
        let fixture = ResetThemeFixture::with_theme(variant.theme);
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let app = app_with_accounts(
                [5, 1, 3, 0, 4, 2]
                    .map(|rank| account(&format!("step-{rank}"), reset(60 + rank * 120)))
                    .to_vec(),
            );
            let buffer = drawn(&app);
            let mut steps = Vec::new();
            for rank in 0..6 {
                let color = reset_color(&buffer, &format!("step-{rank}"));
                if steps.last() != Some(&color) {
                    assert!(!steps.contains(&color), "{theme_name}: reset steps repeat");
                    steps.push(color);
                }
            }
            assert_eq!(
                steps.len(),
                3,
                "{theme_name}, tint={tint}: drawn reset steps={steps:?}"
            );
            for (index, first) in steps.iter().enumerate() {
                for second in &steps[index + 1..] {
                    let distance = footer_colors::perceptual_distance(*first, *second);
                    assert!(
                        distance >= MINIMUM_RESET_STEP_DISTANCE,
                        "{theme_name}, tint={tint}: reset steps need dE >= {MINIMUM_RESET_STEP_DISTANCE}: {first:?}/{second:?}, dE={distance:.9}"
                    );
                }
            }
        }
        drop(fixture);
    }
}

#[test]
fn accounts_gradient_fixture_readings_draw_fresh_at_the_fixture_time() {
    let app = app_with_accounts(vec![account("fresh-account", reset(60))]);
    let buffer = drawn(&app);
    let label = locate(&buffer, "fresh-account");
    let left_column = locate(&buffer, AccountsColumn::WeeklyLeft.header()).column;
    let account_text: String = (label.column..left_column)
        .map(|column| buffer[(column, label.row)].symbol())
        .collect();
    assert_eq!(account_text.trim(), "fresh-account");
}

#[test]
fn accounts_gradient_distinct_resets_draw_the_three_reset_roles_on_each_theme() {
    let previous = tui_pane::theme();
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        let app = app_with_accounts(vec![
            account("latest", reset(7200)),
            account("soonest", reset(60)),
            account("middle", reset(120)),
        ]);
        let buffer = drawn(&app);
        for (label, role) in [
            ("soonest", RESET_SOONEST_ROLE),
            ("middle", RESET_MIDDLE_ROLE),
            ("latest", RESET_LATEST_ROLE),
        ] {
            assert_eq!(
                reset_color(&buffer, label),
                readable_rank(reset_role(role)),
                "{label} uses chronological rank"
            );
        }
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn reset_time_order_assigns_the_soonest_role_to_the_earliest_instant() {
    let times = [-3600, 60, 120, 3600, 7200, 86400];
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        let _theme = ResetThemeFixture::with_theme(variant.theme);
        let [soonest, middle, latest] =
            [RESET_SOONEST_ROLE, RESET_MIDDLE_ROLE, RESET_LATEST_ROLE].map(reset_role);
        let expected = [soonest, soonest, middle, middle, latest, latest].map(readable_rank);
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
fn accounts_gradient_equal_timestamps_share_the_middle_rank() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(vec![
        account("soonest", reset(60)),
        account("middle-one", reset(120)),
        account("latest", reset(7200)),
        account("middle-two", reset(120)),
    ]);
    let buffer = drawn(&app);
    let midpoint = reset_color(&buffer, "middle-one");
    assert_eq!(midpoint, reset_color(&buffer, "middle-two"));
    assert_eq!(midpoint, readable_rank(reset_role(RESET_MIDDLE_ROLE)));
}

#[test]
fn accounts_gradient_one_account_uses_the_soonest_reset_role() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(vec![account("only", reset(60))]);
    assert_eq!(
        reset_color(&drawn(&app), "only"),
        readable_rank(reset_role(RESET_SOONEST_ROLE))
    );
}

#[test]
fn accounts_gradient_one_shared_timestamp_uses_the_soonest_reset_role_for_every_account() {
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
            readable_rank(reset_role(RESET_SOONEST_ROLE))
        );
    }
}

#[test]
fn accounts_gradient_missing_reset_keeps_plain_text_and_does_not_shift_other_ranks() {
    let _theme = ResetThemeFixture::install();
    let app = app_with_accounts(vec![
        account("missing", Reading::Unavailable),
        account("soonest", reset(60)),
        account("latest", reset(120)),
    ]);
    let buffer = drawn(&app);
    let column = locate(&buffer, "resets").column + 5;
    let row = locate(&buffer, "missing").row;
    assert_eq!(buffer[(column, row)].symbol(), " ");
    assert_eq!(
        reset_color(&buffer, "soonest"),
        readable_rank(reset_role(RESET_SOONEST_ROLE))
    );
    assert_eq!(
        reset_color(&buffer, "latest"),
        readable_rank(reset_role(RESET_LATEST_ROLE))
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

fn check_rendered_reset_ramp(variant_name: &str, warning_floor: f64) {
    let previous = tui_pane::theme();
    let mut failures = Vec::new();
    let mut reports = Vec::new();
    let variant = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == variant_name)
        .expect("the requested built-in theme exists");
    let name = variant.id.as_str().to_owned();
    tui_pane::set_active_theme(Arc::new(variant.theme));
    let buffer = draw_reset_ranks();
    let left_right = locate(&buffer, "left").column + 4;
    let reset_right = locate(&buffer, "resets").column + 6;
    let positions = reset_rank_positions(&buffer);
    assert_eq!(positions.len(), 256, "the complete quantized ramp draws");
    let mut error_pairs = Vec::new();
    let mut warning_pairs = Vec::new();
    let mut alarm_pairs = Vec::new();
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
        for (other_name, other, floor, pairs) in [
            (
                "error",
                tui_pane::error_color(),
                MINIMUM_RESET_ALERT_DISTANCE,
                &mut error_pairs,
            ),
            ("left warning", warning, warning_floor, &mut warning_pairs),
            (
                "left alarm",
                alarm,
                MINIMUM_RESET_ALERT_DISTANCE,
                &mut alarm_pairs,
            ),
        ] {
            let distance = footer_colors::perceptual_distance(ink, other);
            if distance < floor {
                failures.push(format!(
                    "{name} {label} {ink:?}/{other_name} {other:?} dE={distance:.6} floor={floor}"
                ));
            }
            pairs.push((label.clone(), ink, other_name.to_owned(), other));
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
    ] {
        report.push_str(&tests::palette_pair_report(class, &pairs));
    }
    reports.push(report);

    tui_pane::set_active_theme(previous);
    assert!(
        failures.is_empty(),
        "reset semantic separation: {failures:#?}; measurements: {reports:#?}"
    );
}

#[test]
fn reset_shades_avoid_alarms_with_failed_dots_red_in_default_dark() {
    check_rendered_reset_ramp(DEFAULT_DARK_THEME, MINIMUM_RESET_ALERT_DISTANCE);
}

#[test]
fn reset_shades_avoid_alarms_with_failed_dots_red_in_default_light() {
    check_rendered_reset_ramp(DEFAULT_LIGHT_THEME, MINIMUM_LIGHT_RESET_WARNING_DISTANCE);
}

#[test]
fn reset_shades_avoid_alarms_with_failed_dots_red_in_high_contrast_dark() {
    check_rendered_reset_ramp(DEFAULT_HC_DARK_THEME, MINIMUM_RESET_ALERT_DISTANCE);
}

#[test]
fn reset_shades_avoid_alarms_with_failed_dots_red_in_high_contrast_light() {
    check_rendered_reset_ramp(DEFAULT_HC_LIGHT_THEME, MINIMUM_LIGHT_RESET_WARNING_DISTANCE);
}

#[test]
fn reset_shades_keep_saturated_chroma_and_separation_from_plain_text() {
    check_reset_chroma(DEFAULT_DARK_THEME);
}

#[test]
fn reset_shades_keep_chroma_and_text_separation_in_default_light() {
    check_reset_chroma(DEFAULT_LIGHT_THEME);
}

#[test]
fn reset_shades_keep_chroma_and_text_separation_in_high_contrast_dark() {
    check_reset_chroma(DEFAULT_HC_DARK_THEME);
}

#[test]
fn reset_shades_keep_chroma_and_text_separation_in_high_contrast_light() {
    check_reset_chroma(DEFAULT_HC_LIGHT_THEME);
}

fn check_reset_chroma(theme_name: &str) {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut failures = Vec::new();
    let variant = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == theme_name)
        .expect("the requested built-in theme exists");
    let name = variant.id.as_str().to_owned();
    tui_pane::set_active_theme(Arc::new(variant.theme));
    let mut drawings = HashMap::new();
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let text = SessionRowInk::Text.style().fg.expect("plain text ink");
        // Tint settings can resolve to the same grounds on a transparent screen.
        // Reuse that draw while checking every rank in each case.
        let grounds = (
            tui_pane::pane_background(false),
            tui_pane::pane_background(true),
        );
        let buffer = drawings.entry(grounds).or_insert_with(draw_reset_ranks);
        let reset_column = locate(buffer, "resets").column + 5;
        let positions = reset_rank_positions(buffer);
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
            let distance = footer_colors::perceptual_distance(ink, text);
            if chroma < 0.10 || distance < 15.0 {
                failures.push(format!(
                        "{name} tint={tint} {label} reset={ink:?} text={text:?} C={chroma:.9} floor=0.10 dE={distance:.9} floor=15"
                    ));
            }
        }
    }

    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "reset visibility: {failures:#?}");
}

fn check_astra_reset_shades(theme_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let theme_reset = ResetThemeFixture::install();
    let colors = footer_colors::FooterColors::parse(include_str!(
        "../footer_colors/fixtures/statusline_astra.conf"
    ));
    let mut failures = Vec::new();
    for variant in theme::builtins()
        .into_iter()
        .filter(|variant| variant.id.as_str() == theme_name)
    {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        tui_pane::set_transparent_background(false);
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let buffer = draw_reset_ranks();
            let positions = reset_rank_positions(&buffer);
            let reset_right = locate(&buffer, "resets").column + 6;
            for role in [HierarchyRole::AstraDesigner, HierarchyRole::AstraReviewer] {
                let role_ink = colors
                    .hierarchy_role(role)
                    .style(ratatui::style::Style::default())
                    .fg
                    .ok_or("Astra management ink")?;
                let mut minimum = f64::INFINITY;
                for at in positions.values() {
                    minimum = minimum.min(footer_colors::perceptual_distance(
                        role_ink,
                        buffer[(reset_right - 1, at.row)].fg,
                    ));
                }
                eprintln!("ASTRA_RESET {name} tint={tint} {role:?}: {minimum:.6}");
                if minimum < 15.0 {
                    failures.push(format!("{name} tint={tint} {role:?}: {minimum:.6}"));
                }
            }
        }
    }
    drop(theme_reset);
    assert!(failures.is_empty(), "Astra/reset distances: {failures:?}");
    Ok(())
}

#[test]
fn astra_role_inks_stay_distinct_from_default_light_reset_shades()
-> Result<(), Box<dyn std::error::Error>> {
    check_astra_reset_shades("Default Light")
}

#[test]
fn astra_role_inks_stay_distinct_from_high_contrast_light_reset_shades()
-> Result<(), Box<dyn std::error::Error>> {
    check_astra_reset_shades("High Contrast Light")
}
