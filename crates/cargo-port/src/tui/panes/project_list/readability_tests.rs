use std::path::Path;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::project::CheckoutInfo;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::GitStatus;
use crate::project::HeadState;
use crate::project::LocalGitState;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::TrackedRef;
use crate::project::Visibility;
use crate::tui::panes::PaneId;
use crate::tui::test_support;
use crate::tui::test_support::PROJECT;
use crate::tui::test_support::PROJECT_PATH;
use crate::tui::test_support::WIDTH;

#[test]
fn deleted_project_icon_keeps_its_shape_without_a_strike() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .visibility = Visibility::Deleted;
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let icon = test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, "🦀")
        .context("the project type icon")?;
    assert!(!buffer[icon].modifier.contains(Modifier::CROSSED_OUT));
    Ok(())
}

#[test]
fn narrow_projects_keep_the_total_mark() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::ProjectList, 16)?;
    test_support::position(&buffer, "Σ")?;
    test_support::position(&buffer, "1.0 GiB")?;
    Ok(())
}

#[test]
fn deleted_disk_cell_draws_only_the_dismiss_mark() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .visibility = Visibility::Deleted;
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let (_, y) = test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, PROJECT)
        .context("project row")?;
    let row = &test_support::rows(&buffer)[usize::from(y)];
    assert!(row.contains("[x]"));
    assert!(
        !row.contains("0.0"),
        "a Deleted row has no disk value: {row}"
    );
    Ok(())
}

#[test]
fn deleted_row_crosses_text_and_leaves_empty_cells_clear() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .visibility = Visibility::Deleted;
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let (x, y) = test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, PROJECT)
        .context("project row")?;
    assert!(buffer[(x, y)].modifier.contains(Modifier::CROSSED_OUT));
    let body = app.panes.project_list.body_rect;
    for x in body.x..body.right() {
        let cell = &buffer[(x, y)];
        if cell.symbol().trim().is_empty() {
            assert!(
                !cell.modifier.contains(Modifier::CROSSED_OUT),
                "empty cell {x} is crossed out"
            );
        }
    }
    Ok(())
}

#[test]
fn selected_project_has_one_fill_including_disk() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let body = app.panes.project_list.body_rect;
    let name = test_support::find_text_cell(&buffer, body, PROJECT).context("project row")?;
    let disk = test_support::find_text_cell(&buffer, body, "1.0 GiB").context("disk cell")?;
    assert_eq!(name.1, disk.1);
    let body = app.panes.project_list.body_rect;
    for x in body.x..body.right() {
        assert_eq!(
            buffer[(x, name.1)].bg,
            buffer[name].bg,
            "selection fill differs at {x}"
        );
    }
    app.set_focus_to_pane(PaneId::Cpu);
    let unselected = test_support::draw_ui(&mut app, WIDTH)?;
    assert_ne!(unselected[disk].bg, buffer[disk].bg);
    Ok(())
}

#[test]
fn project_headers_use_common_words() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    test_support::position(&buffer, "Sync")?;
    test_support::position(&buffer, "Main")?;
    Ok(())
}

#[test]
fn projects_leave_a_right_margin_after_disk_and_dismiss() -> anyhow::Result<()> {
    for deleted in [false, true] {
        let mut app = test_support::project_app();
        if deleted {
            app.project_list
                .at_path_mut(Path::new(PROJECT_PATH))
                .expect_found("the fixture project")
                .visibility = Visibility::Deleted;
        }
        let buffer = test_support::draw_ui(&mut app, WIDTH)?;
        let body = app.panes.project_list.body_rect;
        let (_, y) =
            test_support::find_text_cell(&buffer, body, PROJECT).context("the project row")?;
        assert_eq!(
            buffer[(body.right() - 1, y)].symbol(),
            " ",
            "the right margin must be empty"
        );
        let text = &test_support::rows(&buffer)[usize::from(y)];
        assert!(
            text.contains(if deleted { "[x]" } else { "1.0 GiB" }),
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn disk_colors_distinguish_small_and_large_values() -> anyhow::Result<()> {
    let projects = [("small", 73), ("large", 24_374_440_755)].map(|(name, bytes)| {
        let mut project = test_support::make_package(name, Path::new(&format!("/tmp/{name}")));
        if let RootItem::Rust(RustProject::Package(package)) = &mut project {
            package.disk_usage_bytes = Some(bytes);
        }
        project
    });
    let mut app = test_support::make_app(&projects);
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::ProjectList, 80)?;
    let small = test_support::position(&buffer, "73 B")?;
    let large = test_support::position(&buffer, "22.7 GiB")?;
    assert_ne!(buffer[small].fg, buffer[large].fg);
    Ok(())
}

#[test]
fn project_type_icons_have_a_heading() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::ProjectList, 80)?;
    test_support::position(&buffer, "Type")?;
    Ok(())
}

#[test]
fn selected_deleted_name_and_disk_have_readable_contrast() -> anyhow::Result<()> {
    let theme = test_support::shipped_dark_theme()?;
    let previous = tui_pane::theme();
    let mut failures = Vec::new();
    for deleted in [false, true] {
        let mut app = test_support::project_app();
        tui_pane::set_active_theme(std::sync::Arc::new(theme.clone()));
        let info = app
            .project_list
            .at_path_mut(Path::new(PROJECT_PATH))
            .expect_found("the fixture project");
        info.local_git_state = LocalGitState::Detected(Box::new(CheckoutInfo {
            status:              GitStatus::Modified,
            head:                HeadState::Branch("fleet".to_string()),
            last_commit:         CommitDate::Unreadable,
            ahead_behind_local:  Comparison::NotCompared,
            primary_tracked_ref: TrackedRef::Untracked,
            bisect:              None,
        }));
        if deleted {
            info.visibility = Visibility::Deleted;
        }
        let buffer = test_support::draw_ui(&mut app, WIDTH)?;
        let body = app.panes.project_list.body_rect;
        let mut drawn = Buffer::empty(body);
        for y in body.y..body.bottom() {
            for x in body.x..body.right() {
                drawn[(x, y)] = buffer[(x, y)].clone();
            }
        }
        failures.extend(test_support::contrast_failures(
            &drawn,
            &[PROJECT],
            "selected project",
        ));
        if !deleted {
            failures.extend(test_support::contrast_failures(
                &drawn,
                &["1.0 GiB"],
                "selected Disk",
            ));
        }
    }
    tui_pane::set_active_theme(previous);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn project_selection_has_visible_fill_and_one_text_color() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let theme = test_support::shipped_dark_theme()?;
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(theme));
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    tui_pane::set_active_theme(previous);
    let name = test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, PROJECT)
        .context("the selected project name")?;
    let disk = test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, "1.0 GiB")
        .context("the selected Disk value")?;
    assert_eq!(buffer[name].bg, ratatui::style::Color::Rgb(101, 101, 101));
    assert_eq!(buffer[name].fg, ratatui::style::Color::Rgb(255, 255, 255));
    assert_eq!(buffer[disk].fg, buffer[name].fg);
    Ok(())
}

#[test]
fn unselected_disk_has_no_separate_fill() -> anyhow::Result<()> {
    let projects = ["first-project", "other-project"]
        .map(|name| test_support::make_package(name, Path::new(&format!("/tmp/test/{name}"))));
    let mut app = test_support::make_app(&projects);
    app.project_list
        .at_path_mut(Path::new("/tmp/test/other-project"))
        .expect_found("the unselected project")
        .disk_usage_bytes = Some(73);
    app.scan.bump_generation();
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let name = test_support::position(&buffer, "other-project")?;
    let body = app.panes.project_list.body_rect;
    assert_eq!(buffer[name].bg, buffer[(body.right() - 2, name.1)].bg);
    let disk =
        test_support::find_text_cell(&buffer, body, "73 B").context("the unselected Disk value")?;
    assert_eq!(disk.1, name.1);
    assert_eq!(buffer[disk].bg, buffer[name].bg);
    Ok(())
}

#[test]
fn project_expand_mark_has_a_left_margin() -> anyhow::Result<()> {
    let RootItem::Rust(primary) = test_support::make_package(PROJECT, Path::new(PROJECT_PATH))
    else {
        anyhow::bail!("the fixture must be a Rust project");
    };
    let RootItem::Rust(linked) =
        test_support::make_package("linked", Path::new("/tmp/test/linked"))
    else {
        anyhow::bail!("the fixture must be a Rust project");
    };
    let mut app = test_support::make_app(&[RootItem::Worktrees(
        crate::project::WorktreeGroup::new(primary, vec![linked]),
    )]);
    app.project_list
        .select_project_in_tree(Path::new(PROJECT_PATH), true);
    app.project_list.collapse_all(true);
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let body = app.panes.project_list.body_rect;
    let mark = test_support::find_text_cell(&buffer, body, "▶").context("the expand mark")?;
    assert_eq!(buffer[(body.x, mark.1)].symbol(), " ");
    assert_eq!(mark.0, body.x + 1);
    Ok(())
}

#[test]
fn project_disk_units_stay_whole_at_eighty_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project")
        .disk_usage_bytes = Some(25_555_055_411);
    app.scan.bump_generation();
    app.ensure_visible_rows_cached();
    let buffer = test_support::draw_ui(&mut app, 80)?;
    test_support::find_text_cell(&buffer, app.panes.project_list.body_rect, "23.8 GiB")
        .context("the disk value must include the whole unit")?;
    Ok(())
}

#[test]
fn deleted_disk_values_do_not_change_the_live_size_scale() -> anyhow::Result<()> {
    let projects = [
        ("a-selected", 1024),
        ("live-small", 73),
        ("live-large", 1_073_741_824),
        ("removed", 0),
    ]
    .map(|(name, bytes)| {
        let mut project = test_support::make_package(name, Path::new(&format!("/tmp/test/{name}")));
        if let RootItem::Rust(RustProject::Package(package)) = &mut project {
            package.disk_usage_bytes = Some(bytes);
        }
        project
    });
    let mut app = test_support::make_app(&projects);
    let removed = Path::new("/tmp/test/removed");
    app.project_list
        .at_path_mut(removed)
        .expect_found("the removed project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_visible_rows_cached();
    let before = test_support::draw_pane(&mut app, PaneId::ProjectList, 80)?;
    app.project_list
        .at_path_mut(removed)
        .expect_found("the removed project")
        .disk_usage_bytes = Some(1_099_511_627_776);
    app.scan.bump_generation();
    app.ensure_visible_rows_cached();
    let after = test_support::draw_pane(&mut app, PaneId::ProjectList, 80)?;
    for value in ["73 B", "1.0 GiB"] {
        let first = test_support::position(&before, value)?;
        let second = test_support::position(&after, value)?;
        assert_eq!(before[first].fg, after[second].fg, "{value}");
    }
    test_support::position(&after, "[x]")?;
    Ok(())
}
