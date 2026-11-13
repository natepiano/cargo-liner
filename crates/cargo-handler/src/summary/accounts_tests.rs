//! Accounts retain source ordering, nullable readings and their drawn styles.

#![expect(
    clippy::expect_used,
    reason = "fixture readings and drawn labels must exist"
)]
#![expect(
    clippy::panic,
    reason = "a missing rendered account label fails the fixture"
)]

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Local;
use chrono::Timelike;
use ratatui::style::Color;
use serde_json::Value;
use tui_pane::Theme;

use super::*;
use crate::app::App;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::census::hierarchy::HierarchyError;
use crate::census::scan::ScanStage;
use crate::footer_colors::FooterColor;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::footer_colors::tests;
use crate::render;
use crate::sort::AccountsSort;
use crate::theme;

const NOW: u64 = 1_792_166_400;
const RESET: &str = "2026-10-19T06:00:00Z";
const COMPLETE_ACCOUNTS_WIDTH: u16 = 109;

#[test]
fn board_count_headers_keep_the_accounts_board_order() {
    let headers: Vec<_> = AccountsColumn::COUNTS
        .into_iter()
        .map(AccountsColumn::header)
        .collect();
    assert_eq!(
        headers,
        [
            "busy", "starting", "waiting", "idle", "paused", "exited", "total"
        ]
    );
}

#[test]
fn board_role_rows_and_total_draw_all_five_source_counts() {
    let mut account = usage(
        "board-account",
        &["units", "workers"],
        &Value::Null,
        &Value::Null,
        false,
        0,
    );
    account["role_counts"] = serde_json::json!([
        {"busy":2,"starting":23,"waiting":29,"idle":3,"paused":5,"exited":7,"total":69},
        {"busy":11,"starting":31,"waiting":37,"idle":13,"paused":17,"exited":19,"total":128}
    ]);
    let table = contract_table(&[account]);
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    let first = at(&buffer, "board-account").1;
    let total = (first + 2..buffer.area.bottom())
        .find(|&row| {
            contract_cell(&buffer, row, &ranges[AccountsColumn::Account.index()]) == "total"
        })
        .expect("board total row");
    for (name, expected) in [
        ("busy", ["2", "11", "13"]),
        ("starting", ["23", "31", "54"]),
        ("waiting", ["29", "37", "66"]),
        ("idle", ["3", "13", "16"]),
        ("paused", ["5", "17", "22"]),
        ("exited", ["7", "19", "26"]),
        ("total", ["69", "128", "197"]),
    ] {
        let column = AccountsColumn::ALL
            .into_iter()
            .find(|column| column.header() == name)
            .expect("board count column");
        for (row, expected) in [first, first + 1, total].into_iter().zip(expected) {
            assert_eq!(
                contract_cell(&buffer, row, &ranges[column.index()]),
                expected,
                "{name} at row {row}"
            );
        }
    }
}

#[test]
fn narrow_unicode_cuts_use_terminal_cells_and_keep_whole_graphemes() {
    for (input, width, expected) in [
        ("界界界", 4, "界…"),
        ("e\u{301}e\u{301}e\u{301}", 2, "e\u{301}…"),
        ("☀\u{fe0f}☀\u{fe0f}", 3, "☀\u{fe0f}…"),
        ("👩\u{200d}💻project", 3, "👩\u{200d}💻…"),
    ] {
        assert_eq!(
            truncated(input, width),
            expected,
            "input={input:?}, cells={width}"
        );
        assert_eq!(
            usize::from(cell_width(expected)),
            Line::raw(expected).width(),
            "the summary measures the same terminal cells as the renderer"
        );
    }
}

#[test]
fn narrow_unicode_cut_lines_keep_each_visible_spans_ink() {
    let first = Style::default().fg(Color::Red);
    let second = Style::default().fg(Color::Blue);
    let cut = truncated_line(
        Line::from(vec![
            Span::styled("界", first),
            Span::styled("e\u{301}tail", second),
        ]),
        4,
    );
    let visible: String = cut.spans.iter().map(|span| span.content.as_ref()).collect();
    assert_eq!(visible, "界e\u{301}…");
    assert_eq!(
        cut.width(),
        4,
        "wide glyph and combining mark count by cells"
    );
    assert_eq!(cut.spans[0].style, first);
    assert!(
        cut.spans.iter().skip(1).all(|span| span.style == second),
        "visible second grapheme and its ellipsis retain their ink: {cut:?}"
    );
}

/// Restore the caller's appearance even when a redraw assertion fails.
struct SummaryAppearanceGuard {
    theme:       Arc<Theme>,
    transparent: bool,
    tinted:      bool,
}

impl SummaryAppearanceGuard {
    fn save() -> Self {
        Self {
            theme:       tui_pane::theme(),
            transparent: tui_pane::transparent_background(),
            tinted:      tui_pane::focused_pane_tint_enabled(),
        }
    }
}

impl Drop for SummaryAppearanceGuard {
    fn drop(&mut self) {
        tui_pane::set_active_theme(std::sync::Arc::clone(&self.theme));
        tui_pane::set_transparent_background(self.transparent);
        tui_pane::set_focused_pane_tint(self.tinted);
    }
}

fn redraw_account_summary(table: &AccountsTable, colors: &FooterColors) -> Buffer {
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 200, 20);
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(tui_pane::pane_background(true)));
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            NOW,
            account_column: AccountColumn::Hidden,
            footer_colors:  colors,
            sort:           SummarySort::default(),
            accounts:       table,
        },
    );
    buffer
}

fn assert_current_account_summary_inks(buffer: &Buffer) -> [Color; 2] {
    let expected = [
        FooterColor::Magenta
            .account_style()
            .fg
            .expect("account ink"),
        FooterColor::Magenta
            .management_style()
            .fg
            .expect("role ink"),
    ];
    let ground = tui_pane::pane_background(true);
    for (word, ink) in ["claude1", "showrunners"].into_iter().zip(expected) {
        let (x, y) = at(buffer, word);
        for offset in 0..cell_width(word) {
            let cell = &buffer[(x + offset, y)];
            assert_eq!(cell.fg, ink, "current {word} ink: {cell:?}");
            assert_eq!(cell.bg, ground, "current {word} ground: {cell:?}");
        }
    }
    expected
}

#[test]
fn account_summary_redraw_resolves_inks_after_focused_tint_and_theme_changes() {
    let _appearance = SummaryAppearanceGuard::save();
    let dark = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == "Default Dark")
        .expect("dark redraw theme");
    let light = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == "Default Light")
        .expect("light redraw theme");
    tui_pane::set_active_theme(std::sync::Arc::new(dark.theme));
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(false);
    let table = contract_table(&[usage(
        "claude1",
        &["showrunners"],
        &serde_json::json!(60),
        &serde_json::json!(13_446),
        true,
        0,
    )]);
    let colors = FooterColors::parse("account.claude1=magenta\nrole.showrunner=magenta");
    let first = redraw_account_summary(&table, &colors);
    let first_inks = assert_current_account_summary_inks(&first);

    tui_pane::set_focused_pane_tint(true);
    let tinted = redraw_account_summary(&table, &colors);
    let tinted_inks = assert_current_account_summary_inks(&tinted);
    assert_eq!(first.area, tinted.area, "tint redraw keeps the width");
    assert_ne!(
        first[at(&first, "claude1")].bg,
        tinted[at(&tinted, "claude1")].bg
    );
    for (before, after) in first_inks.into_iter().zip(tinted_inks) {
        assert_ne!(before, after, "tint case must detect reused foregrounds");
    }

    tui_pane::set_active_theme(std::sync::Arc::new(light.theme));
    let light = redraw_account_summary(&table, &colors);
    let light_inks = assert_current_account_summary_inks(&light);
    assert_eq!(tinted.area, light.area, "theme redraw keeps the width");
    for (before, after) in tinted_inks.into_iter().zip(light_inks) {
        assert_ne!(before, after, "theme case must detect reused foregrounds");
    }
}

fn usage(
    label: &str,
    roles: &[&str],
    left: &Value,
    credits: &Value,
    use_credits: bool,
    age: u64,
) -> Value {
    serde_json::json!({
        "label":label, "tool":if label.starts_with("claude") {"claude"} else {"codex"},
        "roles":roles,"weekly_left_percent":left,"resets_at":RESET,
        "credits":credits,"use_credits":use_credits,
        "read_at":DateTime::from_timestamp(i64::try_from(NOW-age).expect("fixture time"),0).expect("UTC time").to_rfc3339()
    })
}

fn reading() -> AccountsReading {
    let mut unavailable = usage("claude3", &[], &Value::Null, &Value::Null, false, 0);
    unavailable["resets_at"] = Value::Null;
    unavailable["read_at"] = Value::Null;
    unavailable["email"] = serde_json::json!("fixture@example.test");
    unavailable["folder"] = serde_json::json!("/private/account-home");
    AccountsReading::parse(&serde_json::to_vec(&serde_json::json!({
        "accounts":[
            usage("codex2", &["workers"], &serde_json::json!(10), &serde_json::json!(12.5), true, 30),
            usage("claude1", &["showrunners","units"], &serde_json::json!(10.1), &serde_json::Value::Null, false, 3600),
            usage("codex1", &["spare"], &serde_json::json!(9), &serde_json::json!("unlimited"), false, 7200),
            unavailable
        ],
        "problems":["cached role map unavailable for fixture-host"]
    })).expect("fixture JSON encodes")).expect("accounts.py wire shape")
}

fn agents() -> Vec<AgentRow> {
    [(10, "zeta-session", 100), (20, "alpha-session", 200)]
        .map(|(pid, name, started)| {
            serde_json::from_value(serde_json::json!({
                "agent":"claude","name":name,"started":started,"pid":pid,
                "directory":"/fixture","desktop":null,"branch":null,"launched_by":null,"children":[{"depth":0,"kind":"shell","pid":9999,"name":"fixture command","started":100}]
            }))
            .expect("agent fixture")
        })
        .into()
}

pub(super) fn drawn(table: &AccountsTable) -> Buffer {
    let colors = FooterColors::parse(
        "account.codex2=orange\naccount.claude1=bright-cyan\naccount.codex1=purple",
    );
    drawn_with_colors(table, &colors)
}

fn drawn_with_colors(table: &AccountsTable, colors: &FooterColors) -> Buffer {
    let state = MachineState::Answered(agents(), HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    let frame = SummaryFrame {
        accounts_sort:  AccountsSort::SourceOrder,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: AccountColumn::Hidden,
        footer_colors:  colors,
        sort:           SummarySort::default(),
        accounts:       table,
    };
    let area = Rect::new(
        3,
        2,
        width(
            frame.machines,
            frame.placement,
            frame.now,
            frame.account_column,
            frame.sort,
            frame.accounts,
            AccountsSort::SourceOrder,
        ),
        u16::try_from(height(&machines, &placement, table)).expect("small summary"),
    );
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, area, &frame);
    buffer
}

pub(super) fn text(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn at(buffer: &Buffer, label: &str) -> (u16, u16) {
    for (row, line) in text(buffer).iter().enumerate() {
        if let Some(offset) = line.find(label) {
            return (
                buffer.area.x + u16::try_from(line[..offset].chars().count()).expect("column"),
                buffer.area.y + u16::try_from(row).expect("row"),
            );
        }
    }
    panic!("missing drawn label {label}: {:?}", text(buffer));
}

#[test]
fn accounts_table_measured_demand_draws_both_tables_source_order_and_local_reset() {
    let buffer = drawn(&AccountsTable::Ready(reading()));
    let lines = text(&buffer);
    let joined = lines.join("\n");
    assert!(
        joined.contains("zeta-session") && joined.contains("alpha-session"),
        "{joined}"
    );
    let accounts_y = at(&buffer, "accounts").1;
    let agents_y = at(&buffer, "alpha-session").1;
    assert_eq!(
        accounts_y,
        agents_y + 2,
        "one blank line separates the tables"
    );
    for header in [
        "account", "left", "role", "busy", "starting", "waiting", "idle", "paused", "exited",
        "total", "resets", "credits", "enabled",
    ] {
        let header_y = accounts_y + 1;
        assert!(
            lines[usize::from(header_y - buffer.area.y)].contains(header),
            "{joined}"
        );
    }
    let positions = ["codex2", "claude1", "codex1", "claude3"].map(|label| at(&buffer, label).1);
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "source order: {joined}"
    );
    let first_role_row = usize::from(at(&buffer, "claude1").1 - buffer.area.y);
    assert!(lines[first_role_row].contains("showrunners"), "{joined}");
    assert!(lines[first_role_row + 1].contains("units"), "{joined}");
    assert!(
        joined.contains("unlimited") && joined.contains("13"),
        "{joined}"
    );
    let reset = DateTime::parse_from_rfc3339(RESET)
        .expect("reset timestamp")
        .with_timezone(&Local)
        .format("%a %H:%M")
        .to_string();
    assert!(joined.contains(&reset), "local reset: {joined}");
    assert_eq!(
        lines
            .iter()
            .find(|line| line.contains("cached role map unavailable"))
            .expect("problem row")
            .trim(),
        "cached role map unavailable for fixture-host"
    );
    assert!(
        !joined.contains('@') && !joined.contains("login") && !joined.contains("on-credits"),
        "{joined}"
    );
    assert!(!joined.contains("/private/account-home"), "{joined}");
    assert_eq!(
        buffer[at(&buffer, "left")].fg,
        buffer[at(&buffer, "session")].fg,
        "both tables share the header style"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.split_whitespace().any(|word| word == "status")),
        "{joined}"
    );
}

#[test]
fn accounts_table_reset_minutes_round_hour_jitter_and_half_minute_up() {
    let cases = [
        ("before-hour", "2026-10-19T05:59:59.63+00:00", RESET),
        ("after-hour", "2026-10-19T06:00:00.47+00:00", RESET),
        ("below-half", "2026-10-19T06:00:29.999+00:00", RESET),
        (
            "at-half",
            "2026-10-19T06:00:30+00:00",
            "2026-10-19T06:01:00Z",
        ),
        (
            "above-half",
            "2026-10-19T06:00:30.001+00:00",
            "2026-10-19T06:01:00Z",
        ),
    ];
    let accounts: Vec<_> = cases
        .iter()
        .map(|(label, reset, _)| {
            let mut row = usage(label, &[], &Value::Null, &Value::Null, false, 0);
            row["resets_at"] = serde_json::json!(reset);
            row
        })
        .collect();
    let table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({"accounts":accounts,"problems":[]}))
                .expect("reset fixtures encode"),
        )
        .expect("reset fixtures parse"),
    );
    let buffer = drawn(&table);
    let column = at(&buffer, "resets").0 + 6 - 9;
    for (label, _, rounded) in cases {
        let row = at(&buffer, label).1;
        let actual: String = (column..column + 9)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        let expected = DateTime::parse_from_rfc3339(rounded)
            .expect("expected minute")
            .with_timezone(&Local)
            .format("%a %H:%M")
            .to_string();
        assert_eq!(actual, expected, "reset for {label}");
    }
}

#[test]
fn accounts_table_threshold_footer_credits_staleness_and_unavailable_cells_draw_styles() {
    let buffer = drawn(&AccountsTable::Ready(reading()));
    for (label, expected) in [
        ("codex2", Color::Rgb(255, 135, 0)),
        ("claude1", Color::Rgb(0, 255, 255)),
    ] {
        let ink = buffer[at(&buffer, label)].fg;
        assert_eq!(tui_pane::blend_color(ink, ink, 0), expected);
    }
    let low = buffer[at(&buffer, "9%")].fg;
    let warning = buffer[at(&buffer, "10%")].fg;
    assert_ne!(low, warning, "9% is low; rounded10% is warning");
    let ranges = contract_ranges(&buffer);
    for (label, credits, enabled) in [
        ("codex2", "13", "yes"),
        ("claude1", "", ""),
        ("codex1", "unlimited", ""),
        ("claude3", "", ""),
    ] {
        let y = at(&buffer, label).1;
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Account.index()]),
            label,
            "account holds only its name"
        );
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Credits.index()]),
            credits
        );
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Enabled.index()]),
            enabled
        );
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Busy.index()]),
            ""
        );

        assert!(!buffer[at(&buffer, label)].modifier.contains(Modifier::DIM));
    }
    assert!(!text(&buffer).join("\n").contains("old"));
    assert!(
        buffer[at(&buffer, "cached role map")]
            .modifier
            .contains(Modifier::DIM)
    );
}

#[test]
fn zero_weekly_quota_remains_readable_on_every_drawn_theme_ground() {
    let previous = tui_pane::theme();
    let mut row = usage(
        "claude1",
        &["units"],
        &serde_json::json!(0),
        &Value::Null,
        false,
        0,
    );
    row["resets_at"] = Value::Null;
    let table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({"accounts":[row],"problems":[]}))
                .expect("zero-quota fixture encodes"),
        )
        .expect("zero-quota fixture parses"),
    );
    for variant in theme::builtins() {
        tui_pane::set_active_theme(std::sync::Arc::new(variant.theme));
        for ground in [
            tui_pane::theme().text.bg_focus.color,
            tui_pane::pane_background(false),
            tui_pane::pane_background(true),
        ] {
            let area = Rect::new(0, 0, 90, 3);
            let mut buffer = Buffer::empty(area);
            buffer.set_style(area, Style::default().bg(ground));
            accounts::draw_sorted(
                &mut buffer,
                area,
                &table,
                NOW,
                &FooterColors::default(),
                AccountsSort::SourceOrder,
            );
            let (x, y) = at(&buffer, "0%");
            for column in x..x + 2 {
                let cell = &buffer[(column, y)];
                let ratio = tests::contrast_ratio(cell.fg, cell.bg);
                assert!(
                    ratio >= MINIMUM_CONTRAST,
                    "{} zero quota: {:?} on {:?} is {ratio:.2}:1",
                    variant.id.as_str(),
                    cell.fg,
                    cell.bg,
                );
            }
        }
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn accounts_contrast_fresh_stale_and_unread_labels_remain_readable_on_every_theme() {
    let previous = tui_pane::theme();
    let colors = FooterColors::parse(
        "account.codex2=blue\naccount.claude1=blue\naccount.codex1=blue\naccount.claude3=blue",
    );
    for variant in theme::builtins() {
        let background = variant.theme.text.bg_focus.color;
        tui_pane::set_active_theme(std::sync::Arc::new(variant.theme));
        let buffer = drawn_with_colors(&AccountsTable::Ready(reading()), &colors);
        for label in ["codex2", "claude1", "codex1", "claude3"] {
            let (x, y) = at(&buffer, label);
            for column in x..x + u16::try_from(label.len()).expect("short account label") {
                let cell = &buffer[(column, y)];
                assert!(
                    !cell.modifier.contains(Modifier::DIM),
                    "{} {label}: terminal DIM defeats contrast",
                    variant.id.as_str()
                );
                let ratio = tests::contrast_ratio(cell.fg, background);
                assert!(
                    ratio >= MINIMUM_CONTRAST,
                    "{} {label}: {:?} on {background:?} is {ratio}:1",
                    variant.id.as_str(),
                    cell.fg
                );
            }
        }
        let stale = buffer[at(&buffer, "codex1")].fg;
        let unread = buffer[at(&buffer, "claude3")].fg;
        assert!(
            matches!(stale, Color::Rgb(..)),
            "stale labels use a computed subdued foreground"
        );
        assert_eq!(
            stale, unread,
            "both uncertain readings have the same subdued blue"
        );
        let stale_line = &text(&buffer)[usize::from(at(&buffer, "codex1").1 - buffer.area.y)];
        assert!(
            !stale_line.contains("2h") && !stale_line.contains("old"),
            "{stale_line}"
        );
        let unread_line = &text(&buffer)[usize::from(at(&buffer, "claude3").1 - buffer.area.y)];
        assert!(
            !unread_line.contains("old"),
            "unread age must not be invented: {unread_line}"
        );
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn accounts_table_every_state_preserves_agents_and_failure_retains_the_last_reading() {
    let mut ready = reading();
    ready.received_at = NOW - 14 * 60;
    let failure = AccountsFailure::Command(HierarchyError::ExitedUnsuccessfully);
    let mut table = AccountsTable::AwaitingFirstReading;
    assert_eq!(
        text(&drawn(&table)),
        text(&drawn(&AccountsTable::NotConfigured))
    );
    assert!(table.apply(Ok(ready.clone())));
    assert!(table.apply(Err(failure)));
    assert!(!table.apply(Err(failure)));
    for current in [
        AccountsTable::NotConfigured,
        AccountsTable::AwaitingFirstReading,
        AccountsTable::Ready(ready),
        AccountsTable::Failed(AccountsFailure::InvalidReading),
        table,
    ] {
        let buffer = drawn(&current);
        let joined = text(&buffer).join("\n");
        assert!(
            joined.contains("zeta-session") && joined.contains("alpha-session"),
            "{joined}"
        );
        match current {
            AccountsTable::NotConfigured | AccountsTable::AwaitingFirstReading => {
                assert!(!joined.contains("accounts"), "{joined}");
            },
            AccountsTable::Ready(_) => assert!(
                joined.contains("codex2") && !joined.contains("failed"),
                "{joined}"
            ),
            AccountsTable::Failed(_) => {
                assert!(!joined.contains("codex2"), "{joined}");
                assert!(joined.contains("invalid JSON"), "{joined}");
                assert!(
                    buffer[at(&buffer, "invalid JSON")]
                        .modifier
                        .contains(Modifier::DIM)
                );
            },
            AccountsTable::FailedKeepingLast { .. } => {
                assert!(
                    joined.contains("codex2")
                        && joined.contains("accounts: command failed")
                        && joined.contains("showing the reading from 14 min ago"),
                    "{joined}"
                );
                assert!(
                    buffer[at(&buffer, "accounts: command failed")]
                        .modifier
                        .contains(Modifier::DIM)
                );
            },
        }
    }
}

#[test]
fn accounts_table_wide_problem_and_failure_notes_fit_the_measured_width() {
    let mut source = reading();
    source.received_at = NOW - 14 * 60;
    let problem = "stored role cache unavailable: the producer could not locate the source's last known role map on the fixture machine";
    source.problems = vec![problem.to_owned()];
    let table = AccountsTable::FailedKeepingLast {
        last:    source,
        failure: AccountsFailure::InvalidReading,
    };
    let buffer = drawn(&table);
    let lines = text(&buffer);
    assert!(lines.iter().any(|line| line.contains(problem)), "{lines:?}");
    assert_eq!(
        lines
            .iter()
            .find(|line| line.contains("invalid JSON"))
            .expect("failure note")
            .trim(),
        "accounts: invalid JSON reading; showing the reading from 14 min ago"
    );
}

#[test]
fn account_credit_policy_from_wire_controls_balance_emphasis() {
    let table = AccountsTable::Ready(AccountsReading::parse(br#"{
        "accounts":[
            {"label":"held","roles":[],"weekly_left_percent":50,"resets_at":null,"credits":12.5,"use_credits":false,"read_at":"2026-10-16T16:00:00Z"},
            {"label":"spending","roles":[],"weekly_left_percent":50,"resets_at":null,"credits":12.5,"use_credits":true,"read_at":"2026-10-16T16:00:00Z"}
        ],"problems":[]
    }"#).expect("wire credit policy"));
    let buffer = drawn(&table);
    let range = contract_ranges(&buffer)[AccountsColumn::Credits.index()].clone();
    let held_y = at(&buffer, "held").1;
    let spending_y = at(&buffer, "spending").1;
    assert_eq!(contract_cell(&buffer, held_y, &range), "13");
    assert_eq!(contract_cell(&buffer, spending_y, &range), "13");
    let held = &buffer[(range.end - 1, held_y)];
    let spending = &buffer[(range.end - 1, spending_y)];
    assert!(
        !held.modifier.contains(Modifier::DIM),
        "credit permission cannot delegate readable ink to terminal dimming"
    );
    assert!(!spending.modifier.contains(Modifier::DIM));
    assert_ne!(
        held.fg, spending.fg,
        "disabled permission keeps subdued balance emphasis"
    );
}

#[test]
fn accounts_only_summary_demand_includes_rows_and_all_notes() {
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = [];
    let colors = FooterColors::default();
    let mut source = reading();
    let problem = "stored account source: the cache could not load the last known role map for this fixture machine";
    source.problems = vec![problem.to_owned()];
    for table in [
        AccountsTable::Ready(source.clone()),
        AccountsTable::FailedKeepingLast {
            last:    source,
            failure: AccountsFailure::InvalidReading,
        },
        AccountsTable::Failed(AccountsFailure::InvalidReading),
    ] {
        let area = Rect::new(
            0,
            0,
            width(
                &machines,
                &placement,
                NOW,
                AccountColumn::Hidden,
                SummarySort::default(),
                &table,
                AccountsSort::SourceOrder,
            ),
            u16::try_from(height(&machines, &placement, &table)).expect("summary height"),
        );
        let mut buffer = Buffer::empty(area);
        draw(
            &mut buffer,
            area,
            &SummaryFrame {
                accounts_sort:  AccountsSort::SourceOrder,
                accounts:       &table,
                machines:       &machines,
                placement:      &placement,
                cells:          &cells,
                now:            NOW,
                account_column: AccountColumn::Hidden,
                footer_colors:  &colors,
                sort:           SummarySort::default(),
            },
        );
        let output = text(&buffer);
        let joined = output.join("\n");
        match table {
            AccountsTable::Ready(_) => {
                assert!(joined.contains("claude3"), "{joined}");
                assert_eq!(
                    output
                        .iter()
                        .find(|line| line.contains("stored account source:"))
                        .expect("problem")
                        .trim(),
                    problem
                );
            },
            AccountsTable::FailedKeepingLast { .. } => {
                assert!(
                    joined.contains("claude3") && joined.contains(problem),
                    "{joined}"
                );
                assert_eq!(
                    output
                        .iter()
                        .find(|line| line.contains("invalid JSON"))
                        .expect("failure")
                        .trim(),
                    "accounts: invalid JSON reading; showing the reading from 0 min ago"
                );
            },
            AccountsTable::Failed(_) => {
                assert_eq!(output.join("\n").trim(), "accounts: invalid JSON reading");
            },
            _ => panic!("fixture states must have visible content"),
        }
    }
}

/// Render only the account section on a real pane ground, without clipping its demand.
fn contract_table(rows: &[Value]) -> AccountsTable {
    AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({"accounts":rows,"problems":[]}))
                .expect("account contract JSON"),
        )
        .expect("account contract wire"),
    )
}

fn contract_row(label: &str, left: &Value, credits: &Value, age: u64) -> Value {
    let mut row = usage(label, &["showrunners", "units"], left, credits, true, age);
    row["read_ok"] = serde_json::json!(true);

    row
}

fn contract_draw(table: &AccountsTable, ground: Color) -> Buffer {
    let area = Rect::new(
        0,
        0,
        accounts::sorted_width(table, NOW, AccountsSort::SourceOrder),
        24,
    );
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(ground));
    accounts::draw_sorted(
        &mut buffer,
        area,
        table,
        NOW,
        &FooterColors::default(),
        AccountsSort::SourceOrder,
    );
    buffer
}

/// Header right edges also delimit right-aligned values in the following rows.
fn contract_ranges(buffer: &Buffer) -> Vec<Range<u16>> {
    let lines = text(buffer);
    let header = lines
        .iter()
        .find(|line| line.contains("account") && line.contains("total"))
        .expect("account headers");
    let starts: Vec<_> = AccountsColumn::ALL
        .iter()
        .map(|column| {
            let offset = header.find(column.header()).expect("board column");
            buffer.area.x + cell_width(&header[..offset])
        })
        .collect();
    AccountsColumn::ALL
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let right_aligned = column.alignment() == Alignment::Right;
            let start = match column {
                AccountsColumn::Account
                | AccountsColumn::WeeklyLeft
                | AccountsColumn::Roles
                | AccountsColumn::Enabled => starts[index],
                AccountsColumn::Busy => {
                    let role_width = ["showrunners", "units", "workers", "spare"]
                        .into_iter()
                        .filter(|role| lines.iter().any(|line| line.contains(role)))
                        .map(cell_width)
                        .fold(cell_width("role"), u16::max);
                    starts[AccountsColumn::Roles.index()] + role_width + 2
                },
                _ => starts[index - 1] + cell_width(AccountsColumn::ALL[index - 1].header()) + 2,
            };
            let end = if right_aligned {
                starts[index] + cell_width(column.header())
            } else if index + 1 < starts.len() {
                starts[index + 1] - 2
            } else {
                buffer.area.right()
            };
            start..end
        })
        .collect()
}

fn contract_cell(buffer: &Buffer, row: u16, range: &Range<u16>) -> String {
    range
        .clone()
        .map(|x| buffer[(x, row)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

#[test]
fn board_columns_draw_lowercase_in_order_with_names_and_credit_states() {
    let mut second = contract_row(
        "second",
        &serde_json::json!(24),
        &serde_json::json!("unlimited"),
        7200,
    );
    second["use_credits"] = serde_json::json!(false);

    let mut missing = contract_row("missing", &Value::Null, &Value::Null, 0);
    missing["read_at"] = Value::Null;
    let table = contract_table(&[
        contract_row(
            "first",
            &serde_json::json!(60.4),
            &serde_json::json!(13_446.4),
            45,
        ),
        second,
        missing,
    ]);
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let header = &text(&buffer)[1];
    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        [
            "account", "left", "role", "busy", "starting", "waiting", "idle", "paused", "exited",
            "total", "resets", "credits", "enabled"
        ]
    );
    let ranges = contract_ranges(&buffer);
    for (label, left, credits, enabled) in [
        ("first", "60%", "13,446", "yes"),
        ("second", "24%", "unlimited", ""),
        ("missing", "—", "", "yes"),
    ] {
        let y = at(&buffer, label).1;
        for (index, expected) in [
            (0, label),
            (1, left),
            (AccountsColumn::Credits.index(), credits),
            (AccountsColumn::Enabled.index(), enabled),
            (3, ""),
            (4, ""),
            (5, ""),
            (6, ""),
            (7, ""),
            (8, ""),
            (9, ""),
        ] {
            assert_eq!(
                contract_cell(&buffer, y, &ranges[index]),
                expected,
                "{label} column {index}: {:?}",
                text(&buffer)
            );
        }
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Roles.index()]),
            "showrunners"
        );
        assert_eq!(
            contract_cell(&buffer, y + 1, &ranges[AccountsColumn::Roles.index()]),
            "units"
        );
        assert_eq!(
            contract_cell(&buffer, y + 1, &ranges[AccountsColumn::Account.index()]),
            ""
        );
    }
    assert!(!text(&buffer).join("\n").contains("old"));
}

#[test]
fn read_outcome_dots_distinguish_ok_failed_and_missing_wire_fields() {
    let ok = contract_row("read-ok", &serde_json::json!(25), &Value::Null, 30);
    let mut failed = contract_row("read-failed", &serde_json::json!(9), &Value::Null, 30);
    failed["read_ok"] = serde_json::json!(false);
    let mut missing = contract_row("read-legacy", &serde_json::json!(24), &Value::Null, 30);
    missing
        .as_object_mut()
        .expect("wire object")
        .remove("read_ok");
    let buffer = contract_draw(
        &contract_table(&[ok, failed, missing]),
        tui_pane::pane_background(false),
    );
    let mut dots = Vec::new();
    for (label, expected) in [
        ("read-ok", tui_pane::success_color()),
        ("read-failed", tui_pane::error_color()),
        ("read-legacy", tui_pane::error_color()),
    ] {
        let (account, y) = at(&buffer, label);
        let x = (buffer.area.x..account)
            .find(|&x| buffer[(x, y)].symbol() == "●")
            .expect("account read dot");
        let actual = buffer[(x, y)].fg;
        assert_eq!(
            tui_pane::blend_color(actual, actual, 0),
            tui_pane::blend_color(expected, expected, 0),
            "{label}"
        );
        assert!(!(buffer.area.x..account).any(|x| buffer[(x, y + 1)].symbol() == "●"));
        dots.push(actual);
    }
    assert_ne!(dots[0], dots[1]);
    let ranges = contract_ranges(&buffer);
    let inks: Vec<_> = ["read-failed", "read-legacy", "read-ok"]
        .map(|label| {
            let y = at(&buffer, label).1;
            let x = ranges[AccountsColumn::WeeklyLeft.index()]
                .clone()
                .find(|&x| !buffer[(x, y)].symbol().trim().is_empty())
                .expect("left ink");
            buffer[(x, y)].fg
        })
        .into();
    assert_ne!(inks[0], inks[1], "9% red differs from 24% yellow");
    assert_ne!(inks[1], inks[2], "24% yellow differs from 25% plain");
}

#[test]
fn account_numeric_formats_align_their_last_cells() {
    let table = contract_table(
        &[
            ("seconds", 45, 9.4, 13_446.4),
            ("minutes", 720, 24.4, 2.0),
            ("hours", 7200, 25.0, 100.0),
            ("days", 259_200, 100.0, 999.6),
        ]
        .map(|(label, age, left, credits)| {
            let mut row = contract_row(
                label,
                &serde_json::json!(left),
                &serde_json::json!(credits),
                age,
            );
            row["role_counts"] = serde_json::json!([
                {"busy": age, "idle": 2, "paused":1,"exited":0,"total": age + 3},
                {"busy": 0, "idle": 0, "paused":0,"exited":0,"total": 0}
            ]);
            row
        }),
    );
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    for (label, age, left, credits) in [
        ("seconds", "45", "9%", "13,446"),
        ("minutes", "720", "24%", "2"),
        ("hours", "7200", "25%", "100"),
        ("days", "259200", "100%", "1,000"),
    ] {
        let y = at(&buffer, label).1;
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Busy.index()]),
            age
        );
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::WeeklyLeft.index()]),
            left
        );
        assert_eq!(
            contract_cell(&buffer, y, &ranges[AccountsColumn::Credits.index()]),
            credits
        );
        for index in [1, 8, 9, 3, 4, 5, 6, 7] {
            assert!(
                !buffer[(ranges[index].end - 1, y)]
                    .symbol()
                    .trim()
                    .is_empty(),
                "{label} numeric column {index} must end at header edge"
            );
            assert_eq!(
                buffer[(ranges[index].end, y)].symbol(),
                " ",
                "numeric trailing gap"
            );
        }
        assert_eq!(
            buffer[(ranges[AccountsColumn::WeeklyLeft.index()].end - 1, y)].symbol(),
            "%"
        );
    }
}

/// Independent glyph oracle: production coverage is deliberately not imported.
fn account_glyph_contrast(foreground: Color, background: Color) -> f64 {
    let rgb = |color| {
        let Color::Rgb(r, g, b) = tui_pane::blend_color(color, color, 0) else {
            panic!("explicit RGB color required")
        };
        [r, g, b].map(f64::from)
    };
    let fg = rgb(foreground);
    let bg = rgb(background);
    // Measure fractional and nearest-byte covered channels independently.
    let covered =
        std::array::from_fn::<_, 3, _>(|i| f64::mul_add(bg[i], 47.0, fg[i] * 208.0) / 255.0);
    let luminance = |channels: [f64; 3]| {
        let [r, g, b] = channels.map(|c| {
            let v = c / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        0.0722f64.mul_add(b, 0.7152f64.mul_add(g, 0.2126 * r))
    };
    let b = luminance(bg);
    [covered, covered.map(f64::round)]
        .map(|channels| {
            let a = luminance(channels);
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        })
        .into_iter()
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn every_account_numeric_ink_keeps_glyph_contrast_in_all_themes_and_pane_grounds() {
    let table = contract_table(
        &(0..3)
            .map(|i| {
                let mut row = contract_row(
                    &format!("rank-{i}"),
                    &serde_json::json!([9, 24, 25][i]),
                    &serde_json::json!(13_446.4),
                    [45, 720, 7200][i],
                );
                row["resets_at"] = serde_json::json!(
                    DateTime::from_timestamp(
                        i64::try_from(NOW).expect("now")
                            + i64::try_from(i + 1).expect("rank") * 3600,
                        0
                    )
                    .expect("reset")
                    .to_rfc3339()
                );
                row["use_credits"] = serde_json::json!(false);
                row["role_counts"] = serde_json::json!([
                    {"busy":1,"idle":3,"paused":2,"exited":0,"total":6},
                    {"busy":4,"idle":6,"paused":5,"exited":0,"total":15}
                ]);
                row
            })
            .collect::<Vec<_>>(),
    );
    let previous = tui_pane::theme();
    let mut failures = Vec::new();
    for variant in theme::builtins() {
        tui_pane::set_active_theme(std::sync::Arc::new(variant.theme));
        for ground in [
            tui_pane::theme().text.bg_focus.color,
            tui_pane::pane_background(false),
            tui_pane::pane_background(true),
        ] {
            let buffer = contract_draw(&table, ground);
            let ranges = contract_ranges(&buffer);
            let rows: Vec<_> = (0..3)
                .flat_map(|rank| {
                    let first = at(&buffer, &format!("rank-{rank}")).1;
                    [first, first + 1]
                })
                .collect();
            for column in [
                AccountsColumn::WeeklyLeft,
                AccountsColumn::Resets,
                AccountsColumn::Credits,
            ]
            .into_iter()
            .chain(AccountsColumn::COUNTS)
            {
                let index = column.index();
                for &y in &rows {
                    if AccountsColumn::COUNTS.contains(&column) {
                        assert!(
                            !contract_cell(&buffer, y, &ranges[index]).is_empty(),
                            "count glyphs must draw"
                        );
                    }
                    for x in ranges[index].clone() {
                        let cell = &buffer[(x, y)];
                        if cell.symbol().trim().is_empty() {
                            continue;
                        }
                        assert!(
                            !cell.modifier.contains(Modifier::DIM),
                            "numeric ink must not rely on terminal DIM: {cell:?}"
                        );
                        let ratio = account_glyph_contrast(cell.fg, cell.bg);
                        if ratio < 4.5 {
                            failures.push(format!(
                                "{} {index} row{y} {cell:?} ratio{ratio:.6}",
                                variant.id.as_str()
                            ));
                        }
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    assert!(failures.is_empty(), "numeric glyph contrast {failures:#?}");
}

#[test]
fn account_resets_use_same_day_week_and_later_calendar_formats() {
    let now = DateTime::from_timestamp(i64::try_from(NOW).expect("now"), 0)
        .expect("now UTC")
        .with_timezone(&Local);
    let afternoon = now
        .with_hour(14)
        .expect("hour")
        .with_minute(0)
        .expect("minute")
        .with_second(0)
        .expect("second");
    let dates = [
        afternoon,
        afternoon + chrono::Duration::days(3),
        afternoon + chrono::Duration::days(8),
    ];
    let formats = ["%H:%M", "%a %H:%M", "%d %b"];
    let table = contract_table(
        &dates
            .iter()
            .enumerate()
            .map(|(i, date)| {
                let mut row = contract_row(
                    &format!("calendar-{i}"),
                    &serde_json::json!(50),
                    &Value::Null,
                    0,
                );
                row["resets_at"] = serde_json::json!(date.to_rfc3339());
                row
            })
            .collect::<Vec<_>>(),
    );
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    for (i, (date, format)) in dates.iter().zip(formats).enumerate() {
        assert_eq!(
            contract_cell(
                &buffer,
                at(&buffer, &format!("calendar-{i}")).1,
                &ranges[AccountsColumn::Resets.index()]
            ),
            date.format(format).to_string()
        );
    }
}

fn narrow_accounts() -> AccountsTable {
    let mut first = contract_row(
        "first",
        &serde_json::json!(60),
        &serde_json::json!(13_446.4),
        7200,
    );
    first["resets_at"] = serde_json::json!("2026-10-17T14:00:00Z");

    let mut second = contract_row(
        "second",
        &serde_json::json!(24),
        &serde_json::json!("unlimited"),
        45,
    );
    second["resets_at"] = serde_json::json!("2026-10-18T14:00:00Z");
    contract_table(&[first, second])
}

fn narrow_draw(table: &AccountsTable, width: u16, height: u16) -> Buffer {
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = [];
    let colors = FooterColors::default();
    let frame = SummaryFrame {
        accounts_sort:  AccountsSort::SourceOrder,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: AccountColumn::Hidden,
        footer_colors:  &colors,
        sort:           SummarySort::default(),
        accounts:       table,
    };
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, area, &frame);
    buffer
}

fn assert_complete_account_values(buffer: &Buffer) {
    let lines = text(buffer);
    let output = lines.join("\n");
    for value in [
        "first",
        "second",
        "showrunners",
        "units",
        "60%",
        "24%",
        "13,446",
        "unlimited",
        "busy",
        "total",
    ] {
        assert!(
            output.contains(value),
            "missing complete {value}: {lines:#?}"
        );
    }
    for reset in ["2026-10-17T14:00:00Z", "2026-10-18T14:00:00Z"] {
        let expected = DateTime::parse_from_rfc3339(reset)
            .expect("reset fixture")
            .with_timezone(&Local)
            .format("%a %H:%M")
            .to_string();
        assert!(
            output.contains(&expected),
            "missing complete reset {expected}: {lines:#?}"
        );
    }
    let first = usize::from(at(buffer, "first").1);
    let second = usize::from(at(buffer, "second").1);
    for (range, expected) in [
        (first..second, ["showrunners", "units", "60%", "13,446"]),
        (
            second..lines.len(),
            ["showrunners", "units", "24%", "unlimited"],
        ),
    ] {
        let account = lines[range].join("\n");
        for value in expected {
            assert!(
                account.contains(value),
                "missing complete {value} for account: {account}"
            );
        }
    }
    assert_eq!(output.matches("showrunners").count(), 2);
    assert_eq!(output.matches("units").count(), 2);
    assert_eq!(
        output.matches("yes").count(),
        0,
        "enabled drops before counts wrap"
    );
    assert!(!lines[1].contains("enabled"));
    assert!(
        lines
            .iter()
            .all(|line| line.chars().count() <= usize::from(COMPLETE_ACCOUNTS_WIDTH))
    );
}

#[test]
fn complete_five_count_summary_keeps_every_account_value_whole() {
    let table = narrow_accounts();
    for width in [99, COMPLETE_ACCOUNTS_WIDTH] {
        let buffer = narrow_draw(&table, width, 40);
        if width == 99 {
            let lines = text(&buffer);
            let output = lines.join("\n");
            for value in [
                "first",
                "second",
                "showrunners",
                "units",
                "60%",
                "24%",
                "busy",
                "starting",
                "waiting",
                "idle",
                "paused",
                "exited",
                "total",
            ] {
                assert!(output.contains(value), "missing fitted {value}: {lines:#?}");
            }
            for omitted in ["credits", "enabled", "13,446", "unlimited"] {
                assert!(
                    !output.contains(omitted),
                    "optional {omitted} must leave first: {lines:#?}"
                );
            }
            for instant in ["2026-10-17T14:00:00Z", "2026-10-18T14:00:00Z"] {
                let reset = DateTime::parse_from_rfc3339(instant)
                    .expect("reset fixture")
                    .with_timezone(&Local)
                    .format("%a %H:%M")
                    .to_string();
                assert!(
                    output.contains(&reset),
                    "missing fitted reset {reset}: {lines:#?}"
                );
            }
            assert_eq!(output.matches("showrunners").count(), 2);
            assert_eq!(output.matches("units").count(), 2);
            assert!(
                lines
                    .iter()
                    .all(|line| line.chars().count() <= usize::from(width))
            );
        } else {
            assert_complete_account_values(&buffer);
        }
    }
}

#[test]
fn two_line_accounts_align_resets_after_optional_columns_drop() {
    let table = narrow_accounts();
    // Roles may now shrink to six cells before the kept columns wrap.
    let buffer = narrow_draw(&table, 79, 40);
    let lines = text(&buffer);
    let (_, header_y) = at(&buffer, "role");
    let (reset_x, reset_y) = at(&buffer, "resets");
    assert_eq!(reset_y, header_y + 1, "two header lines: {lines:#?}");
    for omitted in ["credits", "enabled"] {
        assert!(
            !lines.iter().any(|line| line.contains(omitted)),
            "{lines:#?}"
        );
    }
    for (label, instant) in [
        ("first", "2026-10-17T14:00:00Z"),
        ("second", "2026-10-18T14:00:00Z"),
    ] {
        let reset = DateTime::parse_from_rfc3339(instant)
            .expect("reset fixture")
            .with_timezone(&Local)
            .format("%a %H:%M")
            .to_string();
        let (x, y) = at(&buffer, &reset);
        assert_eq!(
            y,
            at(&buffer, label).1 + 2,
            "{label} reset follows both role rows"
        );
        assert_eq!(x + cell_width(&reset), reset_x + cell_width("resets"));
    }
}

#[test]
fn stacked_accounts_wrap_whole_facts_with_board_words_and_separators() {
    let table = narrow_accounts();
    // The first table half no longer fits even with the six-cell roles floor.
    let width = 23;
    let buffer = narrow_draw(&table, width, 40);
    let lines = text(&buffer);
    let first = usize::from(at(&buffer, "first").1);
    let second = usize::from(at(&buffer, "second").1);
    assert!(lines[first].contains("● first showr…"), "{lines:#?}");
    let facts: Vec<_> = lines[first + 1..second]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let reset = DateTime::parse_from_rfc3339("2026-10-17T14:00:00Z")
        .expect("reset fixture")
        .with_timezone(&Local)
        .format("%a %H:%M")
        .to_string();
    let expected = [
        "60% left".to_owned(),
        format!("resets {reset}"),
        "units".to_owned(),
    ];
    let actual: Vec<_> = facts
        .iter()
        .flat_map(|line| line.trim().split(" · "))
        .collect();
    assert_eq!(
        actual, expected,
        "facts remain intact and ordered: {lines:#?}"
    );
    for line in &facts {
        if line.trim() == "units" {
            assert_eq!(
                line.find("units"),
                Some(1),
                "the role has a separate heading"
            );
        } else {
            assert!(line.starts_with("  "), "indented fact: {line}");
        }
    }
    assert!(
        facts
            .iter()
            .all(|line| !line.trim_start().starts_with([',', ':', '·']))
    );
    for omitted in ["credits", "enabled"] {
        assert!(
            !lines.iter().any(|line| line.contains(omitted)),
            "{lines:#?}"
        );
    }
    assert!(
        lines
            .iter()
            .all(|line| line.chars().count() <= usize::from(width))
    );
    assert!(
        lines[second + 1..]
            .iter()
            .any(|line| line.contains("24% left")),
        "final account keeps its mandatory facts"
    );
}

#[test]
fn missing_roles_and_resets_leave_blank_cells_but_missing_left_keeps_its_mark() {
    let mut row = contract_row("empty", &Value::Null, &Value::Null, 0);
    row["roles"] = serde_json::json!([]);
    row["resets_at"] = Value::Null;
    row["read_at"] = Value::Null;
    let buffer = contract_draw(&contract_table(&[row]), tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    let y = at(&buffer, "empty").1;
    assert_eq!(
        contract_cell(&buffer, y, &ranges[AccountsColumn::Roles.index()]),
        "",
        "no role placeholder"
    );
    assert_eq!(
        contract_cell(&buffer, y, &ranges[AccountsColumn::Resets.index()]),
        "",
        "no reset placeholder"
    );
    assert_eq!(
        contract_cell(&buffer, y, &ranges[AccountsColumn::WeeklyLeft.index()]),
        "—",
        "missing quota keeps its mark"
    );
}

#[test]
fn past_resets_use_weekdays_within_six_days_and_dates_afterwards() {
    let now = u64::try_from(
        DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .expect("fixed now")
            .timestamp(),
    )
    .expect("positive time");
    let cases = [
        ("yesterday", "2026-10-15T14:00:00Z", "%a %H:%M"),
        ("last-week", "2026-10-09T14:00:00Z", "%d %b"),
    ];
    let table = contract_table(&cases.map(|(label, reset, _)| {
        let mut row = contract_row(label, &serde_json::json!(60), &Value::Null, 0);
        row["resets_at"] = serde_json::json!(reset);
        row
    }));
    let area = Rect::new(
        0,
        0,
        accounts::sorted_width(&table, now, AccountsSort::SourceOrder),
        8,
    );
    let mut buffer = Buffer::empty(area);
    accounts::draw_sorted(
        &mut buffer,
        area,
        &table,
        now,
        &FooterColors::default(),
        AccountsSort::SourceOrder,
    );
    let ranges = contract_ranges(&buffer);
    for (label, reset, format) in cases {
        let expected = DateTime::parse_from_rfc3339(reset)
            .expect("reset fixture")
            .with_timezone(&Local)
            .format(format)
            .to_string();
        assert_eq!(
            contract_cell(
                &buffer,
                at(&buffer, label).1,
                &ranges[AccountsColumn::Resets.index()]
            ),
            expected,
            "{label}"
        );
    }
}

#[test]
fn summary_height_matches_every_drawn_row_in_full_two_line_and_stacked_layouts() {
    let table = narrow_accounts();
    let full = accounts::sorted_width(&table, NOW, AccountsSort::SourceOrder);
    let stacked = 23;
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let mut heights = Vec::new();
    for width in [full, 80, 60, stacked] {
        let generous = narrow_draw(&table, width, 40);
        let lines = text(&generous);
        let painted_height = lines
            .iter()
            .rposition(|line| !line.trim().is_empty())
            .expect("account rows")
            + 1;
        let measured = height_at_width(
            &machines,
            &placement,
            &table,
            NOW,
            AccountsSort::SourceOrder,
            width,
        );
        assert_eq!(measured, painted_height, "width {width}: {lines:#?}");
        assert_eq!(
            accounts::height_at_width(&table, NOW, AccountsSort::SourceOrder, width),
            painted_height
        );
        let exact = narrow_draw(
            &table,
            width,
            u16::try_from(measured).expect("small summary"),
        );
        assert_eq!(
            text(&exact),
            lines[..painted_height],
            "exact demand retains the final row at width {width}"
        );
        assert!(
            text(&exact).iter().any(|line| line.contains("second")),
            "final account survives measurement"
        );
        heights.push(measured);
    }
    assert!(
        heights.windows(2).all(|pair| pair[0] <= pair[1])
            && heights[0] < *heights.last().expect("layout heights"),
        "narrower layouts require more measured rows: {heights:?}"
    );
}

#[test]
fn stacked_accounts_omit_unavailable_facts_and_disabled_credit_flags() {
    let mut row = contract_row("empty", &Value::Null, &Value::Null, 0);
    row["roles"] = serde_json::json!([]);
    row["resets_at"] = Value::Null;
    row["read_at"] = Value::Null;
    row["use_credits"] = serde_json::json!(false);
    let table = contract_table(&[row]);
    let width = u16::try_from("  credits 13,446".len()).expect("short fact width");
    let buffer = narrow_draw(&table, width, 20);
    let lines: Vec<_> = text(&buffer)
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines.iter().map(|line| line.trim()).collect::<Vec<_>>(),
        [
            "accounts",
            "● empty",
            "— left",
            "total",
            "busy 0",
            "starting 0",
            "waiting 0",
            "idle 0",
            "paused 0",
            "exited 0",
            "total 0"
        ]
    );
}

const COMPACT_ACCOUNT_LABELS: [&str; 4] = ["claude1", "claude2", "codex1", "codex2"];
const COMPACT_ACCOUNT_ERROR: &str = "claude1: scratch reading failed";

const LIVE_ACCOUNT_LABELS: [&str; 7] = [
    "claude1", "claude2", "claude3", "claude4", "claude5", "codex1", "codex2",
];

fn live_accounts_grid() -> App {
    let mut app = crate::app::App::new_for_test().expect("live account app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("six tiles in one grid column");
    app.loaded_config.config.tiles.widen_summary = false;
    let rows = (1..=10)
        .map(|pid| {
            // Busy agent tiles compete for rows with the complete summary.
            let commands = (0..8)
                .map(|index| {
                    serde_json::json!({
                        "depth":0, "kind":"shell", "pid":1000+pid*10+index,
                        "name":format!("command-{index}"), "started":100
                    })
                })
                .collect::<Vec<_>>();
            serde_json::from_value(serde_json::json!({
                "agent":"claude", "pid":pid, "name":format!("session-{pid}"),
                "session_id":format!("session-{pid}"), "started":100,
                "directory":"/fixture", "children":commands
            }))
            .expect("live session")
        })
        .collect();
    let hierarchy = (1..=9)
        .step_by(2)
        .map(|pid| {
            serde_json::json!({
                "manager":format!("session-{pid}"),
                "member":format!("session-{}", pid+1), "role":"unit"
            })
        })
        .collect::<Vec<_>>();
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges(
            serde_json::from_value(serde_json::json!(hierarchy)).expect("five managed trees"),
        )
        .into(),
        stage: ScanStage::Completed,
    });
    let rows = LIVE_ACCOUNT_LABELS.map(|label| {
        let mut row = contract_row(label, &serde_json::json!(60), &serde_json::json!(13_446), 0);
        row["roles"] = if label == "claude1" {
            serde_json::json!(["showrunners", "units"])
        } else {
            serde_json::json!(["workers"])
        };
        row["resets_at"] = serde_json::json!("2026-12-01T10:00:00Z");
        row
    });
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({
                "accounts":rows, "problems":[COMPACT_ACCOUNT_ERROR]
            }))
            .expect("live account JSON"),
        )
        .expect("live account wire"),
    );
    app
}

fn settled_live_summary(app: &mut App, width: u16) -> (Buffer, Rect, usize) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 50))
        .expect("live terminal");
    let keymap = app.keymap.clone();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, NOW))
        .expect("live first draw");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| render::draw_at_for_test(frame, app, &keymap, NOW))
        .expect("live settled draw");
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let summary = placements
        .iter()
        .find(|tile| matches!(tile.content, crate::tiles::TileContent::Summary))
        .expect("live summary")
        .frame
        .inner();
    let needed = match &*app.painted_summary.borrow() {
        PaintedSummary::Drawn(layout) => layout.height(),
        PaintedSummary::NotDrawn => panic!("live summary was not painted"),
    };
    assert_eq!(
        placements.len(),
        6,
        "one summary and five agent tiles; summary={summary:?}, needed={needed}"
    );
    let buffer = terminal.backend().buffer();
    let mut content = Buffer::empty(summary);
    for y in summary.y..summary.bottom() {
        for x in summary.x..summary.right() {
            content[(x, y)] = buffer[(x, y)].clone();
        }
    }
    (content, summary, needed)
}

fn assert_live_accounts_visible(buffer: &Buffer) -> Vec<usize> {
    let lines = text(buffer);
    let rows = LIVE_ACCOUNT_LABELS.map(|label| {
        lines
            .iter()
            .position(|line| line.contains(label))
            .unwrap_or_else(|| panic!("missing {label}, area={:?}: {lines:#?}", buffer.area))
    });
    assert!(
        lines.join("\n").contains(COMPACT_ACCOUNT_ERROR),
        "lost error: {lines:#?}"
    );
    rows.into()
}

#[test]
fn live_accounts_at_40_columns_keep_complete_role_groups_and_the_error_before_tiles() {
    let mut app = live_accounts_grid();
    let wide = text(&narrow_draw(&app.accounts.table, 200, 20)).join("\n");
    assert!(
        wide.contains("13,446"),
        "fixture has a whole grouped credit number"
    );
    let (buffer, area, needed) = settled_live_summary(&mut app, 40);
    assert!(
        usize::from(area.height) >= needed,
        "complete summary before busy tiles: {area:?}, needed={needed}: {:?}",
        text(&buffer)
    );
    let lines = text(&buffer);
    let rows = assert_live_accounts_visible(&buffer);
    let mut expected_row = rows[0];
    for (index, &row) in rows.iter().enumerate() {
        assert_eq!(row, expected_row, "consecutive account groups: {lines:#?}");
        let role_rows = if index == 0 { 3 } else { 2 };
        if index == 0 {
            assert!(lines[row + 2].contains("units"), "second role: {lines:#?}");
        } else {
            assert_eq!(
                lines[row].split_whitespace().nth(2),
                Some("worke…"),
                "the first role fits its six-column cell: {lines:#?}"
            );
        }
        expected_row = row + role_rows;
    }
    assert_eq!(
        lines[expected_row + 4].trim(),
        COMPACT_ACCOUNT_ERROR,
        "the error follows the final account without a gap: {lines:#?}"
    );
    assert!(
        // `AccountsColumn::COUNTS` adds three lines to the stacked total.
        needed == 29,
        "main budget: {area:?}, needed={needed}: {lines:#?}"
    );
    assert!(
        usize::from(area.height) >= needed,
        "all measured lines fit: {area:?}, needed={needed}"
    );
    assert!(
        lines[rows[0]].contains('…'),
        "cut roles have an ellipsis: {lines:#?}"
    );
    for row in rows {
        assert!(lines[row + 1].contains("60%"), "whole percent: {lines:#?}");
        assert!(
            lines[row + 1].contains("01 Dec"),
            "whole reset date: {lines:#?}"
        );
    }
    assert!(
        !lines.join("\n").contains("credits"),
        "credits drop before roles shorten"
    );
}

fn compact_accounts() -> AccountsTable {
    let rows = COMPACT_ACCOUNT_LABELS.map(|label| {
        let mut row = contract_row(
            label,
            &serde_json::json!(60),
            &serde_json::json!(13_446),
            7200,
        );
        row["resets_at"] = serde_json::json!("2026-10-17T14:00:00Z");

        if label == "codex2" {
            row["credits"] = serde_json::json!("unlimited");
        }
        row
    });
    AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({
                "accounts": rows, "problems": [COMPACT_ACCOUNT_ERROR]
            }))
            .expect("compact fixture JSON"),
        )
        .expect("compact accounts wire"),
    )
}

#[test]
fn compact_accounts_keep_role_rows_at_200_80_and_50_columns() {
    let table = compact_accounts();
    for width in [200, 80, 50] {
        let buffer = narrow_draw(&table, width, 30);
        let lines = text(&buffer);
        assert_eq!(
            accounts::height_at_width(&table, NOW, AccountsSort::SourceOrder, width),
            if width == 200 { 12 } else { 17 },
            "section, headers, eight role rows and error at {width}: {lines:#?}"
        );
        for (index, label) in COMPACT_ACCOUNT_LABELS.iter().enumerate() {
            let stride = if width == 200 { 2 } else { 3 };
            let first = match width {
                50 => 1,
                80 => 3,
                _ => 2,
            };
            let expected_row = first + index * stride;
            assert_eq!(
                usize::from(at(&buffer, label).1 - buffer.area.y),
                expected_row,
                "the account label starts its role group at {width}: {lines:#?}"
            );
            assert!(lines[expected_row].contains(label), "{width}: {lines:#?}");
        }
        assert_eq!(
            lines[if width == 200 { 11 } else { 16 }].trim(),
            COMPACT_ACCOUNT_ERROR,
            "{width}: {lines:#?}"
        );
        if width == 200 {
            assert_eq!(
                lines.iter().filter(|line| line.contains("13,446")).count(),
                3
            );
        }
    }
}

#[test]
fn compact_accounts_keep_every_account_error_and_whole_word_at_40_columns() {
    let table = compact_accounts();
    let buffer = narrow_draw(&table, 40, 24);
    let lines = text(&buffer);
    let output = lines.join("\n");
    for label in COMPACT_ACCOUNT_LABELS {
        assert!(output.contains(label), "missing {label}: {lines:#?}");
    }
    assert!(
        output.contains(COMPACT_ACCOUNT_ERROR),
        "lost error: {lines:#?}"
    );
    let wide = text(&narrow_draw(&table, 200, 19)).join(" ");
    let mut words: BTreeSet<_> = wide.split_whitespace().collect();
    words.extend(["left", "resets", "ago", "enabled", "in", "use", "·"]);
    for line in &lines {
        let line = line.trim();
        assert!(
            !line.starts_with([',', ':', '·']),
            "punctuation starts line: {lines:#?}"
        );
        for word in line.split_whitespace() {
            assert!(
                words.contains(word)
                    || word.strip_suffix('…').is_some_and(
                        |prefix| !prefix.is_empty() && "showrunners".starts_with(prefix)
                    ),
                "broken word or number {word:?}: {lines:#?}"
            );
        }
    }
}

#[test]
fn compact_accounts_drop_optional_columns_before_wrapping() {
    let table = compact_accounts();
    let optional = [AccountsColumn::Enabled, AccountsColumn::Credits];
    let mut seen = [false; 3];
    let mut previous_dropped = 0;
    for width in (50..=140).rev() {
        let headers = accounts::header_spans(
            Rect::new(0, 0, width, 19),
            &table,
            NOW,
            AccountsSort::SourceOrder,
        );
        if headers.is_empty() {
            let buffer = narrow_draw(&table, width, 30);
            let lines = text(&buffer);
            for label in COMPACT_ACCOUNT_LABELS {
                assert!(
                    lines.iter().any(|line| line.contains(label)),
                    "{width}: {lines:#?}"
                );
            }
            for word in ["left", "resets", "total"] {
                assert!(
                    lines.iter().any(|line| line.contains(word)),
                    "{width}: {lines:#?}"
                );
            }
            assert!(
                lines
                    .iter()
                    .all(|line| !line.contains("credits") && !line.contains("enabled"))
            );
            continue;
        }
        let dropped = optional
            .iter()
            .take_while(|column| !headers.iter().any(|header| header.column == **column))
            .count();
        for (index, column) in optional.iter().enumerate() {
            assert_eq!(
                headers.iter().any(|header| header.column == *column),
                index >= dropped,
                "drop order at {width}"
            );
        }
        assert!(dropped >= previous_dropped, "columns reappear at {width}");
        previous_dropped = dropped;
        seen[dropped] = true;
        for column in [
            AccountsColumn::Account,
            AccountsColumn::Roles,
            AccountsColumn::WeeklyLeft,
            AccountsColumn::Resets,
            AccountsColumn::Busy,
            AccountsColumn::Starting,
            AccountsColumn::Waiting,
            AccountsColumn::Idle,
            AccountsColumn::Paused,
            AccountsColumn::Exited,
            AccountsColumn::Total,
        ] {
            assert!(
                headers.iter().any(|header| header.column == column),
                "kept {column:?} at {width}"
            );
        }
    }
    assert_eq!(seen, [true; 3], "each optional drop has a measured width");
}

#[test]
fn compact_summary_marks_a_cut_machine_title_with_an_ellipsis() {
    let state = MachineState::Answered(agents(), HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    let colors = FooterColors::default();
    let table = AccountsTable::NotConfigured;
    let frame = SummaryFrame {
        accounts_sort:  AccountsSort::SourceOrder,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: AccountColumn::Hidden,
        footer_colors:  &colors,
        sort:           SummarySort::default(),
        accounts:       &table,
    };
    let area = Rect::new(0, 0, 8, 8);
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, area, &frame);
    let lines = text(&buffer);
    assert!(lines[0].ends_with('…'), "cut title: {lines:#?}");
}

#[test]
fn compact_accounts_wrap_error_words_without_leading_separators() {
    let problem = "claude1: scratch reading failed, all units need another attempt after reset";
    let table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({"accounts": [], "problems": [problem]}))
                .expect("word wrap JSON"),
        )
        .expect("word wrap wire"),
    );
    let lines = text(&narrow_draw(&table, 40, 19));
    let first = lines
        .iter()
        .position(|line| line.contains("claude1:"))
        .expect("error starts");
    let words: Vec<_> = lines[first..]
        .iter()
        .flat_map(|line| line.split_whitespace())
        .collect();
    assert_eq!(
        words,
        problem.split_whitespace().collect::<Vec<_>>(),
        "{lines:#?}"
    );
    for line in &lines[first..] {
        assert!(
            !line.trim_start().starts_with([',', ':', '·']),
            "{lines:#?}"
        );
    }
}

#[test]
fn narrow_review_machine_heading_never_cuts_the_count() {
    let template = agents().remove(0);
    let rows = (0..12)
        .map(|index| AgentRow {
            pid: 100 + index,
            ..template.clone()
        })
        .collect();
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "abcdef",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let frame = SummaryFrame {
        machines:       &machines,
        placement:      &placement,
        cells:          &[],
        now:            NOW,
        account_column: AccountColumn::Hidden,
        footer_colors:  &FooterColors::default(),
        sort:           SummarySort::default(),
        accounts:       &AccountsTable::NotConfigured,
        accounts_sort:  AccountsSort::SourceOrder,
    };
    let machine = SummaryMachines::new(&frame, 12);
    let text: String = machine.lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        !text.contains("1…"),
        "a count must show whole or drop whole: {text:?}"
    );
}

fn counted_account(label: &str, roles: &[&str]) -> Value {
    let mut account = usage(label, roles, &serde_json::json!(50), &Value::Null, false, 0);
    account["role_counts"] = serde_json::json!(roles.iter().enumerate().map(|(index, _)| {
        serde_json::json!({"busy":index + 1,"idle":index + 2,"paused":0,"exited":0,"total":index + 3})
    }).collect::<Vec<_>>());
    account
}

#[test]
fn accounts_total_row_sums_every_recorded_board_state() {
    let mut claude = usage(
        "claude1",
        &["units", "workers"],
        &serde_json::json!(50),
        &Value::Null,
        false,
        0,
    );
    claude["role_counts"] = serde_json::json!([
        {"busy":2,"idle":4,"paused":3,"exited":0,"total":9},
        {"busy":1,"idle":5,"paused":0,"exited":0,"total":6}
    ]);
    let mut codex = usage(
        "codex2",
        &["workers"],
        &serde_json::json!(50),
        &Value::Null,
        false,
        0,
    );
    codex["role_counts"] = serde_json::json!([
        {"busy":7,"idle":1,"paused":0,"exited":0,"total":8}
    ]);
    for (rows, expected) in [
        (
            vec![claude, codex.clone()],
            ["10", "0", "0", "10", "3", "0", "23"],
        ),
        (vec![codex], ["7", "0", "0", "1", "0", "0", "8"]),
    ] {
        let table = contract_table(&rows);
        let buffer = contract_draw(&table, tui_pane::pane_background(false));
        let ranges = contract_ranges(&buffer);
        let lines = accounts::build_lines(
            &table,
            NOW,
            &FooterColors::default(),
            AccountsSort::SourceOrder,
            200,
        );
        let row = buffer.area.y
            + u16::try_from(lines.selections.last().expect("account").1.end()).expect("total row");
        assert_eq!(
            contract_cell(&buffer, row, &ranges[AccountsColumn::Account.index()]),
            "total"
        );
        assert!(
            buffer[(ranges[AccountsColumn::Account.index()].start, row)]
                .modifier
                .contains(Modifier::BOLD)
        );
        for (column, value) in AccountsColumn::COUNTS.into_iter().zip(expected) {
            assert_eq!(contract_cell(&buffer, row, &ranges[column.index()]), value);
        }
        for column in [1, 2, 10, 11, 12] {
            assert_eq!(contract_cell(&buffer, row, &ranges[column]), "");
        }
    }
}

#[test]
fn accounts_total_row_stays_after_sorted_accounts_and_cannot_be_selected() {
    let table = contract_table(&[
        counted_account("zeta", &["workers"]),
        counted_account("alpha", &["units", "workers"]),
    ]);
    let mut sort = AccountsSort::SourceOrder;
    sort.select(AccountsColumn::Account);
    for expected in [["alpha", "zeta"], ["zeta", "alpha"]] {
        let lines = accounts::build_lines(&table, NOW, &FooterColors::default(), sort, 200);
        assert_eq!(
            lines
                .selections
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let total_row = lines.lines.len() - 1;
        assert!(
            lines.lines[total_row]
                .to_string()
                .trim_start()
                .starts_with("total")
        );
        assert!(
            lines
                .selections
                .iter()
                .all(|(_, span)| span.end() <= total_row)
        );
        sort.select(AccountsColumn::Account);
    }
}

#[test]
fn accounts_board_count_cells_draw_source_values() {
    let table = contract_table(&[counted_account("codex2", &["workers"])]);
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    let row = at(&buffer, "codex2").1;
    for (column, expected) in [
        (3, "1"),
        (4, "0"),
        (5, "0"),
        (6, "2"),
        (7, "0"),
        (8, "0"),
        (9, "3"),
    ] {
        assert_eq!(contract_cell(&buffer, row, &ranges[column]), expected);
    }
    assert_eq!(
        accounts::height_at_width(&table, NOW, AccountsSort::SourceOrder, 200),
        4
    );
}

#[test]
fn accounts_role_rows_draw_only_each_roles_counts_and_share_account_selection() {
    let table = contract_table(&[counted_account(
        "claude1",
        &["units", "showrunners", "workers"],
    )]);
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    let first = at(&buffer, "claude1").1;
    for (index, role) in ["units", "showrunners", "workers"].into_iter().enumerate() {
        let row = first + u16::try_from(index).expect("role row");
        assert_eq!(
            contract_cell(&buffer, row, &ranges[AccountsColumn::Roles.index()]),
            role
        );
        assert_eq!(
            contract_cell(&buffer, row, &ranges[AccountsColumn::Busy.index()]),
            (index + 1).to_string()
        );
        assert_eq!(
            contract_cell(&buffer, row, &ranges[AccountsColumn::Total.index()]),
            (index + 3).to_string()
        );
        if index > 0 {
            for column in [0, 1, 10, 11, 12] {
                assert_eq!(contract_cell(&buffer, row, &ranges[column]), "");
            }
        }
    }
    let lines = accounts::build_lines(
        &table,
        NOW,
        &FooterColors::default(),
        AccountsSort::SourceOrder,
        200,
    );
    assert_eq!(lines.selections.len(), 1);
    assert_eq!(lines.selections[0].0, "claude1");
    assert_eq!(
        lines.selections[0].1.end() - lines.selections[0].1.start(),
        3
    );
}

#[test]
fn accounts_refresh_removes_a_departed_role_row() {
    let mut table = contract_table(&[counted_account(
        "claude1",
        &["units", "showrunners", "workers"],
    )]);
    let replacement = contract_table(&[counted_account("claude1", &["units", "workers"])]);
    let AccountsTable::Ready(reading) = replacement else {
        panic!("ready fixture");
    };
    assert!(table.apply(Ok(reading)));
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    assert!(!text(&buffer).join(" ").contains("showrunners"));
    assert_eq!(at(&buffer, "workers").1, at(&buffer, "units").1 + 1);
    assert_eq!(
        accounts::height_at_width(&table, NOW, AccountsSort::SourceOrder, 200),
        5
    );
}

#[test]
fn accounts_count_sort_keeps_role_rows_under_their_account() {
    let mut rows = [
        counted_account("three", &["units", "showrunners", "workers"]),
        counted_account("one", &["spare"]),
    ];
    for row in &mut rows {
        for counts in row["role_counts"]
            .as_array_mut()
            .expect("role count fixtures")
        {
            counts["paused"] = serde_json::json!(1);
            counts["exited"] = serde_json::json!(1);
        }
    }
    let table = contract_table(&rows);
    for column in [
        AccountsColumn::Busy,
        AccountsColumn::Idle,
        AccountsColumn::Paused,
        AccountsColumn::Exited,
        AccountsColumn::Total,
    ] {
        let mut sort = AccountsSort::SourceOrder;
        sort.select(column);
        for expected in [["one", "three"], ["three", "one"]] {
            let lines = accounts::build_lines(&table, NOW, &FooterColors::default(), sort, 200);
            assert_eq!(
                lines
                    .selections
                    .iter()
                    .map(|(label, _)| label.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            let span = lines
                .selections
                .iter()
                .find(|(label, _)| label == "three")
                .expect("three roles")
                .1;
            assert_eq!(span.end() - span.start(), 3);
            assert!(lines.lines[span.start()].to_string().contains("units"));
            assert!(
                lines.lines[span.start() + 1]
                    .to_string()
                    .contains("showrunners")
            );
            assert!(
                lines.lines[span.start() + 2]
                    .to_string()
                    .contains("workers")
            );
            sort.select(column);
        }
    }
}

#[test]
fn count_sort_and_total_skip_unknown_roles_and_preserve_board_states() {
    let mut mixed = counted_account("mixed", &["units", "workers"]);
    mixed["role_counts"] = serde_json::json!([
        {"busy":2,"idle":4,"paused":3,"exited":0,"total":9},null
    ]);
    let unknown = usage(
        "unknown",
        &["workers"],
        &Value::Null,
        &Value::Null,
        false,
        0,
    );
    let table = contract_table(&[mixed, unknown]);
    for column in AccountsColumn::COUNTS {
        let mut sort = AccountsSort::SourceOrder;
        sort.select(column);
        let lines = accounts::build_lines(&table, NOW, &FooterColors::default(), sort, 200);
        let expected = if matches!(
            column,
            AccountsColumn::Starting | AccountsColumn::Waiting | AccountsColumn::Exited
        ) {
            ["mixed", "unknown"]
        } else {
            ["unknown", "mixed"]
        };
        assert_eq!(
            lines
                .selections
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{column:?}"
        );
        let area = Rect::new(0, 0, 200, 12);
        let mut buffer = Buffer::empty(area);
        accounts::draw_sorted(
            &mut buffer,
            area,
            &table,
            NOW,
            &FooterColors::default(),
            sort,
        );
        assert!(
            at(&buffer, expected[0]).1 < at(&buffer, expected[1]).1,
            "rendered count order"
        );
    }
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let ranges = contract_ranges(&buffer);
    let total = at(&buffer, "total").1;
    let row = (total + 1..buffer.area.bottom())
        .find(|&y| contract_cell(&buffer, y, &ranges[AccountsColumn::Account.index()]) == "total")
        .expect("total row");
    for (column, expected) in AccountsColumn::COUNTS
        .into_iter()
        .zip(["2", "0", "0", "4", "3", "0", "9"])
    {
        assert_eq!(
            contract_cell(&buffer, row, &ranges[column.index()]),
            expected
        );
    }
}

#[test]
fn accounts_without_roles_keep_one_selected_row_before_the_next_account() {
    let table = contract_table(&[
        counted_account("empty", &[]),
        counted_account("roles", &["units", "workers"]),
    ]);
    let lines = accounts::build_lines(
        &table,
        NOW,
        &FooterColors::default(),
        AccountsSort::SourceOrder,
        200,
    );
    let buffer = contract_draw(&table, tui_pane::pane_background(false));
    let first = at(&buffer, "empty").1;
    let next = at(&buffer, "roles").1;
    assert_eq!(next, first + 1);
    assert_eq!(lines.selections[0].1.start(), usize::from(first));
    assert_eq!(lines.selections[0].1.end(), usize::from(next));
    assert_eq!(
        lines.selections[1].1.end() - lines.selections[1].1.start(),
        2
    );
    let ranges = contract_ranges(&buffer);
    assert_eq!(
        contract_cell(&buffer, next + 1, &ranges[AccountsColumn::Roles.index()]),
        "workers"
    );
}

#[test]
fn two_line_accounts_keep_role_counts_before_one_account_detail_line() {
    let rows = ["alpha", "beta"].map(|label| {
        let mut row = counted_account(label, &["units", "spare"]);
        row["role_counts"] = serde_json::json!([
            {"busy":1,"idle":3,"paused":2,"exited":0,"total":6},
            {"busy":4,"idle":6,"paused":5,"exited":0,"total":15}
        ]);
        row
    });
    let table = contract_table(&rows);
    for width in [75, 83] {
        let buffer = narrow_draw(&table, width, 16);
        let lines = text(&buffer);
        let reset = DateTime::parse_from_rfc3339(RESET)
            .expect("reset")
            .with_timezone(&Local)
            .format("%a %H:%M")
            .to_string();
        if width == 75 {
            let built = accounts::build_lines(
                &table,
                NOW,
                &FooterColors::default(),
                AccountsSort::SourceOrder,
                width,
            );
            assert_eq!(
                built.headers.len(),
                0,
                "stacked facts have no column headers"
            );
            for (label, (_, span)) in ["alpha", "beta"].into_iter().zip(&built.selections) {
                assert_eq!(span.start(), usize::from(at(&buffer, label).1));
                let account = lines[span.start()..span.end()].join(" ");
                for fact in [
                    "50% left", "busy 1", "idle 3", "paused 2", "exited 0", "total 6", "busy 4",
                    "idle 6", "paused 5", "total 15", "units", "spare", &reset,
                ] {
                    assert!(
                        account.contains(fact),
                        "missing stacked {fact} for {label}: {account}"
                    );
                }
                assert!(
                    span.end() - span.start() > 3,
                    "stacked facts remain in the account selection"
                );
            }
            assert_eq!(lines.iter().filter(|line| line.contains(&reset)).count(), 2);
            continue;
        }
        assert_eq!(at(&buffer, "busy").1, at(&buffer, "role").1);
        assert_eq!(at(&buffer, "resets").1, at(&buffer, "role").1 + 1);
        for (index, label) in ["alpha", "beta"].into_iter().enumerate() {
            let first = usize::from(at(&buffer, label).1);
            assert_eq!(first, 3 + index * 3, "{lines:#?}");
            assert_eq!(
                lines[first].split_whitespace().collect::<Vec<_>>(),
                [
                    "●", label, "50%", "units", "1", "0", "0", "3", "2", "0", "6"
                ]
            );
            assert_eq!(
                lines[first + 1].split_whitespace().collect::<Vec<_>>(),
                ["spare", "4", "0", "0", "6", "5", "0", "15"]
            );
            assert_eq!(lines[first + 2].trim(), reset);
        }
        assert_eq!(lines.iter().filter(|line| line.contains(&reset)).count(), 2);
        let built = accounts::build_lines(
            &table,
            NOW,
            &FooterColors::default(),
            AccountsSort::SourceOrder,
            width,
        );
        for (index, (_, span)) in built.selections.iter().enumerate() {
            assert_eq!((span.start(), span.end()), (3 + index * 3, 6 + index * 3));
        }
    }
}
