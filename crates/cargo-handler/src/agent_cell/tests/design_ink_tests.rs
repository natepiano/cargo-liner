//! Status and cut text are checked in rendered cells.

use std::sync::Arc;

use super::*;
use crate::census::Activity;
use crate::census::activity::RunLocation;
use crate::census::activity::VerifyStep;
use crate::census::transcript::SubagentRole;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::footer_colors::tests;
use crate::theme;

fn check_statuses(ground: Color) {
    let mut root = agent(1, "fixture", 60, None);
    let activities = [
        AgentActivity::Idle,
        AgentActivity::Active(Activity::Waiting),
        AgentActivity::Active(Activity::Thinking),
        AgentActivity::Active(Activity::Reading),
        AgentActivity::Active(Activity::Verify {
            step:     VerifyStep::Check,
            location: RunLocation::Local,
        }),
    ];
    root.children = activities
        .iter()
        .enumerate()
        .map(|(index, &activity)| {
            let mut row = child(
                0,
                ChildKind::Subagent(SubagentRole::Unrecorded),
                None,
                &format!("job-{index}"),
                10,
            );
            row.activity = activity;
            row
        })
        .collect();
    let cell = entry(&root, None);
    let rect = area(150, 15);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(ground));
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    for activity in activities {
        let word = activity.text();
        let (x, y) = find(&buffer, word).expect("drawn activity word");
        for offset in 0..summary::cell_width(word) {
            let cell = &buffer[(x + offset, y)];
            let ratio = tests::contrast_ratio(cell.fg, cell.bg);
            assert!(ratio >= MINIMUM_CONTRAST, "{word}: {cell:?} is {ratio}:1");
        }
    }
}

#[test]
fn status_words_meet_contrast_on_focused_and_unfocused_tiles_in_every_theme() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    tui_pane::set_focused_pane_tint(true);
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for transparency in [false, true] {
            tui_pane::set_transparent_background(transparency);
            for focused in [false, true] {
                check_statuses(tui_pane::pane_background(focused));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}

#[test]
fn drawn_cuts_remove_whitespace_before_the_truncation_mark() {
    for text in [
        "scripts into a long directory",
        "You are doing a longer job",
        "scripts into\t a longer directory",
    ] {
        let width = if text.starts_with("You") { 5 } else { 14 };
        let line = cut_line(
            vec![Span::styled(text, Style::default().fg(Color::Cyan))],
            width,
        );
        let rect = area(width, 1);
        let mut buffer = Buffer::empty(rect);
        Paragraph::new(line).render(rect, &mut buffer);
        let drawn: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(drawn.contains(TRUNCATION_MARK), "{drawn:?}");
        let before = drawn.split(TRUNCATION_MARK).next().expect("cut prefix");
        assert!(!before.ends_with(char::is_whitespace), "{drawn:?}");
        assert_eq!(buffer[(summary::cell_width(before), 0)].fg, Color::Cyan);
        assert!(summary::truncated(text, usize::from(width)).ends_with(TRUNCATION_MARK));
    }
    assert_eq!(summary::truncated("x", 0), "");
    assert_eq!(summary::truncated("long", 1), "…");
    assert_eq!(summary::truncated("fits ", 5), "fits ");
}

fn check_role_and_account_cells(ground: Color) {
    let colors = FooterColors::parse(
        "role.showrunner=bright-magenta\nrole.unit-director=bright-blue\nrole.worker=bright-cyan\nrole.unassigned=purple\naccount.claude1=brown\naccount.claude2=magenta\naccount.claude3=blue\naccount.claude4=orange\naccount.codex1=pink\naccount.codex2=teal",
    );
    let root = agent(10, "root", 60, None);
    let labels = [
        "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
    ];
    let roles = [
        (RoleLabel::Stated(HierarchyRole::Showrunner), "showrunner"),
        (RoleLabel::Stated(HierarchyRole::Unit), "unit"),
        (RoleLabel::Stated(HierarchyRole::Impl), "impl"),
        (RoleLabel::Unlabeled, "session"),
    ];
    let members: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| AgentRow {
            account: serde_json::from_value(
                serde_json::json!({"home": format!("~/.{label}"), "label":label}),
            )
            .expect("account fixture"),
            ..agent(
                u32::try_from(index + 20).expect("fixture pid"),
                &format!("member-{index}"),
                20,
                None,
            )
        })
        .collect();
    let mut cell = entry(&root, None);
    for (index, member) in members.iter().enumerate() {
        cell.lines.push(TreeLine {
            dispatch_label: DispatchLabel::Original,
            machine:        "natedev",
            location:       SessionLocation::SameMachine,
            row:            member,
            hue:            RainbowHue::Red,
            rails:          Vec::new(),
            branch:         Branch::Last,
            subject:        LineSubject::Agent(roles[index % roles.len()].0),
        });
    }
    let rect = area(180, 20);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(ground));
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Shown,
        &colors,
    );
    let role_x = find(&buffer, "role").expect("role header").0;
    let account_x = find(&buffer, "account").expect("account header").0;
    for (index, member) in members.iter().enumerate() {
        let y = find(&buffer, &member.name).expect("member row").1;
        for (x, word) in [
            (role_x, roles[index % roles.len()].1),
            (account_x, labels[index]),
        ] {
            for offset in 0..summary::cell_width(word) {
                let cell = &buffer[(x + offset, y)];
                assert!(
                    tests::contrast_ratio(cell.fg, cell.bg) >= MINIMUM_CONTRAST,
                    "{word}: {cell:?}"
                );
            }
        }
    }
}

#[test]
fn tree_role_and_account_words_meet_contrast_on_every_tile_ground() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    tui_pane::set_focused_pane_tint(true);
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for transparency in [false, true] {
            tui_pane::set_transparent_background(transparency);
            for focused in [false, true] {
                check_role_and_account_cells(tui_pane::pane_background(focused));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}
