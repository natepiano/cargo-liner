use std::path::Path;

use anyhow::Context;
use ratatui::layout::Position;
use tui_pane::ColorMode;
use tui_pane::Hittable;
use tui_pane::PaneFocusState;

use crate::config::NonRustInclusion;
use crate::project::CratesIoRelease;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::tui::panes;
use crate::tui::panes::GitData;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackagePresence;
use crate::tui::panes::PackageSection;
use crate::tui::panes::PackageVersion;
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

#[test]
fn narrow_package_structure_follows_path_and_disk() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        path: "/tmp/workspace".to_string(),
        disk: Some(73),
        types: Some(vec![crate::project::ProjectType::Binary]),
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 2)],
        test_rows: vec![("Total", 1)],
        crates_io_rows: vec![("Version", "1.2.3".to_string())],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    let area = buffer.area;
    let drawn: Vec<String> = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    let path = test_support::find_text_cell(&buffer, area, "Path")
        .ok_or_else(|| anyhow::anyhow!("the Path row"))?;
    let disk = test_support::find_text_cell(&buffer, area, "Disk")
        .ok_or_else(|| anyhow::anyhow!("the Disk row"))?;
    let structure = test_support::find_text_cell(&buffer, area, "Structure")
        .ok_or_else(|| anyhow::anyhow!("the Structure section: {drawn:?}"))?;
    assert!(
        structure.0 > path.0 && structure.0 > disk.0,
        "Structure must label only its own column"
    );
    let divider = (path.0 + 1..structure.0)
        .find(|x| buffer[(*x, disk.1)].symbol() == "│")
        .ok_or_else(|| anyhow::anyhow!("the divider beside the metadata: {drawn:?}"))?;
    assert_eq!(buffer[(divider, structure.1)].symbol(), "┬");
    for title in ["Tests", "crates.io"] {
        let section = test_support::find_text_cell(&buffer, area, title)
            .ok_or_else(|| anyhow::anyhow!("the {title} section"))?;
        assert_eq!(
            section.0, structure.0,
            "section titles must share one column"
        );
    }
    Ok(())
}

#[test]
fn package_keeps_statistics_and_release_values_at_forty_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        title: "Package".to_string(),
        name: "alpha".to_string(),
        path: "/tmp/alpha".to_string(),
        disk: Some(73),
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 1)],
        crates_io_rows: vec![
            ("Version", "0.0.0".to_string()),
            ("Downloads", "53".to_string()),
        ],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 40)?;
    let path = test_support::position(&buffer, "Path")?;
    let structure = test_support::position(&buffer, "Structure")?;
    let (divider, y) = test_support::position(&buffer, "┬")?;
    assert_eq!(y, structure.1);
    assert!(path.0 < divider && divider < structure.0);
    assert_eq!(buffer[(divider, path.1)].symbol(), "│");
    let release = test_support::position(&buffer, "crates.io")?;
    assert_eq!(release.0, structure.0);
    for (label, value) in [("binary", "1"), ("Version", "0.0.0"), ("Downloads", "53")] {
        let label = test_support::position(&buffer, label)?;
        let row = &test_support::rows(&buffer)[usize::from(label.1)];
        assert!(label.0 > divider);
        assert!(row.trim_end_matches(['│', ' ']).ends_with(value), "{row}");
    }
    Ok(())
}

#[test]
fn narrow_package_has_one_column_and_keeps_whole_disk_values() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        disk: Some(25_662_849_843),
        in_project_target: Some(0),
        types: Some(vec![crate::project::ProjectType::Binary]),
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 1)],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 21)?;
    let disk = test_support::position(&buffer, "Disk")?;
    let value = test_support::position(&buffer, "23.9 GiB")?;
    assert_eq!(disk.1, value.1);
    let project_type = test_support::position(&buffer, "Type")?;
    assert!(test_support::rows(&buffer)[usize::from(project_type.1)].contains("binary"));
    for y in 1..buffer.area.bottom() - 1 {
        for x in 1..buffer.area.right() - 1 {
            assert_ne!(buffer[(x, y)].symbol(), "│");
        }
    }
    Ok(())
}

#[test]
fn package_without_description_titles_the_structure_column() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        description: Some("The repository description".to_string()),
        ..GitData::default()
    });
    app.panes.package.set_content(PackageData {
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 1)],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    let (divider, y) = test_support::position(&buffer, "┬")?;
    let (title, title_y) = test_support::position(&buffer, "Structure")?;
    assert_eq!(y, 0);
    assert_eq!(title_y, y);
    assert!(title > divider);
    assert_eq!(buffer[(divider, 1)].symbol(), "│");
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        name: "cargo-liner-port".to_string(),
        stats_rows: vec![(crate::tui::constants::TARGET_KIND_BIN_LABEL, 1)],
        test_rows: vec![("unit", 5782)],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 49)?;
    let (divider, y) = test_support::position(&buffer, "┬")?;
    let structure = test_support::position(&buffer, "Structure")?;
    let tests = test_support::position(&buffer, "Tests")?;
    assert_eq!(structure.0, tests.0);
    assert_eq!(y, 0);
    assert_eq!(buffer[(divider, 1)].symbol(), "│");
    let title: String = (1..divider - 1).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(title.trim_end().ends_with('…'), "{title}");
    assert!(matches!(buffer[(divider - 1, 0)].symbol(), "─" | " "));
    Ok(())
}

#[test]
fn package_section_rules_join_the_right_border() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.package.set_content(PackageData {
        crates_io_rows: vec![("Version", "1.2.3".to_string())],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    let (_, y) = test_support::position(&buffer, "crates.io")?;
    assert_eq!(buffer[(79, y)].symbol(), "┤");
    Ok(())
}

#[test]
fn narrow_package_fields_keep_values_and_wrap_section_words() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        types: Some(vec![crate::project::ProjectType::Binary]),
        version: PackageVersion::Manifest("0.1.0".to_string()),
        package_presence: PackagePresence::Present,
        disk: Some(73),
        in_project_target: Some(0),
        in_project_non_target: Some(73),
        primary_section: Some(PackageSection::PrimaryPackage),
        worktree_group_summary: Some(WorktreeGroupSummary {
            worktrees: 3,
            deleted:   0,
        }),
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    for text in [
        "Type",
        "binary",
        "Version",
        "0.1.0",
        "Primary",
        "package",
        "Worktree group",
        "summary",
        "Disk",
        "target/",
        "Other",
        "73 B",
        "0 B",
    ] {
        test_support::position(&buffer, text)?;
    }
    let primary = test_support::position(&buffer, "Primary")?;
    assert!(test_support::rows(&buffer)[usize::from(primary.1 + 1)].contains("package"));
    let group = test_support::position(&buffer, "Worktree group")?;
    assert!(test_support::rows(&buffer)[usize::from(group.1 + 1)].contains("summary"));
    Ok(())
}

#[test]
fn wrapped_package_values_share_selection_and_row_hits() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        types: Some(vec![crate::project::ProjectType::Binary]),
        ..PackageData::default()
    });
    app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
    test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    let rows =
        panes::package_rows_from_data(app.panes.package.content().context("Package content")?);
    let index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                crate::tui::panes::PackageRow::Field(crate::tui::panes::DetailField::Targets)
            )
        })
        .context("Type row")?;
    app.panes.package.viewport.set_pos(index);
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    let label = test_support::position(&buffer, "Type")?;
    let value = test_support::position(&buffer, "binary")?;
    assert_eq!(value.1, label.1 + 1);
    assert_eq!(buffer[label].bg, buffer[value].bg);
    assert_eq!(buffer[label].fg, buffer[value].fg);
    for (x, y) in [label, value] {
        assert!(matches!(app.panes.package.hit_test_at(Position::new(x, y)),
            Some(crate::tui::hit_test::HoverTarget::PaneRow { pane: PaneId::Package, row }) if row == index));
    }
    Ok(())
}
