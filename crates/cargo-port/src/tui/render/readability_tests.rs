use std::path::Path;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;

use crate::config::LintIndicator;
use crate::http::RateLimitQuota;
use crate::project::AheadBehind;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::GitStatus;
use crate::project::Package;
use crate::project::ResolvedBranch;
use crate::project::RustProject;
use crate::scan::ProjectStorage;
use crate::sccache::SccacheSummary;
use crate::sccache::StatsResult;
use crate::tui::app::App;
use crate::tui::panes;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::GitProbe;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackagePresence;
use crate::tui::panes::PackageVersion;
use crate::tui::panes::PaneId;
use crate::tui::panes::WorktreeInfo;
use crate::tui::project_list::ExpandKey;
use crate::tui::sccache;
use crate::tui::test_support;
use crate::tui::test_support::PROJECT;
use crate::tui::test_support::WIDTH;

#[test]
fn targets_keep_the_cursor_across_full_frames() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    app.set_focus_to_pane(PaneId::Targets);
    test_support::draw_ui(&mut app, WIDTH)?;
    app.panes.targets.viewport.down();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let first = test_support::position(&buffer, "first")?;
    let second = test_support::position(&buffer, "second")?;
    assert_eq!(app.panes.targets.viewport.pos(), 1);
    assert_ne!(buffer[first].bg, buffer[second].bg);
    Ok(())
}

#[test]
fn narrow_layout_keeps_git_values_and_hides_unreadable_panes() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        checkout: GitCheckout::Branch {
            name:     "fleet".to_string(),
            relation: HeadRelation::AwaitingRepoProbe,
        },
        ..crate::tui::panes::GitData::default()
    });
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let git = app.panes.tiled_layout.area(PaneId::Git);
    let git_rows = (git.y..git.bottom())
        .map(|y| {
            (git.x..git.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(
        git_rows
            .iter()
            .any(|row| row.contains("Branch") && row.contains("fleet")),
        "{git_rows:?}"
    );
    let lang = app.panes.tiled_layout.area(PaneId::Lang);
    assert!(
        lang.is_empty(),
        "Languages cannot fit useful rows at 80 columns: {lang:?}"
    );
    assert!(!app.tabbable_panes().contains(&PaneId::Lang));
    assert!(app.panes.lang.viewport.content_area().is_empty());
    Ok(())
}

#[test]
fn narrow_layout_moves_focus_once_and_restores_panes_without_moving_focus() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    test_support::draw_ui(&mut app, WIDTH)?;
    app.set_focus_to_pane(PaneId::Lang);
    test_support::draw_ui(&mut app, 80)?;
    assert_ne!(app.focused_pane_id(), PaneId::Lang);
    let focus = app.focused_pane_id();
    test_support::draw_ui(&mut app, WIDTH)?;
    assert!(!app.panes.tiled_layout.area(PaneId::Lang).is_empty());
    assert_eq!(app.focused_pane_id(), focus);
    Ok(())
}

#[test]
fn narrowing_moves_cpu_focus_to_a_drawn_pane() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.panes.targets.set_content(test_support::target_data());
    test_support::draw_ui(&mut app, 95)?;
    app.set_focus_to_pane(PaneId::Cpu);
    assert_eq!(app.focused_pane_id(), PaneId::Cpu);
    let buffer = test_support::draw_ui(&mut app, 80)?;
    assert!(app.panes.tiled_layout.area(PaneId::Cpu).is_empty());
    assert_ne!(app.focused_pane_id(), PaneId::Cpu);
    let focus = app.focused_pane_id();
    let area = app.panes.tiled_layout.area(focus);
    assert!(!area.is_empty());
    assert!((area.x..area.right()).any(|x| buffer[(x, area.y)].symbol() != " "));
    test_support::draw_ui(&mut app, 95)?;
    assert_eq!(app.focused_pane_id(), focus);
    assert!(!app.panes.tiled_layout.area(PaneId::Cpu).is_empty());
    Ok(())
}

#[test]
fn status_names_the_disk_cache() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    sccache::apply_summary(
        &mut app,
        SccacheSummary::Reported {
            cache_size: "101 GiB".to_string(),
            hit_rate:   "4.90 %".to_string(),
        },
    );
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    test_support::position(&buffer, "Disk cache")?;
    Ok(())
}

#[test]
fn no_lint_runs_appears_once() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    assert_eq!(
        test_support::rows(&buffer)
            .join("\n")
            .matches("No lint runs")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn narrow_workspace_shows_path_and_disk_values() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        name: PROJECT.to_string(),
        path: "/tmp/workspace".to_string(),
        disk: Some(73),
        ..crate::tui::panes::PackageData::default()
    });
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.tiled_layout.area(PaneId::Package);
    let path = test_support::find_text_cell(&buffer, area, "Path").context("the Path label")?;
    let disk = test_support::find_text_cell(&buffer, area, "Disk").context("the Disk label")?;
    let path_text: String = (path.0..area.right())
        .map(|x| buffer[(x, path.1)].symbol())
        .collect();
    let disk_text: String = (disk.0..area.right())
        .map(|x| buffer[(x, disk.1)].symbol())
        .collect();
    assert!(path_text.contains("/tmp"));
    assert!(disk_text.contains("73 B"));
    Ok(())
}

#[test]
fn a_git_description_does_not_share_the_frame_with_no_description() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        description: Some("A collection of useful development tools".to_string()),
        ..GitData::default()
    });
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    test_support::position(&buffer, "A collection")?;
    let text = test_support::rows(&buffer).join("\n");
    assert!(!text.contains("No description"), "{text}");
    Ok(())
}

#[test]
fn inactive_rows_have_no_enter_activation_hint() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.set_focus_to_pane(PaneId::Lang);
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let rows = test_support::rows(&buffer);
    let status = rows.last().context("the status row")?;
    assert!(
        !status.to_lowercase().contains("enter activate"),
        "{status}"
    );
    Ok(())
}

#[test]
fn an_open_popup_clears_or_dims_every_background_cell() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    for width in [80, WIDTH] {
        test_support::draw_ui(&mut app, width)?;
        app.overlays.open_sccache();
        let request = app.overlays.sccache_pane.start_loading();
        app.overlays.sccache_pane.apply_result(
            request,
            StatsResult::Ready(vec!["Cache hits  7".to_string()]),
        );
        let buffer = test_support::draw_ui(&mut app, width)?;
        let (_, title_y) = test_support::position(&buffer, "Sccache stats")?;
        let (_, hint_y) = test_support::position(&buffer, "Esc close")?;
        let inner = app.overlays.sccache_pane.viewport.content_area();
        let popup =
            ratatui::layout::Rect::new(inner.x - 1, title_y, inner.width + 2, hint_y - title_y + 2);
        for y in buffer.area.y..buffer.area.bottom().saturating_sub(1) {
            for x in buffer.area.x..buffer.area.right() {
                if !popup.contains((x, y).into()) {
                    let cell = &buffer[(x, y)];
                    assert!(
                        cell.symbol().trim().is_empty()
                            || cell.modifier.contains(ratatui::style::Modifier::DIM),
                        "popup background cell {x},{y}: {cell:?}"
                    );
                }
            }
        }
        app.overlays.close_sccache();
    }
    Ok(())
}

#[test]
fn popup_keeps_the_status_row_and_a_toast() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.show_timed_toast("Repair notice", "The scan is complete");
    app.overlays.open_sccache();
    let request = app.overlays.sccache_pane.start_loading();
    app.overlays.sccache_pane.apply_result(
        request,
        StatsResult::Ready(vec!["Cache hits  7".to_string()]),
    );
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    test_support::position(&buffer, "Uptime:")?;
    test_support::position(&buffer, "Repair notice")?;
    Ok(())
}

#[test]
fn narrow_workspace_focus_selects_a_drawn_field() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        path: "/tmp/workspace".to_string(),
        disk: Some(73),
        ..PackageData::default()
    });
    app.panes.git.set_content(GitData {
        description: Some("A fleet of useful tools".to_string()),
        ..GitData::default()
    });
    app.set_focus_to_pane(PaneId::Package);
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.tiled_layout.area(PaneId::Package);
    let path = test_support::find_text_cell(&buffer, area, "Path").context("the Path row")?;
    assert_ne!(buffer[path].bg, ratatui::style::Color::Reset);
    assert_eq!(
        buffer[path].fg,
        crate::tui::theme_roles::selection_style(
            tui_pane::PaneSelectionState::Active,
            app.framework.color_mode,
        )
        .fg
        .context("selection text")?
    );
    Ok(())
}

#[test]
fn a_cpu_pane_too_short_for_bars_keeps_the_body_page_mark() -> anyhow::Result<()> {
    let mut app = test_support::sampled_cpu_app(64);
    let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Cpu, 24, 11)?;
    let (_, y) = test_support::position(&buffer, "▼")?;
    assert!(y > 0 && y < buffer.area.bottom() - 1);
    assert!(
        !test_support::rows(&buffer)
            .iter()
            .any(|row| row.contains('█'))
    );
    test_support::position(&buffer, "CPU cores")?;
    Ok(())
}

fn app_with_narrow_detail_fields() -> App {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.config.current_mut().lint.enabled = LintIndicator::Enabled;
    app.ensure_fit_widths_cached();
    app.project_list
        .cached_fit_widths
        .observe(crate::tui::columns::COL_NAME, 100);
    app.panes.package.set_content(PackageData {
        path: "/tmp/long-package-path".to_string(),
        disk: Some(26_521_488_179),
        in_project_target: Some(25_769_803_776),
        in_project_non_target: Some(751_684_403),
        types: Some(vec![crate::project::ProjectType::Binary]),
        version: PackageVersion::Manifest("0.1.0".to_string()),
        package_presence: PackagePresence::Present,
        ..PackageData::default()
    });
    app.panes.git.set_content(GitData {
        checkout: GitCheckout::Branch {
            name:     "master".to_string(),
            relation: HeadRelation::Default,
        },
        status: Some(GitStatus::Clean),
        inception: FirstCommit::Probed(CommitDate::At("2026-11-06 08:21".to_string())),
        ..GitData::default()
    });
    app
}

fn assert_visible_field_is_whole(
    buffer: &Buffer,
    area: Rect,
    label: &str,
    value: &str,
) -> anyhow::Result<()> {
    if let Some((_, y)) = test_support::find_text_cell(buffer, area, label) {
        let value_area = ratatui::layout::Rect::new(area.x, y + 1, area.width, 1);
        assert!(y + 1 < area.bottom() - 1, "{label} has no value row");
        test_support::find_text_cell(buffer, value_area, value).context("value below its label")?;
    }
    if let Some((_, y)) = test_support::find_text_cell(buffer, area, value) {
        assert!(y > area.y + 1, "{value} has no label row");
        let label_area = ratatui::layout::Rect::new(area.x, y - 1, area.width, 1);
        test_support::find_text_cell(buffer, label_area, label).context("label above its value")?;
    }
    Ok(())
}

#[test]
fn package_pages_keep_wrapped_labels_with_values_at_eighty_columns() -> anyhow::Result<()> {
    let mut app = app_with_narrow_detail_fields();
    app.set_focus_to_pane(PaneId::Package);
    let rows =
        panes::package_rows_from_data(app.panes.package.content().context("Package content")?);
    let mut saw_type = false;
    let mut saw_version = false;
    for index in 0..rows.len() {
        app.panes.package.viewport.set_pos(index);
        let buffer = test_support::draw_ui(&mut app, 80)?;
        let area = app.panes.tiled_layout.area(PaneId::Package);
        assert!(area.width <= 17);
        assert_visible_field_is_whole(&buffer, area, "Type", "binary")?;
        assert_visible_field_is_whole(&buffer, area, "Version", "0.1.0")?;
        saw_type |= test_support::find_text_cell(&buffer, area, "binary").is_some();
        saw_version |= test_support::find_text_cell(&buffer, area, "0.1.0").is_some();
    }
    assert!(saw_type && saw_version);
    Ok(())
}

#[test]
fn git_pages_keep_wrapped_labels_with_values_at_eighty_columns() -> anyhow::Result<()> {
    let mut app = app_with_narrow_detail_fields();
    app.set_focus_to_pane(PaneId::Git);
    let fields = panes::git_fields_from_data(app.panes.git.content().context("Git content")?);
    let mut saw_status = false;
    for index in 0..fields.len() {
        app.panes.git.viewport.set_pos(index);
        let buffer = test_support::draw_ui(&mut app, 80)?;
        let area = app.panes.tiled_layout.area(PaneId::Git);
        assert!(area.width <= 17);
        assert_visible_field_is_whole(&buffer, area, "Status", "✨ clean")?;
        saw_status |= test_support::find_text_cell(&buffer, area, "✨ clean").is_some();
    }
    assert!(saw_status);
    Ok(())
}

#[test]
fn git_pages_keep_table_headers_with_the_first_worktree() -> anyhow::Result<()> {
    let mut app = app_with_narrow_detail_fields();
    let data = app.panes.git.content().context("Git content")?;
    let mut data = data.clone();
    data.worktrees = ["master", "verify", "sdlc"]
        .into_iter()
        .map(|branch| WorktreeInfo {
            name:         "proof_crate".to_string(),
            path:         "/tmp/proof".to_string(),
            branch:       Some(branch.to_string()),
            tracked:      None,
            ahead_behind: Comparison::NotCompared,
        })
        .collect();
    let fields = panes::git_fields_from_data(&data);
    app.panes.git.set_content(data);
    app.set_focus_to_pane(PaneId::Git);
    let mut saw_header = false;
    for index in 0..fields.len() + 3 {
        app.panes.git.viewport.set_pos(index);
        let buffer = test_support::draw_ui(&mut app, 80)?;
        let area = app.panes.tiled_layout.area(PaneId::Git);
        if let Some((_, y)) = test_support::find_text_cell(&buffer, area, "Name") {
            saw_header = true;
            assert!(y + 1 < area.bottom() - 1);
            test_support::find_text_cell(
                &buffer,
                ratatui::layout::Rect::new(area.x, y + 1, area.width, 1),
                "master",
            )
            .context("first worktree below the header")?;
        }
    }
    assert!(saw_header);
    Ok(())
}

fn draw_short_ui(app: &mut App) -> anyhow::Result<Buffer> {
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
    terminal.draw(|frame| super::ui(frame, app))?;
    Ok(terminal.backend().buffer().clone())
}

#[test]
fn one_line_git_fields_keep_whole_labels_and_shortened_values() -> anyhow::Result<()> {
    let mut app = app_with_narrow_detail_fields();
    let mut data = app.panes.git.content().context("Git content")?.clone();
    data.description = Some("a fleet of cargo tools for projects, tests, builds, caches, worktrees and continuous integration".to_string());
    data.checkout = GitCheckout::Branch {
        name:     "cargo-liner-port".to_string(),
        relation: HeadRelation::Feature,
    };
    data.vs_local = GitProbe::Probed(Comparison::Counted(AheadBehind {
        ahead:  34,
        behind: 11,
    }));
    data.local_main_branch = GitProbe::Probed(ResolvedBranch::Named("main".to_string()));
    let quota = RateLimitQuota {
        limit:     5000,
        used:      149,
        remaining: 4851,
        reset_at:  None,
    };
    data.rate_limit_core = Some(quota);
    data.rate_limit_graphql = Some(quota);
    let fields = panes::git_fields_from_data(&data);
    app.panes.git.set_content(data);
    app.set_focus_to_pane(PaneId::Git);
    for (index, field) in fields.iter().enumerate() {
        app.panes.git.viewport.set_pos(index + 1);
        let buffer = draw_short_ui(&mut app)?;
        let area = app.panes.git.viewport.content_area();
        assert_eq!(area.height, 1, "the description leaves one Git row");
        let (x, y) =
            test_support::find_text_cell(&buffer, area, field.label()).with_context(|| {
                format!(
                    "whole label {}: {:?}",
                    field.label(),
                    test_support::rows(&buffer)
                )
            })?;
        let value_x = x + u16::try_from(field.label().len())?;
        let value = (value_x..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        assert!(
            !value.trim().is_empty(),
            "{} has no value: {value:?}",
            field.label()
        );
        if matches!(field, panes::DetailField::Head) {
            assert!(value.trim_end().ends_with('…'), "{value:?}");
        }
    }
    Ok(())
}

fn app_with_paged_worktrees(root: &Path) -> anyhow::Result<App> {
    let mut projects = Vec::new();
    for branch in ["master", "verify-branch", "sdlc-merge"] {
        let path = root.join(format!("proof_crate-{branch}-with-a-long-name"));
        std::fs::create_dir_all(&path)?;
        std::fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"proof_crate\"\nversion = \"0.1.0\"\n",
        )?;
        let mut package = Package {
            path: crate::project::AbsolutePath::from(path),
            name: Some("proof_crate [claude-build-followups-sdlc-merge-gate/scripts/aws_target/production]".to_string()),
            ..crate::project::Package::default()
        };
        package.disk_usage_bytes = Some(1 << 30);
        projects.push(RustProject::Package(package));
    }
    let mut projects = projects.into_iter();
    let primary = projects.next().context("primary worktree")?;
    let mut app = app_with_narrow_detail_fields();
    app.panes.cpu = test_support::sampled_cpu_app(8).panes.cpu;
    app.project_list = crate::tui::project_list_state::ProjectList::new(vec![
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::new(
            primary,
            projects.collect(),
        )),
    ]);
    app.project_list.expanded.expand(ExpandKey::Node(0), 0);
    app.ensure_visible_rows_cached();
    let paths = app.project_list.visible_project_storage_paths();
    app.project_list
        .set_project_storage(&paths, ProjectStorage::Available(4 << 30));
    app.panes.package.set_content(PackageData {
        title: "Project".to_string(),
        name: "proof_crate".to_string(),
        path: root.display().to_string(),
        disk: Some(26_521_488_179),
        ..PackageData::default()
    });
    let mut data = app.panes.git.content().context("Git content")?.clone();
    data.worktrees = ["master", "verify-branch", "sdlc-merge"]
        .into_iter()
        .map(|branch| WorktreeInfo {
            name:         format!("proof_crate-{branch}-with-a-long-name"),
            path:         root
                .join(format!("proof_crate-{branch}-with-a-long-name"))
                .display()
                .to_string(),
            branch:       Some(branch.to_string()),
            tracked:      None,
            ahead_behind: Comparison::NotCompared,
        })
        .collect();
    data.last_commit = GitProbe::Probed(CommitDate::At("2026-11-06 08:21".to_string()));
    app.panes.git.set_content(data);
    Ok(app)
}

fn assert_pane_page(buffer: &Buffer, area: Rect, expected: &str) {
    let row: String = (area.x..area.right())
        .map(|x| buffer[(x, area.bottom() - 1)].symbol())
        .collect();
    assert!(row.contains(expected), "{row}: expected {expected}");
    if !expected.contains('▼') {
        assert!(!row.contains('▼'), "the last page has no rows below: {row}");
    }
}

#[test]
fn git_worktree_pages_keep_the_divider_title_and_drawn_page_number() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_paged_worktrees(scratch.path())?;
    app.set_focus_to_pane(PaneId::Git);
    app.panes.git.viewport.home();
    let first = draw_short_ui(&mut app)?;
    let area = app.panes.tiled_layout.area(PaneId::Git);
    assert_eq!(area.width, 16, "the full UI keeps the narrow Git pane");
    assert_pane_page(&first, area, "1 of 2 ▼");
    let first_worktree =
        panes::git_fields_from_data(app.panes.git.content().context("Git content")?).len();
    app.panes.git.viewport.set_pos(first_worktree - 1);
    let second = draw_short_ui(&mut app)?;
    let title = test_support::find_text_cell(&second, area, "Worktrees (3)")
        .with_context(|| format!("worktree divider: {:?}", test_support::rows(&second)))?;
    assert_eq!(second[(area.x, title.1)].symbol(), "├");
    assert_eq!(second[(area.right() - 1, title.1)].symbol(), "┤");
    assert_pane_page(&second, area, "▲ 2 of 2");
    test_support::find_text_cell(&second, area, "master").context("worktree table on page 2")?;
    app.panes.git.viewport.home();
    let first_again = draw_short_ui(&mut app)?;
    assert_pane_page(&first_again, area, "1 of 2 ▼");
    Ok(())
}

#[test]
fn package_pages_name_the_drawn_page_in_both_directions() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut app = app_with_paged_worktrees(scratch.path())?;
    app.set_focus_to_pane(PaneId::Package);
    app.panes.package.viewport.home();
    let first = draw_short_ui(&mut app)?;
    let area = app.panes.tiled_layout.area(PaneId::Package);
    assert_pane_page(&first, area, "1 of 2 ▼");
    app.panes.package.viewport.end();
    let second = draw_short_ui(&mut app)?;
    assert_pane_page(&second, area, "▲ 2 of 2");
    test_support::find_text_cell(&second, area, "CI").context("Package fields on page 2")?;
    app.panes.package.viewport.home();
    let first_again = draw_short_ui(&mut app)?;
    assert_pane_page(&first_again, area, "1 of 2 ▼");
    Ok(())
}
