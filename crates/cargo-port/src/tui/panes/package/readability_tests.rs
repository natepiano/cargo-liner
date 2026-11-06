use std::path::Path;

use tui_pane::PaneFocusState;

use crate::config::NonRustInclusion;
use crate::project::CratesIoRelease;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::tui::panes::GitData;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackageSection;
use crate::tui::panes::PaneId;
use crate::tui::panes::pane_data::WorktreeGroupSummary;
use crate::tui::test_support;
use crate::tui::test_support::PROJECT;
use crate::tui::test_support::PROJECT_PATH;

#[test]
fn deleted_non_rust_details_keep_the_project_fields() -> anyhow::Result<()> {
    let project = RootItem::NonRust(crate::project::NonRustProject::new(
        crate::project::AbsolutePath::from(PROJECT_PATH),
        Some(PROJECT.to_string()),
    ));
    let mut config = crate::config::CargoPortConfig::default();
    config.tui.include_non_rust = NonRustInclusion::Include;
    let mut app = test_support::make_app_with_config(&[project], &config);
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    assert!(
        !test_support::rows(&buffer)
            .iter()
            .any(|row| row.split_whitespace().any(|word| word == "Type")),
        "a deleted non-Rust project must not gain a Rust Type row"
    );
    test_support::position(&buffer, &format!("Project - {PROJECT} (deleted)"))?;
    assert!(
        !test_support::rows(&buffer)
            .iter()
            .any(|row| row.contains("Targets:"))
    );
    Ok(())
}

#[test]
fn narrow_package_disk_values_share_one_column() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        disk: Some(73),
        in_project_target: Some(0),
        in_project_non_target: Some(73),
        ..crate::tui::panes::PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 20)?;
    let disk = test_support::position(&buffer, "73 B")?;
    let target = test_support::position(&buffer, "0 B")?;
    assert_eq!(disk.0, target.0);
    Ok(())
}

#[test]
fn package_values_end_with_an_ellipsis_when_they_do_not_fit() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        path: "/workspace/a-long-project-path/with-a-visible-end".to_string(),
        ..crate::tui::panes::PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 30)?;
    let (_, y) = test_support::position(&buffer, "Path")?;
    let row = test_support::rows(&buffer)[usize::from(y)].clone();
    assert!(
        row.trim_end_matches(['│', ' ']).ends_with('…'),
        "cut value needs a mark: {row}"
    );
    Ok(())
}

#[test]
fn deleted_package_details_name_the_state_and_clear_disk_breakdown() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let info = app
        .project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project");
    info.visibility = Visibility::Deleted;
    info.disk_usage_bytes = Some(0);
    info.in_project_non_target = Some(74);
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 60)?;
    test_support::position(&buffer, &format!("Package - {PROJECT} (deleted)"))?;
    assert!(
        !test_support::rows(&buffer)
            .iter()
            .any(|row| row.contains("74 B"))
    );
    Ok(())
}

#[test]
fn structure_uses_the_same_binary_word_as_package() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        types: Some(vec![crate::project::ProjectType::Binary]),
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 1)],
        ..crate::tui::panes::PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    assert_eq!(
        test_support::rows(&buffer)
            .iter()
            .filter(|row| row.contains("binary"))
            .count(),
        2
    );
    Ok(())
}

#[test]
fn package_group_sections_use_sentence_case() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        worktree_group_summary: Some(WorktreeGroupSummary {
            worktrees: 1,
            deleted:   0,
        }),
        primary_section: Some(PackageSection::PrimaryPackage),
        ..crate::tui::panes::PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    test_support::position(&buffer, "Worktree group summary")?;
    test_support::position(&buffer, "Primary package")?;
    Ok(())
}

#[test]
fn empty_description_is_one_marked_row() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 20)?;
    let matching: Vec<_> = test_support::rows(&buffer)
        .into_iter()
        .filter(|row| row.contains("No description"))
        .collect();
    assert_eq!(matching.len(), 1);
    assert!(matching[0].trim_end_matches(['│', ' ']).ends_with('…'));
    assert!(
        !test_support::rows(&buffer)
            .iter()
            .any(|row| row.contains("available"))
    );
    Ok(())
}

#[test]
fn compact_target_disk_value_stays_on_its_label_row() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        disk: Some(73),
        in_project_target: Some(0),
        in_project_non_target: Some(73),
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 18)?;
    let (_, y) = test_support::position(&buffer, "target/")?;
    assert!(test_support::rows(&buffer)[usize::from(y)].contains("0 B"));
    Ok(())
}

#[test]
fn package_draws_sentence_case_release_and_disk_labels() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.project_list.handle_crates_io_version_msg(
        Path::new(PROJECT_PATH),
        CratesIoRelease {
            version:    "1.2.3".to_string(),
            prerelease: None,
            downloads:  1234,
        },
    );
    let info = app
        .project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("fixture project");
    info.in_project_target = Some(0);
    info.in_project_non_target = Some(73);
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    for label in ["Version", "Downloads", "Other"] {
        test_support::position(&buffer, label)?;
    }
    Ok(())
}

#[test]
fn selected_descriptions_use_one_text_color_and_fill_each_row() -> anyhow::Result<()> {
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(test_support::shipped_dark_theme()?));
    let result = (|| -> anyhow::Result<()> {
        for focus in [PaneFocusState::Active, PaneFocusState::Remembered] {
            for pane in [PaneId::Package, PaneId::Git] {
                let mut app = test_support::project_app();
                if pane == PaneId::Git {
                    app.panes.git.set_content(GitData {
                        description: Some(
                            "A fleet of tools with a long description for several rows".to_string(),
                        ),
                        ..crate::tui::panes::GitData::default()
                    });
                    app.panes.git.focus.pane_focus_state = focus;
                } else {
                    app.panes.package.focus.pane_focus_state = focus;
                }
                let buffer = test_support::draw_pane(&mut app, pane, 24)?;
                let word = if pane == PaneId::Git {
                    "A fleet"
                } else {
                    "No description"
                };
                let failures =
                    test_support::contrast_failures(&buffer, &[word], "selected description");
                assert!(failures.is_empty(), "{}", failures.join("\n"));
                let (_, y) = test_support::position(&buffer, word)?;
                let ground = buffer[(2, y)].bg;
                for row in y..buffer.area.bottom() - 1 {
                    if buffer[(2, row)].bg != ground {
                        break;
                    }
                    for x in 1..buffer.area.right() - 1 {
                        assert_eq!(
                            buffer[(x, row)].bg,
                            ground,
                            "description fill {pane:?} {focus:?} at {x},{row}"
                        );
                        assert_eq!(
                            buffer[(x, row)].fg,
                            ratatui::style::Color::Rgb(255, 255, 255)
                        );
                    }
                }
            }
        }
        Ok(())
    })();
    tui_pane::set_active_theme(previous);
    result
}
