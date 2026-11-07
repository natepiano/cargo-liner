use std::path::Path;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;

use crate::config::LintIndicator;
use crate::project::AbsolutePath;
use crate::project::DiskMeasurement;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::scan::ProjectStorage;
use crate::tui::app::App;
use crate::tui::panes::CiData;
use crate::tui::panes::CiEmptyState;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::targets;
use crate::tui::project_list::ExpandKey;
use crate::tui::project_list_state::ProjectList;
use crate::tui::test_support;

const LONG_NAME: &str =
    "proof_crate [claude-build-followups-sdlc-merge-gate/scripts/aws_target/production]";
const HEADINGS: [&str; 8] = [
    "Projects", "Lint", "CI", "Type", "Git", "Sync", "Main", "Disk",
];

#[test]
fn phase4e_same_named_roots_show_different_bracket_text() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let items = ["alpha", "beta"]
        .into_iter()
        .map(|folder| {
            package(
                &scratch.path().join(folder).join("proof_crate"),
                "proof_crate",
                73,
            )
            .map(|package| RootItem::Rust(RustProject::Package(package)))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut app = test_support::make_app(&items);
    app.framework.color_mode = ColorMode::Colored;
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.ensure_visible_rows_cached();
    for width in [95, 80] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let body = app.panes.project_list.body_rect;
        let column = test_support::find_text_cell(
            &buffer,
            Rect::new(body.x, body.y - 1, body.width, 1),
            "CI",
        )
        .context("CI heading")?
        .0;
        let names: Vec<String> = (body.y..body.y + 2)
            .map(|y| text_row(&buffer, Rect::new(body.x, y, column - body.x, 1), y))
            .collect();
        assert_ne!(names[0], names[1], "same names at width {width}: {names:?}");
        assert!(names.iter().any(|name| name.contains("alpha")), "{names:?}");
        assert!(names.iter().any(|name| name.contains("beta")), "{names:?}");
    }
    Ok(())
}

fn package(root: &Path, name: &str, bytes: u64) -> anyhow::Result<Package> {
    std::fs::create_dir_all(root)?;
    std::fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )?;
    let mut package = Package {
        path: AbsolutePath::from(root),
        name: Some(name.to_string()),
        ..Package::default()
    };
    package.disk = DiskMeasurement::TotalOnly(bytes);
    Ok(package)
}

fn app_with_names(root: &Path, name: &str) -> anyhow::Result<App> {
    let directory = name.replace('/', "-");
    let first = package(&root.join(format!("{directory}-first")), name, 1 << 30)?;
    let second = package(&root.join(format!("{directory}-second")), name, 2 << 30)?;
    let mut deleted = package(&root.join("deleted"), "deleted", 0)?;
    deleted.visibility = Visibility::Deleted;
    let items = vec![
        RootItem::Rust(RustProject::Package(first)),
        RootItem::Rust(RustProject::Package(second)),
        RootItem::Rust(RustProject::Package(deleted)),
    ];
    let mut app = test_support::make_app(&items);
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.project_list = ProjectList::new(items);
    app.ensure_visible_rows_cached();
    let paths = app.project_list.visible_project_storage_paths();
    app.project_list
        .set_project_storage(&paths, ProjectStorage::Available(4 << 30));
    app.ensure_detail_cached();
    Ok(app)
}

fn text_row(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn assert_columns_and_disk_edges(app: &App, buffer: &Buffer, long: bool) -> anyhow::Result<()> {
    let area = app.panes.project_list.body_rect;
    let header = text_row(buffer, area, area.y - 1);
    for heading in HEADINGS {
        assert!(header.contains(heading), "missing {heading}: {header}");
    }
    let mut edges = Vec::new();
    for value in ["1.0 GiB", "2.0 GiB", "3.0 GiB", "4.0 GiB", "[x]"] {
        let (x, y) = test_support::find_text_cell(buffer, area, value).context(value)?;
        edges.push(x + u16::try_from(value.len())?);
        assert_eq!(buffer[(edges[0], y)].symbol(), " ");
    }
    assert!(edges.iter().all(|edge| *edge == edges[0]), "{edges:?}");
    test_support::find_text_cell(buffer, area, "Σ").context("Σ label")?;
    test_support::find_text_cell(buffer, area, "Available").context("Available label")?;
    if long {
        let lint_x = test_support::find_text_cell(
            buffer,
            Rect::new(area.x, area.y - 1, area.width, 1),
            "CI",
        )
        .context("CI column")?
        .0;
        for value in ["1.0 GiB", "2.0 GiB"] {
            let (_, y) = test_support::find_text_cell(buffer, area, value).context(value)?;
            let name = text_row(buffer, Rect::new(area.x, y, lint_x - area.x, 1), y);
            assert!(name.trim_end().ends_with('…'), "{name}");
        }
    }
    Ok(())
}

#[test]
fn project_names_shrink_before_columns_at_eighty_and_ninety_five() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    for name in ["app", LONG_NAME] {
        let mut app = app_with_names(scratch.path(), name)?;
        for width in [95, 80] {
            let buffer = test_support::draw_ui(&mut app, width)?;
            assert_columns_and_disk_edges(&app, &buffer, name == LONG_NAME)?;
        }
    }
    Ok(())
}

#[test]
fn worktree_names_shrink_before_columns_and_keep_disk_alignment() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let primary = package(
        &scratch
            .path()
            .join("proof_crate-claude-build-followups-sdlc-merge-gate-primary"),
        LONG_NAME,
        1 << 30,
    )?;
    let linked = package(&scratch.path().join(LONG_NAME), LONG_NAME, 2 << 30)?;
    let mut app = test_support::make_app(&[]);
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.project_list = ProjectList::new(vec![RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Package(primary),
        vec![RustProject::Package(linked)],
    ))]);
    app.project_list.expanded.expand(ExpandKey::Node(0), 0);
    app.ensure_visible_rows_cached();
    for width in [95, 80] {
        let buffer = test_support::draw_ui(&mut app, width)?;

        let area = app.panes.project_list.body_rect;
        let header = text_row(&buffer, area, area.y - 1);
        for heading in HEADINGS {
            assert!(header.contains(heading), "{header}");
        }
        for y in area.y..area.y + 3 {
            let row = text_row(&buffer, area, y);
            assert!(row.contains('…'), "{row}");
        }
        let first =
            test_support::find_text_cell(&buffer, area, "1.0 GiB").context("primary size")?;
        let second =
            test_support::find_text_cell(&buffer, area, "2.0 GiB").context("linked size")?;
        assert_eq!(first.0, second.0);
    }
    Ok(())
}

#[test]
fn narrow_project_columns_drop_in_order_after_the_name_floor() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    for (width, last, removed) in [
        (43, "Sync", "Main"),
        (38, "Git", "Sync"),
        (34, "Type", "Git"),
        (29, "CI", "Type"),
        (26, "Lint", "CI"),
        (24, "Projects", "Lint"),
    ] {
        let buffer = test_support::draw_pane(&mut app, PaneId::ProjectList, width)?;
        let area = app.panes.project_list.body_rect;
        let header = text_row(&buffer, area, area.y - 1);
        assert!(header.contains(last), "width {width}: {header}");
        assert!(!header.contains(removed), "width {width}: {header}");
        for value in ["1.0 GiB", "2.0 GiB", "3.0 GiB", "4.0 GiB", "[x]"] {
            let (x, y) = test_support::find_text_cell(&buffer, area, value).context(value)?;
            assert_eq!(
                x + u16::try_from(value.len())?,
                area.right() - 1,
                "width {width}, row {y}"
            );
        }
    }
    Ok(())
}

#[test]
fn deep_worktree_prefix_keeps_the_disk_column_at_the_name_floor() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut member = package(&scratch.path().join("linked/member"), LONG_NAME, 1 << 30)?;
    let mut vendored = VendoredPackage {
        path: AbsolutePath::from(scratch.path().join("linked/member/vendor/dependency")),
        name: Some("dependency-with-a-long-name".to_string()),
        ..VendoredPackage::default()
    };
    vendored.project_info.disk = DiskMeasurement::TotalOnly(2 << 30);
    member.vendored.push(vendored);
    let linked = Workspace {
        path: AbsolutePath::from(scratch.path().join("linked")),
        groups: vec![MemberGroup::Named {
            name:    LONG_NAME.to_string(),
            members: vec![member],
        }],
        ..Workspace::default()
    };
    let primary = package(&scratch.path().join("primary"), "primary", 3 << 30)?;
    let mut app = test_support::make_app(&[]);
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.project_list = ProjectList::new(vec![RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Package(primary),
        vec![RustProject::Workspace(linked)],
    ))]);
    app.project_list.expanded = tui_pane::ExpansionModel::from_open_keys([
        ExpandKey::Node(0),
        ExpandKey::Worktree(0, 1),
        ExpandKey::WorktreeGroup(0, 1, 0),
        ExpandKey::WorktreeMember(0, 1, 0, 0),
    ]);
    app.ensure_visible_rows_cached();
    for width in [48, 16] {
        let buffer = test_support::draw_pane(&mut app, PaneId::ProjectList, width)?;
        let area = app.panes.project_list.body_rect;
        let member =
            test_support::find_text_cell(&buffer, area, "1.0 GiB").context("member size")?;
        let vendored =
            test_support::find_text_cell(&buffer, area, "2.0 GiB").context("vendored size")?;
        assert_eq!(member.0, vendored.0);
        assert_eq!(vendored.0 + 7, area.right() - 1);
        assert!(text_row(&buffer, area, vendored.1).contains('…'));
    }
    Ok(())
}

#[test]
fn project_column_priority_keeps_the_remaining_panes_readable() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    install_targets(&mut app)?;
    for width in [95, 80, 67, 60] {
        if width == 67 {
            let run = serde_json::from_value(serde_json::json!({
                "run_id": 1, "created_at": "2026-11-05T12:00:00Z", "branch": "fleet",
                "url": "", "ci_status": "passed", "jobs": [],
                "wall_clock_secs": 60, "commit_title": "Repair the display"
            }))?;
            app.ci.set_content(CiData {
                runs:           vec![run],
                mode_label:     Some("branch".to_string()),
                current_branch: Some("fleet".to_string()),
                empty_state:    CiEmptyState::NoRuns,
            });
        }
        let buffer = test_support::draw_ui(&mut app, width)?;
        assert_drawn_pane_widths(&app, width);
        for pane in [
            PaneId::Package,
            PaneId::Git,
            PaneId::Cpu,
            PaneId::Lints,
            PaneId::CiRuns,
        ] {
            let area = app.panes.tiled_layout.area(pane);
            let rows: Vec<_> = (area.y..area.bottom())
                .map(|y| text_row(&buffer, area, y))
                .collect();
            if !area.is_empty() {
                assert!(rows.iter().any(|row| !row.trim().is_empty()));
            }
        }
        if width >= 80 {
            assert_columns_and_disk_edges(&app, &buffer, true)?;
        }
        if width == 60 {
            assert_eq!(app.panes.tiled_layout.area(PaneId::ProjectList).width, 30);
            for pane in [PaneId::Package, PaneId::Git, PaneId::Lints, PaneId::CiRuns] {
                let area = app.panes.tiled_layout.area(pane);
                assert!(!area.is_empty(), "{pane:?}");
                if matches!(pane, PaneId::Package | PaneId::Git) {
                    assert_eq!(area.width, 15);
                    assert!(app.tabbable_panes().contains(&pane), "{pane:?}");
                }
                assert_eq!(buffer[(area.right() - 1, area.y + 1)].symbol(), "│");
            }
            let projects = app.panes.project_list.body_rect;
            let header = text_row(&buffer, projects, projects.y - 1);
            for retained in ["Projects", "Lint", "CI", "Disk"] {
                assert!(header.contains(retained), "{header}");
            }
            for removed in ["Main", "Sync", "Git", "Type"] {
                assert!(!header.contains(removed), "{header}");
            }
            let lints = app.panes.tiled_layout.area(PaneId::Lints);
            test_support::find_text_cell(&buffer, lints, "No lint runs")
                .context("visible Lints pane")?;
        } else if width == 67 {
            let cpu = app.panes.tiled_layout.area(PaneId::Cpu);
            assert!(cpu.is_empty());
            assert!(!app.tabbable_panes().contains(&PaneId::Cpu));
            let targets = app.panes.tiled_layout.area(PaneId::Targets);
            test_support::find_text_cell(&buffer, targets, "first").context("whole target row")?;
            assert!(app.tabbable_panes().contains(&PaneId::Targets));
            assert!(app.tabbable_panes().contains(&PaneId::CiRuns));
            let ci = app.panes.tiled_layout.area(PaneId::CiRuns);
            test_support::find_text_cell(&buffer, ci, "CI runs").context("visible CI pane")?;
        } else {
            let git = app.panes.tiled_layout.area(PaneId::Git);
            test_support::find_text_cell(&buffer, git, "Core").context("whole Git rate label")?;
            let package = app.panes.tiled_layout.area(PaneId::Package);
            test_support::find_text_cell(&buffer, package, "Path")
                .context("whole Package label")?;
            let targets = app.panes.tiled_layout.area(PaneId::Targets);
            test_support::find_text_cell(&buffer, targets, "first").context("whole target row")?;
            assert!(app.tabbable_panes().contains(&PaneId::Targets));
            let cpu = app.panes.tiled_layout.area(PaneId::Cpu);
            if width == 80 {
                assert!(cpu.is_empty());
                assert!(!app.tabbable_panes().contains(&PaneId::Cpu));
            } else {
                test_support::find_text_cell(&buffer, cpu, "Monitoring off")
                    .context("whole CPU row")?;
                assert!(app.tabbable_panes().contains(&PaneId::Cpu));
            }
        }
    }
    Ok(())
}

fn assert_drawn_pane_widths(app: &App, width: u16) {
    if width == 60 {
        return;
    }
    let projects = app.panes.tiled_layout.area(PaneId::ProjectList);
    let expected_projects = if width == 67 { 37 } else { 48 };
    assert_eq!(projects.width, expected_projects);
    assert_eq!(
        app.panes.tiled_layout.area(PaneId::CiRuns).width,
        width - projects.width
    );
    let package = app.panes.tiled_layout.area(PaneId::Package);
    let git = app.panes.tiled_layout.area(PaneId::Git);
    let targets = app.panes.tiled_layout.area(PaneId::Targets);
    match width {
        95 => {
            assert_eq!((package.width, git.width), (24, 23));
            assert_eq!(app.panes.tiled_layout.area(PaneId::Cpu).width, 19);
            assert_eq!(targets.width, 28);
        },
        80 => {
            assert_eq!((package.width, git.width), (16, 16));
            assert_eq!(targets.width, 32);
        },
        _ => {
            assert_eq!(width, 67);
            assert_eq!((package.width, git.width), (15, 15));
            assert_eq!(targets.width, 30);
            for pane in [PaneId::Package, PaneId::Git] {
                assert!(app.tabbable_panes().contains(&pane));
            }
        },
    }
}

fn install_targets(app: &mut App) -> anyhow::Result<()> {
    let path = AbsolutePath::from(
        app.project_list
            .selected_project_path()
            .context("selected project")?,
    );
    let mut data = test_support::target_data();
    for entry in &mut data.binaries {
        entry.project_path = path.clone();
        entry.package_path = path.clone();
    }
    app.panes.targets.set_content(data);
    app.panes.git.set_content(GitData {
        checkout: GitCheckout::Unborn,
        ..crate::tui::panes::GitData::default()
    });
    Ok(())
}

#[test]
fn targets_always_fill_the_right_column_before_cpu_joins() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    install_targets(&mut app)?;
    let targets_min = targets::minimum_width();
    let cpu_width = crate::tui::panes::constants::CPU_PANE_WIDTH;
    let projects_width = 48;
    for right_width in [
        cpu_width,
        targets_min + cpu_width - 1,
        targets_min + cpu_width,
    ] {
        let buffer = test_support::draw_ui(&mut app, projects_width + right_width)?;
        let cpu = app.panes.tiled_layout.area(PaneId::Cpu);
        let targets = app.panes.tiled_layout.area(PaneId::Targets);
        assert!(!targets.is_empty());
        assert!(app.tabbable_panes().contains(&PaneId::Targets));
        test_support::find_text_cell(&buffer, targets, "first").context("target row")?;
        let drawn_right_width =
            buffer.area.width - app.panes.tiled_layout.area(PaneId::ProjectList).width;
        assert_eq!(cpu.is_empty(), drawn_right_width < targets_min + cpu_width);
        assert_eq!(app.tabbable_panes().contains(&PaneId::Cpu), !cpu.is_empty());
    }
    Ok(())
}

fn long_worktree_app(root: &Path) -> anyhow::Result<App> {
    let entries = [
        "proof_crate-a-primary-with-a-long-name",
        "proof_crate-b-linked-with-a-long-name",
        "proof_crate-c-other-with-a-long-name",
    ]
    .into_iter()
    .map(|name| package(&root.join(name), LONG_NAME, 1 << 30).map(RustProject::Package))
    .collect::<anyhow::Result<Vec<_>>>()?;
    let mut entries = entries.into_iter();
    let primary = entries.next().context("primary worktree")?;
    let mut app = test_support::make_app(&[]);
    app.framework.color_mode = ColorMode::Colored;
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.project_list = ProjectList::new(vec![RootItem::Worktrees(WorktreeGroup::new(
        primary,
        entries.collect(),
    ))]);
    app.project_list.expanded.expand(ExpandKey::Node(0), 0);
    app.ensure_visible_rows_cached();
    install_targets(&mut app)?;
    Ok(app)
}

#[test]
fn long_worktrees_leave_detail_panes_visible_at_sixty_columns() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    for lint in [LintIndicator::Enabled, LintIndicator::Disabled] {
        let mut app = long_worktree_app(scratch.path())?;
        app.config.current_mut().lint.enabled = lint;
        let paths = app.project_list.visible_project_storage_paths();
        app.project_list
            .set_project_storage(&paths, ProjectStorage::Available(295 << 30));
        let buffer = test_support::draw_ui(&mut app, 60)?;
        assert_eq!(app.panes.tiled_layout.area(PaneId::ProjectList).width, 30);
        for pane in [PaneId::Package, PaneId::Git, PaneId::Lints, PaneId::CiRuns] {
            let area = app.panes.tiled_layout.area(pane);
            assert!(!area.is_empty(), "{pane:?}");
            let title = text_row(&buffer, area, area.y);
            assert!(title.contains('─'), "{pane:?}: {title}");
        }
        for pane in [PaneId::Package, PaneId::Git] {
            assert!(app.tabbable_panes().contains(&pane));
            assert_eq!(app.panes.tiled_layout.area(pane).width, 15);
        }
        let area = app.panes.project_list.body_rect;
        for y in area.y..area.y + 4 {
            let size = if y == area.y { "3.0 GiB" } else { "1.0 GiB" };
            let (x, _) =
                test_support::find_text_cell(&buffer, Rect::new(area.x, y, area.width, 1), size)
                    .context("worktree disk value")?;
            assert_eq!(x + 7, area.right() - 1, "{lint:?}");
            assert_eq!(buffer[(area.right() - 1, y)].symbol(), " ");
            assert_eq!(buffer[(area.right(), y)].symbol(), "│");
        }
        let group = text_row(&buffer, area, area.y);
        assert!(group.contains("proof"), "{group}");
        test_support::find_text_cell(&buffer, Rect::new(area.x, area.y, area.width, 1), "… 🌲:3")
            .context("the group keeps its mark")?;
        let primary = text_row(&buffer, area, area.y + 1);
        assert!(
            primary.contains("pro") && primary.contains("… (p)"),
            "{primary}"
        );
    }
    Ok(())
}

#[test]
fn shortened_worktree_names_keep_group_and_primary_marks() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = long_worktree_app(scratch.path())?;
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.project_list.body_rect;
    let group = text_row(&buffer, area, area.y);
    test_support::find_text_cell(&buffer, Rect::new(area.x, area.y, area.width, 1), "… 🌲:3")
        .with_context(|| format!("the shortened group keeps its mark: {group}"))?;
    let primary = text_row(&buffer, area, area.y + 1);
    assert!(primary.contains("… (p)"), "{primary}");
    let mut disk_edge = None;
    for y in area.y..area.y + 4 {
        let row = text_row(&buffer, area, y);
        assert!(row.contains('…'), "{row}");
        if y > area.y + 1 {
            assert!(!row.contains("(p)"), "{row}");
        }
        let size = if y == area.y { "3.0 GiB" } else { "1.0 GiB" };
        let (x, _) =
            test_support::find_text_cell(&buffer, Rect::new(area.x, y, area.width, 1), size)
                .context("worktree disk value")?;
        assert_eq!(*disk_edge.get_or_insert(x + 7), x + 7);
    }
    Ok(())
}

#[test]
fn same_named_roots_remove_shared_folder_words() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let items = ["sdlc-merge-gate", "verify-run-place"]
        .into_iter()
        .map(|folder| {
            package(
                &scratch
                    .path()
                    .join(format!("claude-build-followups-{folder}"))
                    .join("proof_crate"),
                "proof_crate",
                73,
            )
            .map(|package| RootItem::Rust(RustProject::Package(package)))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut app = test_support::make_app(&items);
    app.framework.color_mode = ColorMode::Colored;
    for width in [160, 95, 80] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let body = app.panes.project_list.body_rect;
        let names: Vec<String> = (body.y..body.y + 2)
            .map(|y| text_row(&buffer, body, y))
            .collect();
        assert_ne!(names[0], names[1], "{width}: {names:?}");
        for prefix in ["[sdlc", "[verify"] {
            assert!(
                names.iter().any(|row| row.contains(prefix)),
                "{width}: {names:?}"
            );
        }
        assert!(
            names.iter().all(|row| !row.contains("claude-")),
            "{names:?}"
        );
        if width == 160 {
            for label in [
                "proof_crate [sdlc-merge-gate]",
                "proof_crate [verify-run-place]",
            ] {
                assert!(names.iter().any(|row| row.contains(label)), "{names:?}");
            }
        }
    }
    Ok(())
}
