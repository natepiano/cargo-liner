use std::path::Path;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;

use crate::config::LintIndicator;
use crate::http::RateLimitQuota;
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
use crate::tui::columns::COL_NAME;
use crate::tui::columns::COMPLETE_PROJECT_HEADERS_WIDTH;
use crate::tui::columns::ProjectListWidths;
use crate::tui::panes::CiData;
use crate::tui::panes::CiEmptyState;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::QuotaObservation;
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
    app.framework.color_mode = ColorMode::Colored;
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
        let expected = if heading == "Projects" && header.starts_with("   Proj…") {
            "Proj…"
        } else {
            heading
        };
        assert!(header.contains(expected), "missing {expected}: {header}");
    }
    let mut edges = Vec::new();
    let values = if (60..=80).contains(&buffer.area.width) {
        ["1 GiB", "2 GiB", "3 GiB", "4 GiB", "[x]"]
    } else {
        ["1.0 GiB", "2.0 GiB", "3.0 GiB", "4.0 GiB", "[x]"]
    };
    for value in values {
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
        for value in values.into_iter().take(2) {
            let (_, y) = test_support::find_text_cell(buffer, area, value).context(value)?;
            let name = text_row(buffer, Rect::new(area.x, y, lint_x - area.x, 1), y);
            assert!(name.trim_end().ends_with('…'), "{name}");
            assert!(name.chars().any(char::is_alphabetic), "{name}");
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
            let expected = if heading == "Projects" && header.starts_with("   Proj…") {
                "Proj…"
            } else {
                heading
            };
            assert!(header.contains(expected), "{header}");
        }
        for y in area.y..area.y + 3 {
            let row = text_row(&buffer, area, y);
            assert!(row.contains('…'), "{row}");
        }
        let (first_size, second_size) = if width == 80 {
            ("1 GiB", "2 GiB")
        } else {
            ("1.0 GiB", "2.0 GiB")
        };
        let first =
            test_support::find_text_cell(&buffer, area, first_size).context("primary size")?;
        let second =
            test_support::find_text_cell(&buffer, area, second_size).context("linked size")?;
        assert_eq!(first.0, second.0);
    }
    Ok(())
}

#[test]
fn narrow_project_columns_drop_in_order_after_the_name_floor() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    for (width, last, removed) in [
        (45, "Sync", "Main"),
        (39, "Git", "Sync"),
        (35, "Type", "Git"),
        (30, "CI", "Type"),
        (27, "Lint", "CI"),
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

fn install_git_quotas(app: &mut App) -> anyhow::Result<()> {
    let quota = QuotaObservation::Observed(RateLimitQuota {
        limit:     5000,
        used:      42,
        remaining: 4958,
        reset_at:  None,
    });
    let mut git = app.panes.git.content().context("Git content")?.clone();
    git.rate_limit_core = quota;
    git.rate_limit_graphql = quota;
    app.panes.git.set_content(git);
    Ok(())
}

#[test]
fn project_column_priority_keeps_the_remaining_panes_readable() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    install_targets(&mut app)?;
    install_git_quotas(&mut app)?;
    for (width, lint) in [
        (95, LintIndicator::Enabled),
        (80, LintIndicator::Enabled),
        (67, LintIndicator::Enabled),
        (60, LintIndicator::Enabled),
        (67, LintIndicator::Disabled),
    ] {
        app.config.current_mut().lint.enabled = lint;
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
        let projects = app.panes.project_list.body_rect;
        let header = text_row(&buffer, projects, projects.y - 1);
        assert!(!header.contains("ProjectsLint"), "{width}: {header}");
        let heading = if header.starts_with("   Proj…") {
            "Proj…"
        } else {
            "Projects"
        };
        let end = header.find(heading).context("Projects heading")? + heading.len();
        assert!(header[end..].starts_with(' '), "{width}: {header}");

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
            assert_sixty_column_project_priority(&app, &buffer)?;
        } else if width == 67 {
            let cpu = app.panes.tiled_layout.area(PaneId::Cpu);
            assert!(cpu.is_empty());
            assert!(!app.tabbable_panes().contains(&PaneId::Cpu));
            let targets = app.panes.tiled_layout.area(PaneId::Targets);
            if lint == LintIndicator::Enabled {
                assert!(targets.is_empty());
                assert!(!app.tabbable_panes().contains(&PaneId::Targets));
            } else {
                test_support::find_text_cell(&buffer, targets, "first")
                    .context("whole target row")?;
                assert!(app.tabbable_panes().contains(&PaneId::Targets));
            }
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

#[test]
fn lint_on_headings_sit_over_their_columns_at_sixty_and_eighty() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for width in [60, 80] {
            let buffer = test_support::draw_ui(&mut app, width)?;
            let area = app.panes.project_list.body_rect;
            let header = Rect::new(area.x, area.y - 1, area.width, 1);
            let type_heading =
                test_support::find_text_cell(&buffer, header, "Type").context("Type heading")?;
            let icon = test_support::find_text_cell(&buffer, area, "🦀").context("Rust icon")?;
            assert_eq!(
                type_heading.0,
                icon.0,
                "{mode:?} {width}: {:?}",
                test_support::rows(&buffer)
            );
            let header_text = text_row(&buffer, area, area.y - 1);
            assert!(header_text.starts_with("   Proj… Lint CI"), "{header_text}");
            assert_eq!(buffer[(area.x, area.y - 1)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn project_width_keeps_the_workspace_member_title_whole() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_names(scratch.path(), LONG_NAME)?;
    app.config.current_mut().lint.enabled = LintIndicator::Disabled;
    install_targets(&mut app)?;
    let mut data = app
        .panes
        .package
        .content()
        .context("Package content")?
        .clone();
    data.title = "Workspace Member".to_string();
    app.panes.package.set_content(data);
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for width in [80, 81, 95] {
            let buffer = test_support::draw_ui(&mut app, width)?;
            if width == 80 {
                assert_eq!(
                    app.panes.tiled_layout.area(PaneId::ProjectList).width,
                    COMPLETE_PROJECT_HEADERS_WIDTH
                );
                assert_eq!(app.panes.tiled_layout.area(PaneId::Package).width, 18);
                assert_eq!(app.panes.tiled_layout.area(PaneId::Git).width, 17);
                assert_eq!(app.panes.tiled_layout.area(PaneId::Targets).width, 35);
            }
            let area = app.panes.tiled_layout.area(PaneId::Package);
            test_support::find_text_cell(
                &buffer,
                Rect::new(area.x, area.y, area.width, 1),
                "Workspace Member",
            )
            .with_context(|| format!("whole member title at {width} in {mode:?}"))?;
        }
    }
    Ok(())
}

fn assert_sixty_column_project_priority(app: &App, buffer: &Buffer) -> anyhow::Result<()> {
    assert_columns_and_disk_edges(app, buffer, false)?;
    let projects_width =
        ProjectListWidths::pane_width_limit(60, app.config.current().lint.enabled.into());
    assert_eq!(
        app.panes.tiled_layout.area(PaneId::ProjectList).width,
        projects_width
    );
    assert!(app.panes.tiled_layout.area(PaneId::Package).is_empty());
    assert!(!app.tabbable_panes().contains(&PaneId::Package));
    for pane in [PaneId::Git, PaneId::Lints, PaneId::CiRuns] {
        let area = app.panes.tiled_layout.area(pane);
        assert!(!area.is_empty(), "{pane:?}");
        if pane == PaneId::Git {
            assert_eq!(area.width, 60 - projects_width);
            assert!(app.tabbable_panes().contains(&pane), "{pane:?}");
        }
        assert_eq!(buffer[(area.right() - 1, area.y + 1)].symbol(), "│");
    }
    let projects = app.panes.project_list.body_rect;
    let name_width = app
        .project_list
        .cached_fit_widths
        .fitted(usize::from(projects.width.saturating_sub(1)), 60)
        .get(COL_NAME);
    for y in projects.y..projects.y + 2 {
        let name = text_row(
            buffer,
            Rect::new(projects.x, y, u16::try_from(name_width)?, 1),
            y,
        );
        assert!(name.trim_end().ends_with('…'), "{name}");
        assert!(name.chars().any(char::is_alphabetic), "{name}");
    }
    let header = text_row(buffer, projects, projects.y - 1);
    for retained in HEADINGS {
        let expected = if retained == "Projects" {
            "Proj…"
        } else {
            retained
        };
        assert!(header.contains(expected), "{header}");
    }
    let lints = app.panes.tiled_layout.area(PaneId::Lints);
    test_support::find_text_cell(buffer, lints, "No lint runs").context("visible Lints pane")?;
    Ok(())
}

fn assert_drawn_pane_widths(app: &App, width: u16) {
    if width == 60 {
        return;
    }
    let projects = app.panes.tiled_layout.area(PaneId::ProjectList);
    let expected_projects = if width <= 80 {
        ProjectListWidths::pane_width_limit(width, app.config.current().lint.enabled.into())
    } else {
        // The list sits at its minimum: the whole `Projects` heading, a gap,
        // and the `Lint` column as wide as its heading.
        50
    };
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
            assert_eq!((package.width, git.width), (23, 22));
            assert_eq!(app.panes.tiled_layout.area(PaneId::Cpu).width, 19);
            assert_eq!(targets.width, 26);
        },
        80 => {
            assert_eq!((package.width, git.width), (17, 16));
            assert_eq!(targets.width, width - expected_projects);
        },
        _ => {
            assert_eq!(width, 67);
            assert!(package.is_empty());
            assert_eq!(git.width, width - expected_projects);
            if app.config.current().lint.enabled == LintIndicator::Enabled {
                assert!(targets.is_empty());
            } else {
                assert_eq!(targets.width, width - expected_projects);
            }
            assert!(!app.tabbable_panes().contains(&PaneId::Package));
            assert!(app.tabbable_panes().contains(&PaneId::Git));
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
    let projects_width =
        ProjectListWidths::pane_width_limit(60, app.config.current().lint.enabled.into())
            + targets_min.saturating_sub(cpu_width);
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
fn long_worktrees_keep_columns_and_git_at_sixty_columns() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = long_worktree_app(scratch.path())?;
    for lint in [LintIndicator::Enabled, LintIndicator::Disabled] {
        app.config.current_mut().lint.enabled = lint;
        let paths = app.project_list.visible_project_storage_paths();
        app.project_list
            .set_project_storage(&paths, ProjectStorage::Available(295 << 30));
        let buffer = test_support::draw_ui(&mut app, 60)?;
        let projects_width =
            ProjectListWidths::pane_width_limit(60, app.config.current().lint.enabled.into());
        assert_eq!(
            app.panes.tiled_layout.area(PaneId::ProjectList).width,
            projects_width
        );
        assert!(app.panes.tiled_layout.area(PaneId::Package).is_empty());
        assert!(!app.tabbable_panes().contains(&PaneId::Package));
        for pane in [PaneId::Git, PaneId::Lints, PaneId::CiRuns] {
            let area = app.panes.tiled_layout.area(pane);
            assert!(!area.is_empty(), "{pane:?}");
            let title = text_row(&buffer, area, area.y);
            assert!(title.contains('─'), "{pane:?}: {title}");
        }
        assert!(app.tabbable_panes().contains(&PaneId::Git));
        assert_eq!(
            app.panes.tiled_layout.area(PaneId::Git).width,
            60 - projects_width
        );
        let area = app.panes.project_list.body_rect;
        let header = text_row(&buffer, area, area.y - 1);
        for heading in HEADINGS {
            if heading != "Lint" || lint == LintIndicator::Enabled {
                let expected = if heading == "Projects" && header.starts_with("   Proj…") {
                    "Proj…"
                } else {
                    heading
                };
                assert!(header.contains(expected), "{header}");
            }
        }
        for y in area.y..area.y + 4 {
            let size = if y == area.y { "3 GiB" } else { "1 GiB" };
            let (x, _) =
                test_support::find_text_cell(&buffer, Rect::new(area.x, y, area.width, 1), size)
                    .context("worktree disk value")?;
            assert_eq!(x + 5, area.right() - 1, "{lint:?}");
            assert_eq!(buffer[(area.right() - 1, y)].symbol(), " ");
            assert_eq!(buffer[(area.right(), y)].symbol(), "│");
        }
        let group = text_row(&buffer, area, area.y);
        assert!(group.contains('p'), "{group}");
        if lint == LintIndicator::Enabled {
            assert!(group.contains("proo…"), "{group}");
            assert!(!group.contains('🌲'), "{group}");
        } else {
            test_support::find_text_cell(&buffer, Rect::new(area.x, area.y, area.width, 1), "… 🌲")
                .context("the group keeps its mark")?;
        }
        let primary = text_row(&buffer, area, area.y + 1);
        assert!(
            primary.contains('p') && primary.contains("… (p)"),
            "{primary}"
        );
    }
    Ok(())
}

#[test]
fn shortened_worktree_names_keep_group_and_primary_marks() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = long_worktree_app(scratch.path())?;
    for width in [60, 80] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let area = app.panes.project_list.body_rect;
        let group = text_row(&buffer, area, area.y);
        assert!(group.contains("▼ proo…"), "{group}");
        assert!(!group.contains('🌲'), "{group}");
        let header = text_row(&buffer, area, area.y - 1);
        assert!(header.starts_with("   Proj… Lint CI"), "{header}");
        let primary = text_row(&buffer, area, area.y + 1);
        assert!(primary.contains("… (p)"), "{primary}");
        let mut disk_edge = None;
        for y in area.y..area.y + 4 {
            let row = text_row(&buffer, area, y);
            assert!(row.contains('…'), "{row}");
            if y > area.y + 1 {
                assert!(!row.contains("(p)"), "{row}");
            }
            let size = if y == area.y { "3 GiB" } else { "1 GiB" };
            let (x, _) =
                test_support::find_text_cell(&buffer, Rect::new(area.x, y, area.width, 1), size)
                    .context("worktree disk value")?;
            assert_eq!(*disk_edge.get_or_insert(x + 5), x + 5);
        }
    }
    let mut entries = (0..13)
        .map(|index| {
            package(
                &scratch.path().join(format!("東group-{index}")),
                "many_worktrees",
                1 << 30,
            )
            .map(RustProject::Package)
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter();
    app.project_list = ProjectList::new(vec![RootItem::Worktrees(WorktreeGroup::new(
        entries.next().context("primary worktree")?,
        entries.collect(),
    ))]);
    app.ensure_visible_rows_cached();
    for (width, count_draws) in [(60, false), (80, false), (95, false), (110, true)] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let area = app.panes.project_list.body_rect;
        let group = text_row(&buffer, area, area.y);
        let prefix = group
            .find("▶ ")
            .with_context(|| format!("whole group prefix at {width}: {group}"))?;
        let name_end = if width <= 80 {
            assert!(!group.contains('🌲'), "{width}: {group}");
            group.find('…').context("cut group name")? + '…'.len_utf8()
        } else {
            group
                .find(" 🌲")
                .with_context(|| format!("spaced tree mark: {group}"))?
        };
        let name = &group[prefix + "▶ ".len()..name_end];
        assert!(name.chars().any(char::is_alphabetic), "{width}: {group}");
        assert!(name.ends_with('…'), "{width}: {group}");
        if count_draws {
            test_support::find_text_cell(
                &buffer,
                Rect::new(area.x, area.y, area.width, 1),
                " 🌲:13",
            )
            .with_context(|| format!("whole group count: {group}"))?;
        } else {
            assert!(!group.contains(":13"), "{width}: {group}");
        }
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

#[test]
fn narrow_tree_names_keep_letters_and_align_with_the_projects_header() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut groups = Vec::new();
    for (folder, count) in [("alpha-team", 3), ("workspaces", 2)] {
        let mut entries = (0..count)
            .map(|index| {
                package(
                    &scratch
                        .path()
                        .join(folder)
                        .join(index.to_string())
                        .join("proof_crate"),
                    "proof_crate",
                    79,
                )
                .map(RustProject::Package)
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter();
        groups.push(RootItem::Worktrees(WorktreeGroup::new(
            entries.next().context("primary")?,
            entries.collect(),
        )));
    }
    let mut app = test_support::make_app(&[]);
    app.framework.color_mode = ColorMode::NoColor;
    app.config.current_mut().lint.enabled = LintIndicator::Disabled;
    app.project_list = ProjectList::new(
        std::iter::once(RootItem::Rust(RustProject::Package(package(
            &scratch.path().join("alpha"),
            "alpha",
            32 << 30,
        )?)))
        .chain(groups)
        .chain(std::iter::once(RootItem::Rust(RustProject::Package(
            package(&scratch.path().join("東京tools"), "東京tools", 1 << 30)?,
        ))))
        .collect(),
    );
    app.project_list.expanded.expand(ExpandKey::Node(1), 0);
    app.ensure_visible_rows_cached();
    let paths = app.project_list.visible_project_storage_paths();
    app.project_list
        .set_project_storage(&paths, ProjectStorage::Available(295 << 30));
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.project_list.body_rect;
    let header = test_support::find_text_cell(
        &buffer,
        Rect::new(area.x, area.y - 1, area.width, 1),
        "Projects",
    )
    .context("Projects heading")?;
    let alpha =
        test_support::find_text_cell(&buffer, Rect::new(area.x, area.y, area.width, 1), "alpha")
            .context("root name")?;
    assert_eq!(header.0, alpha.0, "the header starts at the name column");
    let ci =
        test_support::find_text_cell(&buffer, Rect::new(area.x, area.y - 1, area.width, 1), "CI")
            .context("next column")?
            .0;
    for y in area.y + 1..area.y + 5 {
        let name = text_row(&buffer, Rect::new(area.x, y, ci - area.x, 1), y);
        let cut = name
            .find('…')
            .with_context(|| format!("cut tree name: {name}: {:?}", test_support::rows(&buffer)))?;
        assert!(name[..cut].chars().any(char::is_alphabetic), "{name}");
    }
    // A group drops its count, so the name keeps more than one letter.
    for (offset, text) in [(1, "[alp… 🌲 "), (2, "p… (p)"), (5, "[wor… 🌲 ")] {
        test_support::find_text_cell(
            &buffer,
            Rect::new(area.x, area.y + offset, area.width, 1),
            text,
        )
        .with_context(|| format!("whole tree label {text}: {:?}", test_support::rows(&buffer)))?;
    }
    for width in [60, 80] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let area = app.panes.project_list.body_rect;
        test_support::find_text_cell(&buffer, area, "東")
            .with_context(|| format!("wide name keeps its first letter at {width}"))?;
    }
    for (width, count_draws) in [(95, false), (110, true)] {
        let buffer = test_support::draw_ui(&mut app, width)?;
        let area = app.panes.project_list.body_rect;
        if count_draws {
            test_support::find_text_cell(&buffer, area, " 🌲:3")
                .context("the group count draws when its slot fits")?;
        } else {
            let group = text_row(&buffer, area, area.y + 1);
            assert!(!group.contains(":3"), "{width}: {group}");
            assert!(group.contains("▼ "), "{width}: {group}");
            assert!(group.contains("[alp… 🌲"), "{width}: {group}");
        }
    }

    Ok(())
}
