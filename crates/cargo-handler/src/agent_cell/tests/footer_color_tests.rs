//! The configured footer colors survive tile rendering and clipping.

use ratatui::style::Color;

use super::*;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::RoleFallback;
use crate::census::worker_dispatch::WorkerTask;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;

/// A probe account fixture, independent of the machine's configuration.
fn account(label: &str) -> Account {
    serde_json::from_value(serde_json::json!({
        "home": format!("~/.{label}"), "label": label,
    }))
    .expect("the probe account fixture parses")
}

/// Draw with explicit colors rather than reading the user's footer file.
fn rendered_with_colors(
    cell: &AgentEntry<'_>,
    width: u16,
    account_column: AccountColumn,
    colors: &FooterColors,
) -> Buffer {
    let rect = area(width, height(cell, width, NOW, account_column));
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        cell,
        std::slice::from_ref(cell),
        NOW,
        account_column,
        colors,
    );
    buffer
}

/// Assert each `Buffer` cell covering `text` has its foreground set to the
/// expected `Color`, through the final character.
fn assert_color(buffer: &Buffer, text: &str, expected: Color) {
    let (x, y) = find(buffer, text).expect("the label is drawn");
    for offset in 0..summary::cell_width(text) {
        let actual = buffer[(x + offset, y)].fg;
        assert_eq!(
            tui_pane::blend_color(actual, actual, 0),
            tui_pane::blend_color(expected, expected, 0),
            "{text} at {offset}"
        );
    }
}

#[test]
fn footer_role_colors_reach_each_agent_role_in_the_tree() {
    let colors = FooterColors::parse(
        "role.showrunner=red\nrole.unit-director=green\n\
         role.worker=orange\nrole.unassigned=purple",
    );
    let root = agent(10, "root", HOUR, None);
    let member = agent(20, "member", MINUTE, Some(10));
    for (role, label, expected) in [
        (
            RoleLabel::Stated(HierarchyRole::Showrunner),
            "showrunner",
            colors.role(FooterRole::Showrunner),
        ),
        (
            RoleLabel::Inferred(HierarchyRole::Showrunner),
            "showrunner",
            colors.role(FooterRole::Showrunner),
        ),
        (
            RoleLabel::Stated(HierarchyRole::Unit),
            "unit",
            colors.role(FooterRole::UnitDirector),
        ),
        (
            RoleLabel::Inferred(HierarchyRole::Unit),
            "unit",
            colors.role(FooterRole::UnitDirector),
        ),
        (
            RoleLabel::Stated(HierarchyRole::Impl),
            "impl",
            FooterTint::Plain,
        ),
        (
            RoleLabel::Stated(HierarchyRole::Test),
            "test",
            FooterTint::Plain,
        ),
        (
            RoleLabel::Stated(HierarchyRole::Review),
            "review",
            FooterTint::Plain,
        ),
        (
            RoleLabel::Stated(HierarchyRole::Fix),
            "fix",
            FooterTint::Plain,
        ),
        (RoleLabel::Unlabeled, "session", FooterTint::Plain),
    ] {
        let expected = expected
            .style(text_default().into())
            .fg
            .expect("resolved footer color");
        let mut cell = entry(&root, None);
        cell.lines.push(TreeLine {
            dispatch_label: DispatchLabel::Original,
            machine:        "natedev",
            location:       SessionLocation::SameMachine,
            row:            &member,
            hue:            RainbowHue::Red,
            rails:          Vec::new(),
            branch:         Branch::Last,
            subject:        LineSubject::Agent(role),
        });
        for width in [80, 220] {
            let buffer = rendered_with_colors(&cell, width, AccountColumn::Shown, &colors);
            let y = find(&buffer, "member").expect("the member is drawn").1;
            let x = find(&buffer, "role").expect("the role header is drawn").0;
            for offset in 0..summary::cell_width(label) {
                assert_eq!(buffer[(x + offset, y)].fg, expected, "{label} at {width}");
            }
        }
    }
}

#[test]
fn footer_account_colors_reach_the_tree_and_header_without_recoloring_tiers() {
    let colors = FooterColors::parse("account.codex1=pink\naccount.claude2=teal");
    let root = AgentRow {
        role_fallback: RoleFallback::None,
        agent: Agent::Codex,
        service_tier: ServiceTier::Fast,
        account: account("codex1"),
        ..agent(10, "root", HOUR, None)
    };
    let member = AgentRow {
        role_fallback: RoleFallback::None,
        account: account("claude2"),
        ..agent(20, "member", MINUTE, Some(10))
    };
    let mut cell = entry(&root, None);
    cell.lines.push(TreeLine {
        dispatch_label: DispatchLabel::Original,
        machine:        "natedev",
        location:       SessionLocation::SameMachine,
        row:            &member,
        hue:            RainbowHue::Red,
        rails:          Vec::new(),
        branch:         Branch::Last,
        subject:        LineSubject::Agent(RoleLabel::Unlabeled),
    });
    let buffer = rendered_with_colors(&cell, 220, AccountColumn::Shown, &colors);
    assert_color(&buffer, "codex1", Color::Indexed(211));
    assert_color(
        &buffer,
        "claude2",
        colors
            .account("claude2")
            .style(text_default().into())
            .fg
            .expect("covered account ink"),
    );
    assert_color(
        &buffer,
        " fast",
        Role::Codex.style().fg.expect("program color"),
    );

    let hidden = rendered_with_colors(&cell, 220, AccountColumn::Hidden, &colors);
    assert_color(
        &hidden,
        "codex fast",
        Role::Codex.style().fg.expect("program color"),
    );

    let unrecorded = AgentRow {
        role_fallback: RoleFallback::None,
        account: Account::Unrecorded,
        ..root.clone()
    };
    let program_colors = FooterColors::parse("account.codex=green");
    let buffer = rendered_with_colors(
        &entry(&unrecorded, None),
        220,
        AccountColumn::Shown,
        &program_colors,
    );
    assert_color(
        &buffer,
        "codex fast",
        Role::Codex.style().fg.expect("program color"),
    );
}

#[test]
fn managed_thread_roles_and_accounts_take_footer_colors_independently_of_status() {
    let colors = FooterColors::parse("role.worker=orange\naccount.codex1=bright-cyan");
    let root = agent(10, "root", HOUR, None);
    let worker = ChildRow {
        worker_task: WorkerTask::Unrecorded,
        service_tier: ServiceTier::Fast,
        account: account("codex1"),
        ..child(0, ChildKind::Thread, None, "worker task", MINUTE)
    };
    for (role, label) in [
        (HierarchyRole::Impl, "impl"),
        (HierarchyRole::Test, "test"),
        (HierarchyRole::Review, "review"),
        (HierarchyRole::Fix, "fix"),
    ] {
        let mut cell = entry(&root, None);
        cell.lines.push(TreeLine {
            dispatch_label: DispatchLabel::Original,
            machine:        "natedev",
            location:       SessionLocation::SameMachine,
            row:            &root,
            hue:            RainbowHue::Red,
            rails:          Vec::new(),
            branch:         Branch::Last,
            subject:        LineSubject::ManagedThread(&worker, role),
        });
        let buffer = rendered_with_colors(&cell, 220, AccountColumn::Shown, &colors);
        assert_color(&buffer, label, text_default());
        assert_color(&buffer, "codex1", Color::LightCyan);
        assert_color(
            &buffer,
            " fast",
            Role::Codex.style().fg.expect("program color"),
        );
    }
}

/// Read one tree column, bounded by the next visible header.
fn tree_column(buffer: &Buffer, header: &str, y: u16) -> String {
    let (start, header_y) = find(buffer, header).expect("tree column header");
    let end = [
        "session",
        "role",
        "status",
        "account",
        "branch",
        "directory",
        "age",
    ]
    .into_iter()
    .filter_map(|label| find(buffer, label))
    .filter(|&(x, row)| row == header_y && x > start)
    .map(|(x, _)| x)
    .min()
    .unwrap_or(buffer.area.width);
    (start..end)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_string()
}

#[test]
fn codex_worker_and_thread_status_cells_show_activity_with_tier_after_account() {
    let colors = FooterColors::parse("account.codex1=bright-cyan");
    for (state, status, color) in [
        (
            AgentActivity::Active(Activity::Thinking),
            "thinking",
            Role::Busy,
        ),
        (AgentActivity::Idle, "idle", Role::Idle),
    ] {
        let worker = ChildRow {
            worker_task: WorkerTask::Unrecorded,
            activity: state,
            service_tier: ServiceTier::Fast,
            account: account("codex1"),
            ..child(0, ChildKind::Thread, None, "worker task", MINUTE)
        };
        let root = AgentRow {
            role_fallback: RoleFallback::None,
            children: vec![worker.clone()],
            ..agent(10, "root", HOUR, None)
        };
        let mut cells = vec![(entry(&root, None), false)];
        for role in [
            HierarchyRole::Impl,
            HierarchyRole::Test,
            HierarchyRole::Review,
            HierarchyRole::Fix,
        ] {
            let mut cell = entry(&root, None);
            cell.lines.clear();
            cell.lines.push(TreeLine {
                dispatch_label: DispatchLabel::Original,
                machine:        "natedev",
                location:       SessionLocation::SameMachine,
                row:            &root,
                hue:            RainbowHue::Red,
                rails:          Vec::new(),
                branch:         Branch::Last,
                subject:        LineSubject::ManagedThread(&worker, role),
            });
            cells.push((cell, true));
        }
        for (cell, inherits_directory) in cells {
            for width in [80, WIDE] {
                let buffer = rendered_with_colors(&cell, width, AccountColumn::Shown, &colors);
                if width == WIDE {
                    let (session_x, header_y) =
                        find(&buffer, "session").expect("full session header");
                    for header in ["role", "status", "account", "age"] {
                        let (x, y) = find(&buffer, header).expect("full tree column header");
                        assert_eq!(y, header_y, "{header} remains in the full header");
                        assert!(x > session_x, "{header} remains its own column");
                    }
                }
                for header in ["type", "branch"] {
                    assert!(find(&buffer, header).is_none(), "empty {header} is omitted");
                }
                let y = find(&buffer, "worker task").expect("worker row").1;
                if inherits_directory && width == WIDE {
                    let x = find(&buffer, "directory")
                        .expect("inherited directory header")
                        .0;
                    let drawn: String = (x..x + summary::cell_width(&root.directory))
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    assert_eq!(drawn, root.directory);
                } else if !inherits_directory {
                    assert!(
                        find(&buffer, "directory").is_none(),
                        "ordinary work has no directory"
                    );
                }
                assert_eq!(tree_column(&buffer, "status", y), status);
                assert_eq!(tree_column(&buffer, "account", y), "codex1 fast");
                let x = find(&buffer, "status").expect("status header").0;
                for offset in 0..summary::cell_width(status) {
                    assert_eq!(
                        buffer[(x + offset, y)].fg,
                        color.style().fg.expect("status color")
                    );
                }
                assert_color(&buffer, "codex1", Color::LightCyan);
                assert_color(
                    &buffer,
                    " fast",
                    Role::Codex.style().fg.expect("tier color"),
                );
            }
        }
    }
}

#[test]
fn footer_colors_leave_shells_and_subagents_plain_and_color_codex_workers() {
    let colors = FooterColors::parse(
        "role.unassigned=purple\nrole.worker=orange\n\
         role.shell=red\nrole.subagent=green\nrole.thread=blue",
    );
    let root = trunk();
    let buffer = rendered_with_colors(&entry(&root, None), 220, AccountColumn::Shown, &colors);
    assert_color(&buffer, "subagent", text_default());
    assert_color(&buffer, "worker", text_default());
    let role_x = find(&buffer, "role").expect("the role header is drawn").0;
    assert!(
        find(&buffer, "branch").is_none(),
        "the missing branch has no column"
    );
    for shell in [
        "Launch the Phase 1 implementation seat",
        "cargo nextest run -p hana_video --no-fail-fast -- permission_queue",
    ] {
        let y = find(&buffer, shell).expect("the shell line is drawn").1;
        assert_eq!(buffer[(role_x, y)].symbol(), " ");
        assert_eq!(
            buffer[(role_x, y)].style(),
            ratatui::buffer::Cell::default().style()
        );
    }
}

#[test]
fn absent_footer_colors_preserve_the_existing_tile_styles() {
    let root = AgentRow {
        role_fallback: RoleFallback::None,
        account: account("claude2"),
        children: vec![ChildRow {
            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
            account: account("codex1"),
            ..child(0, ChildKind::Thread, None, "worker task", MINUTE)
        }],
        ..agent(10, "root", HOUR, None)
    };
    let buffer = rendered_with_colors(
        &entry(&root, None),
        220,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    assert_color(
        &buffer,
        "claude2",
        Role::Claude.style().fg.expect("program color"),
    );
    assert_color(&buffer, "codex1", text_default());
    assert_color(
        &buffer,
        "worker task",
        Role::Rainbow(RainbowHue::Red)
            .style()
            .fg
            .expect("parent hue"),
    );
    let role_x = find(&buffer, TREE_ROLE_HEADER).expect("role header").0;
    let row = find(&buffer, "worker task").expect("worker name").1;
    assert_eq!(buffer[(role_x, row)].fg, text_default());
}

/// Unroled Codex workers share their parent session's hue in either tree view.
#[test]
fn unroled_workers_inherit_parent_hues_while_shells_and_subagents_stay_plain() {
    let root = AgentRow {
        role_fallback: RoleFallback::None,
        directory: "/fixture/color-check".repeat(4),
        children: vec![
            child(0, ChildKind::Thread, None, "unroled task", MINUTE),
            child(
                0,
                ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                Some(11),
                "shell task",
                MINUTE,
            ),
            child(
                0,
                ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                None,
                "subagent task",
                MINUTE,
            ),
        ],
        ..agent(10, "root", HOUR, None)
    };
    let cell = entry(&root, None);
    let colors = FooterColors::parse("role.worker=bright-cyan");
    let full_width = (1..=220)
        .find(|&width| {
            matches!(
                CellView::at_width(&cell, width, NOW, AccountColumn::Shown),
                CellView::Full(_)
            )
        })
        .expect("the fixture fits the full view");
    for width in [full_width - 1, 220] {
        assert_eq!(
            matches!(
                CellView::at_width(&cell, width, NOW, AccountColumn::Shown),
                CellView::Compressed
            ),
            width < full_width,
            "exercise both compressed and full name colors"
        );
        let buffer = rendered_with_colors(&cell, width, AccountColumn::Shown, &colors);
        assert_color(
            &buffer,
            "unroled task",
            Role::Rainbow(cell.hue).style().fg.expect("parent hue"),
        );
        assert_color(&buffer, "shell task", text_default());
        assert_color(&buffer, "subagent task", text_default());
        let y = find(&buffer, "unroled task").expect("worker row").1;
        assert_eq!(tree_column(&buffer, "role", y), "worker");
        let x = find(&buffer, "role").expect("role header").0;
        assert_eq!(buffer[(x, y)].fg, text_default());
    }
}
