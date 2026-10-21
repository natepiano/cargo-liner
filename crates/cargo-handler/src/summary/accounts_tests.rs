//! Accounts retain source ordering, nullable readings and their drawn styles.

#![expect(
    clippy::expect_used,
    reason = "fixture readings and drawn labels must exist"
)]
#![expect(
    clippy::panic,
    reason = "a missing rendered account label fails the fixture"
)]

use chrono::DateTime;
use chrono::Local;
use ratatui::style::Color;
use serde_json::Value;

use super::*;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::HierarchyError;
use crate::footer_colors::FooterTint;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::footer_colors::tests;
use crate::sort::AccountsSort;
use crate::theme;

const NOW: u64 = 1_792_166_400;
const RESET: &str = "2026-10-19T06:00:00Z";

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
    let placement = AgentPlacement::from_machines(&machines);
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
    for header in ["account", "role", "weekly left", "resets", "credits"] {
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
    assert!(joined.contains("showrunners, units"), "{joined}");
    assert!(
        joined.contains("unlimited") && joined.contains("12.5"),
        "{joined}"
    );
    let reset = DateTime::parse_from_rfc3339(RESET)
        .expect("reset timestamp")
        .with_timezone(&Local)
        .format("%a %H:%M")
        .to_string();
    assert!(joined.contains(&reset), "local reset: {joined}");
    assert_eq!(
        lines.last().expect("problem row").trim(),
        "cached role map unavailable for fixture-host"
    );
    assert!(
        !joined.contains('@') && !joined.contains("login") && !joined.contains("on-credits"),
        "{joined}"
    );
    assert!(!joined.contains("/private/account-home"), "{joined}");
    assert_eq!(
        buffer[at(&buffer, "weekly left")].fg,
        buffer[at(&buffer, "status")].fg,
        "both tables share the header style"
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
    assert_eq!(buffer[at(&buffer, "codex2")].fg, Color::Indexed(208));
    assert_eq!(buffer[at(&buffer, "claude1")].fg, Color::LightCyan);
    assert_eq!(buffer[at(&buffer, "10%")].fg, Color::Red);
    assert_eq!(buffer[at(&buffer, "9%")].fg, Color::Red);
    assert_ne!(buffer[at(&buffer, "10.1%")].fg, Color::Red);
    assert!(
        buffer[at(&buffer, "unlimited")]
            .modifier
            .contains(Modifier::DIM)
    );
    assert!(!buffer[at(&buffer, "12.5")].modifier.contains(Modifier::DIM));
    assert!(
        !buffer[at(&buffer, "claude1")]
            .modifier
            .contains(Modifier::DIM),
        "exactly one hour is fresh"
    );
    let credits_x = at(&buffer, "credits").0 + 6;
    let claude_y = at(&buffer, "claude1").1;
    assert_eq!(buffer[(credits_x, claude_y)].symbol(), "—");
    assert!(
        buffer[(credits_x, claude_y)]
            .modifier
            .contains(Modifier::DIM)
    );
    let stale_y = at(&buffer, "codex1").1;
    let stale_line = &text(&buffer)[usize::from(stale_y - buffer.area.y)];
    assert!(stale_line.contains("2h"), "{stale_line}");
    let spare_x = at(&buffer, "spare").0;
    let spare_end = spare_x + cell_width("spare");
    let subdued = FooterTint::Plain.subdued_style(Style::default().fg(text_default()));
    for x in buffer.area.x..buffer.area.right() {
        if x >= at(&buffer, "role").0 && !buffer[(x, stale_y)].symbol().trim().is_empty() {
            if (spare_x..spare_end).contains(&x) {
                assert_eq!(
                    Some(buffer[(x, stale_y)].fg),
                    subdued.fg,
                    "stale plain role at {x}"
                );
                assert_eq!(buffer[(x, stale_y)].modifier, subdued.add_modifier);
                continue;
            }
            assert!(
                buffer[(x, stale_y)].modifier.contains(Modifier::DIM),
                "stale cell at {x}"
            );
        }
    }
    let missing_y = at(&buffer, "claude3").1;
    let missing_line = &text(&buffer)[usize::from(missing_y - buffer.area.y)];
    assert_eq!(
        missing_line.matches('—').count(),
        4,
        "roles, usage, reset, credits unavailable: {missing_line}"
    );
    assert!(
        !missing_line.contains("old"),
        "no fabricated age: {missing_line}"
    );
    assert!(
        !buffer[at(&buffer, "claude3")]
            .modifier
            .contains(Modifier::DIM)
    );
    assert!(
        buffer[at(&buffer, "cached role map")]
            .modifier
            .contains(Modifier::DIM)
    );
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
        assert!(stale_line.contains("2h old"), "{stale_line}");
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
    let ready = reading();
    let failure = AccountsFailure::Command(HierarchyError::ExitedUnsuccessfully);
    let mut table = AccountsTable::AwaitingFirstReading;
    assert_eq!(
        text(&drawn(&table)),
        text(&drawn(&AccountsTable::NotConfigured))
    );
    assert!(table.apply(Ok(ready.clone())));
    assert!(table.apply(Err(failure)));
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
                    joined.contains("codex2") && joined.contains("accounts: command failed"),
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
    let problem = "stored role cache unavailable: the producer could not locate the source's last known role map on the fixture machine";
    source.problems = vec![problem.to_owned()];
    let table = AccountsTable::FailedKeepingLast {
        last:    source,
        failure: AccountsFailure::InvalidReading,
    };
    let buffer = drawn(&table);
    let lines = text(&buffer);
    assert!(lines.iter().any(|line| line.contains(problem)), "{lines:?}");
    assert!(lines.last().expect("why line").contains("invalid JSON"));
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
    let column = at(&buffer, "credits").0 + 7 - 4;
    let held = buffer[(column, at(&buffer, "held").1)].clone();
    let spending = buffer[(column, at(&buffer, "spending").1)].clone();
    assert_eq!(held.symbol(), "1");
    assert_eq!(spending.symbol(), "1");
    assert!(held.modifier.contains(Modifier::DIM));
    assert!(!spending.modifier.contains(Modifier::DIM));
}

#[test]
fn accounts_only_summary_demand_includes_rows_and_all_notes() {
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines);
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
                assert_eq!(output.last().expect("problem").trim(), problem);
            },
            AccountsTable::FailedKeepingLast { .. } => {
                assert!(
                    joined.contains("claude3") && joined.contains(problem),
                    "{joined}"
                );
                assert_eq!(
                    output.last().expect("failure").trim(),
                    "accounts: invalid JSON reading"
                );
            },
            AccountsTable::Failed(_) => {
                assert_eq!(output.join("\n").trim(), "accounts: invalid JSON reading");
            },
            _ => panic!("fixture states must have visible content"),
        }
    }
}
