//! Every account role keeps its own configured foreground.

#![expect(clippy::expect_used, reason = "rendered fixture roles must exist")]

use chrono::DateTime;
use chrono::Utc;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;

use super::*;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::CreditUse;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::constants::ACCOUNTS_STALE_SECONDS;
use crate::footer_colors::FooterRole;
use crate::sort::AccountsSort;

const NOW: u64 = 1_792_166_400;

fn draw_account_roles(
    read_at: Reading<DateTime<Utc>>,
    colors: &FooterColors,
    area: Rect,
) -> Buffer {
    let table = AccountsTable::Ready(AccountsReading {
        accounts: vec![AccountUsage {
            label: "fixture-account".into(),
            roles: vec![
                AccountRole::Units,
                AccountRole::Showrunners,
                AccountRole::Workers,
                AccountRole::Spare,
            ],
            weekly_left_percent: Reading::Known(50.0),
            resets_at: Reading::Unavailable,
            credits: Credits::Amount(20.0),
            credit_use: CreditUse::Spending,
            read_at,
        }],
        problems: Vec::new(),
    });
    let machines = [];
    let placement = AgentPlacement::from_machines(&machines);
    let frame = SummaryFrame {
        machines:       &machines,
        placement:      &placement,
        cells:          &[],
        now:            NOW,
        account_column: AccountColumn::Hidden,
        footer_colors:  colors,
        sort:           SummarySort::default(),
        accounts:       &table,
        accounts_sort:  AccountsSort::SourceOrder,
    };
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, area, &frame);
    buffer
}

#[test]
fn account_role_spans_use_each_role_color_and_keep_spare_and_separators_plain() {
    let colors = FooterColors::parse(
        "role.showrunner=bright-magenta\nrole.unit-director=bright-blue\nrole.worker=bright-cyan",
    );
    for read_at in [
        Reading::Known(
            DateTime::from_timestamp(i64::try_from(NOW).expect("time"), 0).expect("date"),
        ),
        Reading::Known(
            DateTime::from_timestamp(
                i64::try_from(NOW - ACCOUNTS_STALE_SECONDS - 1).expect("time"),
                0,
            )
            .expect("date"),
        ),
        Reading::Unavailable,
    ] {
        let stale = read_at
            != Reading::Known(
                DateTime::from_timestamp(i64::try_from(NOW).expect("time"), 0).expect("date"),
            );
        let area = Rect::new(0, 0, 180, 12);
        let buffer = draw_account_roles(read_at, &colors, area);
        let row = (0..area.height)
            .find(|&y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("fixture-account")
            })
            .expect("account row");
        let line: String = (0..area.width).map(|x| buffer[(x, row)].symbol()).collect();
        for (word, role) in [
            ("units", FooterRole::UnitDirector),
            ("showrunners", FooterRole::Showrunner),
            ("workers", FooterRole::Worker),
        ] {
            let start = cell_width(&line[..line.find(word).expect("role word")]);
            let base = Style::default().fg(text_default());
            let expected = if stale {
                colors.role(role).subdued_style(base)
            } else {
                colors.role(role).style(base)
            };
            for offset in 0..cell_width(word) {
                assert_eq!(
                    buffer[(start + offset, row)].fg,
                    expected.fg.expect("role color"),
                    "{word}, stale={stale}"
                );
                assert!(
                    !buffer[(start + offset, row)]
                        .modifier
                        .contains(Modifier::DIM),
                    "role color has explicit subdued ink"
                );
            }
        }
        let spare = cell_width(&line[..line.find("spare").expect("spare")]);
        let separator = cell_width(
            &line[..line
                .find(constants::ACCOUNT_ROLE_SEPARATOR)
                .expect("role separator")],
        );
        for offset in 0..cell_width("spare") {
            assert_eq!(
                buffer[(spare + offset, row)].style(),
                buffer[(separator, row)].style()
            );
        }
        assert_eq!(buffer[(spare, row)].fg, text_default());
    }
}
