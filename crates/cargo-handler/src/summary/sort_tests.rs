//! Summary sorting stays inside machine groups and keeps each name's hue.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and rendered labels must exist"
)]
#![expect(clippy::panic, reason = "missing fixture labels should fail the test")]

use ratatui::style::Color;
use tui_pane::CellPoint;

use super::*;
use crate::census::Account;
use crate::census::AgentActivity;
use crate::census::HierarchyEdges;
use crate::census::ServiceTier;
use crate::census::accounts::AccountsTable;
use crate::sort;
use crate::sort::AccountsSort;
use crate::sort::HeaderHit;
use crate::sort::SortDirection;
use crate::sort::SummaryColumn;
use crate::sort::SummarySort;
use crate::theme::RainbowHue;

const NOW: u64 = 1_000_000;
const COLUMNS: [SummaryColumn; 7] = [
    SummaryColumn::Pid,
    SummaryColumn::Agent,
    SummaryColumn::Name,
    SummaryColumn::Status,
    SummaryColumn::Age,
    SummaryColumn::Desktop,
    SummaryColumn::Directory,
];

fn sort_row(
    pid: u32,
    name: &str,
    status: &str,
    started: u64,
    account: &str,
    desktop: &str,
    directory: &str,
) -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"claude","name":name,"status":status,"started":started,"process_started":started,
        "pid":pid,"desktop":desktop,"directory":directory,"branch":null,"launched_by":null,"children":[{"depth":0,"kind":"shell","pid":9999,"name":"fixture command","started":100}],
        "account":{"home":format!("~/.{account}"),"label":account}
    })).expect("summary row fixture")
}

fn sort_states() -> [MachineState; 2] {
    [
        MachineState::Answered(
            vec![
                sort_row(
                    100,
                    "zeta",
                    "idle",
                    NOW - 7200,
                    "z-account",
                    "z-desktop",
                    "/z-directory",
                ),
                sort_row(
                    20,
                    "alpha",
                    "git",
                    NOW - 3600,
                    "a-account",
                    "a-desktop",
                    "/a-directory",
                ),
            ],
            HierarchyEdges::default().into(),
        ),
        MachineState::Answered(
            vec![sort_row(
                1,
                "other-machine",
                "git",
                NOW - 8000,
                "a-account",
                "a-desktop",
                "/a-directory",
            )],
            HierarchyEdges::default().into(),
        ),
    ]
}

fn sort_machines(states: &[MachineState; 2]) -> [Machine<'_>; 2] {
    [
        Machine {
            name:  "natedev",
            state: &states[0],
        },
        Machine {
            name:  "mac",
            state: &states[1],
        },
    ]
}

fn sorted_buffer(machines: &[Machine<'_>], sort: SummarySort) -> Buffer {
    let placement = AgentPlacement::from_machines(machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 180, 20);
    let mut buffer = Buffer::empty(area);
    let footer_colors = FooterColors::parse("account.a-account=orange\naccount.z-account=purple");
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            accounts_sort: AccountsSort::SourceOrder,
            accounts: &AccountsTable::NotConfigured,
            machines,
            placement: &placement,
            cells: &cells,
            now: NOW,
            account_column: AccountColumn::Shown,
            footer_colors: &footer_colors,
            sort,
        },
    );
    buffer
}

fn find_label(buffer: &Buffer, label: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let text: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(start) = text.find(label) {
            return (
                u16::try_from(text[..start].chars().count()).expect("column fits"),
                y,
            );
        }
    }
    panic!("missing label {label}");
}

fn order(buffer: &Buffer) -> Vec<&'static str> {
    let mut names = ["zeta", "alpha", "other-machine"];
    names.sort_by_key(|name| find_label(buffer, name).1);
    names.to_vec()
}

#[test]
fn default_summary_sort_keeps_phase_one_machine_order_rows_and_colors() {
    let states = sort_states();
    let machines = sort_machines(&states);
    let buffer = sorted_buffer(&machines, SummarySort::default());
    assert_eq!(order(&buffer), ["zeta", "alpha", "other-machine"]);
    for (name, expected) in [
        ("zeta", "100 z-account zeta idle 2h z-desktop /z-directory"),
        ("alpha", "20 a-account alpha git 1h a-desktop /a-directory"),
    ] {
        let y = find_label(&buffer, name).1;
        let text: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>().join(" "),
            expected
        );
    }

    for (name, hue) in [
        ("zeta", RainbowHue::Red),
        ("alpha", RainbowHue::Orange),
        ("other-machine", RainbowHue::Green),
    ] {
        assert_eq!(
            buffer[find_label(&buffer, name)].fg,
            Role::Rainbow(hue).style().fg.expect("hue foreground")
        );
    }
}

#[test]
fn every_summary_column_sorts_both_directions_inside_its_machine_group() {
    let states = sort_states();
    let machines = sort_machines(&states);
    let baseline = sorted_buffer(&machines, SummarySort::default());
    for column in COLUMNS {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let buffer = sorted_buffer(&machines, SummarySort { column, direction });
            let expected = if direction == SortDirection::Ascending {
                ["alpha", "zeta", "other-machine"]
            } else {
                ["zeta", "alpha", "other-machine"]
            };
            assert_eq!(order(&buffer), expected, "{column:?} {direction:?}");
            assert!(find_label(&buffer, "mac ·").1 > find_label(&buffer, expected[1]).1);
            for name in ["zeta", "alpha", "other-machine"] {
                assert_eq!(
                    buffer[find_label(&buffer, name)].fg,
                    baseline[find_label(&baseline, name)].fg,
                    "{name}"
                );
            }
            for (account, color) in [
                ("a-account", Color::Indexed(208)),
                ("z-account", Color::Indexed(141)),
            ] {
                assert_eq!(buffer[find_label(&buffer, account)].fg, color);
            }
        }
    }
}

#[test]
fn equal_summary_keys_use_default_age_and_pid_order_even_when_reversed() {
    let mut states = sort_states();
    if let MachineState::Answered(rows, _) = &mut states[0] {
        for row in rows {
            row.activity = AgentActivity::from(Some("git"));
            row.account = Account::Unrecorded;
        }
    }
    let machines = sort_machines(&states);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let buffer = sorted_buffer(
            &machines,
            SummarySort {
                column: SummaryColumn::Status,
                direction,
            },
        );
        assert_eq!(order(&buffer), ["zeta", "alpha", "other-machine"]);
    }
}

#[test]
fn summary_header_clicks_work_for_every_machine_and_ignore_gaps_and_rows() {
    let states = sort_states();
    let machines = sort_machines(&states);
    let placement = AgentPlacement::from_machines(&machines);
    let sort = SummarySort::default();
    let buffer = sorted_buffer(&machines, sort);
    let cells = agent_cell::cell_order_placed(&placement);
    let colors = FooterColors::default();
    let frame = SummaryFrame {
        accounts_sort: AccountsSort::SourceOrder,
        accounts: &AccountsTable::NotConfigured,
        machines: &machines,
        placement: &placement,
        cells: &cells,
        now: NOW,
        account_column: AccountColumn::Shown,
        footer_colors: &colors,
        sort,
    };
    let spans = header_spans(buffer.area, &frame);
    let mut header_rows: Vec<_> = spans.iter().map(|span| span.row).collect();
    header_rows.sort_unstable();
    header_rows.dedup();
    assert_eq!(header_rows.len(), 2);
    for span in &spans {
        assert_eq!(
            sort::header_hit(
                &spans,
                CellPoint {
                    column: span.columns.start,
                    row:    span.row,
                }
            ),
            HeaderHit::Column(span.column)
        );
        assert_eq!(
            sort::header_hit(
                &spans,
                CellPoint {
                    column: span.columns.start,
                    row:    span.row + 1,
                }
            ),
            HeaderHit::Miss
        );
        let label_x = span.columns.start;
        let label: String = (label_x..span.columns.end)
            .map(|x| buffer[(x, span.row)].symbol())
            .collect();
        assert!(label.contains(span.column.header()), "{label}");
    }
    for pair in spans.windows(2).filter(|pair| pair[0].row == pair[1].row) {
        if pair[0].columns.end < pair[1].columns.start {
            assert_eq!(
                sort::header_hit(
                    &spans,
                    CellPoint {
                        column: pair[0].columns.end,
                        row:    pair[0].row,
                    }
                ),
                HeaderHit::Miss
            );
        }
    }
}

#[test]
fn the_summary_arrow_marks_only_the_selected_column() {
    let states = sort_states();
    let machines = sort_machines(&states);
    for column in COLUMNS {
        for (direction, arrow) in [
            (SortDirection::Ascending, "▲"),
            (SortDirection::Descending, "▼"),
        ] {
            let buffer = sorted_buffer(&machines, SummarySort { column, direction });
            let (_, y) = find_label(&buffer, "pid");
            let header: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert_eq!(header.matches('▲').count() + header.matches('▼').count(), 1);
            assert!(
                header.contains(&format!("{} {arrow}", column.header())),
                "{header}"
            );
        }
    }
}

#[test]
fn choosing_a_summary_column_uses_its_natural_direction_and_repeated_clicks_reverse_it() {
    let mut sort = SummarySort::default();
    sort.select(SummaryColumn::Age);
    assert_eq!(sort.direction, SortDirection::Ascending);
    for column in COLUMNS
        .into_iter()
        .filter(|column| *column != SummaryColumn::Age)
    {
        sort.select(column);
        assert_eq!(
            sort,
            SummarySort {
                column,
                direction: SortDirection::Ascending
            }
        );
        sort.select(column);
        assert_eq!(
            sort,
            SummarySort {
                column,
                direction: SortDirection::Descending
            }
        );
    }
    sort.select(SummaryColumn::Age);
    assert_eq!(
        sort,
        SummarySort {
            column:    SummaryColumn::Age,
            direction: SortDirection::Descending,
        }
    );
}

#[test]
fn each_summary_column_has_an_independent_order_including_numeric_pids() {
    let states = [
        MachineState::Answered(
            vec![
                sort_row(
                    100,
                    "bee",
                    "idle",
                    NOW - 8 * 3600,
                    "c-account",
                    "b-desk",
                    "/d",
                ),
                sort_row(
                    20,
                    "ant",
                    "waiting",
                    NOW - 6 * 3600,
                    "d-account",
                    "d-desk",
                    "/b",
                ),
                sort_row(
                    300,
                    "dog",
                    "git",
                    NOW - 4 * 3600,
                    "a-account",
                    "c-desk",
                    "/a",
                ),
                sort_row(
                    40,
                    "cat",
                    "reading",
                    NOW - 2 * 3600,
                    "b-account",
                    "a-desk",
                    "/c",
                ),
            ],
            HierarchyEdges::default().into(),
        ),
        MachineState::Answered(Vec::new(), HierarchyEdges::default().into()),
    ];
    let machines = sort_machines(&states);
    for (column, ascending) in [
        (SummaryColumn::Pid, ["ant", "cat", "bee", "dog"]),
        (SummaryColumn::Agent, ["dog", "cat", "bee", "ant"]),
        (SummaryColumn::Name, ["ant", "bee", "cat", "dog"]),
        (SummaryColumn::Status, ["dog", "bee", "cat", "ant"]),
        (SummaryColumn::Age, ["cat", "dog", "ant", "bee"]),
        (SummaryColumn::Desktop, ["cat", "bee", "dog", "ant"]),
        (SummaryColumn::Directory, ["dog", "ant", "cat", "bee"]),
    ] {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let buffer = sorted_buffer(&machines, SummarySort { column, direction });
            let mut actual = ["bee", "ant", "dog", "cat"];
            actual.sort_by_key(|name| find_label(&buffer, name).1);
            let mut expected = ascending;
            if direction == SortDirection::Descending {
                expected.reverse();
            }
            assert_eq!(actual, expected, "{column:?} {direction:?}");
        }
    }
}

/// Agent ordering follows the full displayed label in either account mode.
#[test]
fn agent_sort_uses_displayed_labels_and_default_ties_in_both_account_modes() {
    let mut priority = sort_row(
        10,
        "priority",
        "idle",
        NOW - 300,
        "shared",
        "desk",
        "/fixture",
    );
    priority.service_tier = ServiceTier::Fast;
    let early = sort_row(
        20,
        "ordinary-early",
        "idle",
        NOW - 200,
        "shared",
        "desk",
        "/fixture",
    );
    let late = sort_row(
        30,
        "ordinary-late",
        "idle",
        NOW - 100,
        "shared",
        "desk",
        "/fixture",
    );
    let state = MachineState::Answered(
        vec![priority, early, late],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let colors = FooterColors::default();
    for accounts in [AccountColumn::Shown, AccountColumn::Hidden] {
        for (direction, expected) in [
            (
                SortDirection::Ascending,
                ["ordinary-early", "ordinary-late", "priority"],
            ),
            (
                SortDirection::Descending,
                ["priority", "ordinary-early", "ordinary-late"],
            ),
        ] {
            let area = Rect::new(0, 0, 160, 8);
            let mut buffer = Buffer::empty(area);
            draw(
                &mut buffer,
                area,
                &SummaryFrame {
                    accounts_sort:  AccountsSort::SourceOrder,
                    accounts:       &AccountsTable::NotConfigured,
                    machines:       &machines,
                    placement:      &placement,
                    cells:          &cells,
                    now:            NOW,
                    account_column: accounts,
                    footer_colors:  &colors,
                    sort:           SummarySort {
                        column: SummaryColumn::Agent,
                        direction,
                    },
                },
            );
            let mut actual = ["priority", "ordinary-early", "ordinary-late"];
            actual.sort_by_key(|name| find_label(&buffer, name).1);
            assert_eq!(actual, expected, "{accounts:?} {direction:?}");
            let label = match accounts {
                AccountColumn::Shown => "shared fast",
                AccountColumn::Hidden => "claude fast",
            };
            assert_eq!(
                find_label(&buffer, label).1,
                find_label(&buffer, "priority").1
            );
        }
    }
}

#[test]
fn summary_header_click_rows_follow_empty_failed_and_clipped_machine_groups() {
    let states = sort_states();
    let empty = MachineState::Answered(Vec::new(), HierarchyEdges::default().into());
    let failed = MachineState::Failed("fixture failure".into());
    let machines = [
        Machine {
            name:  "empty",
            state: &empty,
        },
        Machine {
            name:  "failed",
            state: &failed,
        },
        Machine {
            name:  "natedev",
            state: &states[0],
        },
        Machine {
            name:  "mac",
            state: &states[1],
        },
    ];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let colors = FooterColors::default();
    let frame = SummaryFrame {
        accounts_sort:  AccountsSort::SourceOrder,
        accounts:       &AccountsTable::NotConfigured,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            NOW,
        account_column: AccountColumn::Shown,
        footer_colors:  &colors,
        sort:           SummarySort::default(),
    };
    for height in [1, 4, 6, 7, 8, 10, 14] {
        let area = Rect::new(7, 3, 180, height);
        let mut buffer = Buffer::empty(area);
        draw(&mut buffer, area, &frame);
        let spans = header_spans(area, &frame);
        let header_rows: Vec<_> = (area.y..area.bottom())
            .filter(|&y| {
                let line: String = (area.x..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.contains("pid") && line.contains("age ▼")
            })
            .collect();
        let mut click_rows: Vec<_> = spans.iter().map(|span| span.row).collect();
        click_rows.sort_unstable();
        click_rows.dedup();
        assert_eq!(click_rows, header_rows, "height {height}");
        for span in spans {
            let label: String = span
                .columns
                .clone()
                .map(|x| buffer[(x, span.row)].symbol())
                .collect();
            assert!(
                label.contains(span.column.header()),
                "height {height}: {label}"
            );
        }
    }
}
