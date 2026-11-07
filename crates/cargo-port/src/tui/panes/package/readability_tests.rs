use std::path::Path;

use anyhow::Context;
use ratatui::layout::Position;
use tui_pane::ColorMode;
use tui_pane::Hittable;
use tui_pane::PaneFocusState;

use crate::ci::CiStatus;
use crate::config::NonRustInclusion;
use crate::project::CratesIoRelease;
use crate::project::DiskMeasurement;
use crate::project::GithubRunCount;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::scan::BackgroundMsg;
use crate::tui::panes;
use crate::tui::panes::GitData;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackagePresence;
use crate::tui::panes::PackageSection;
use crate::tui::panes::PackageVersion;
use crate::tui::panes::PaneId;
use crate::tui::panes::pane_data::PackageDeletion;
use crate::tui::panes::pane_data::WorktreeGroupSummary;
use crate::tui::render;
use crate::tui::state::CiDisplay;
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
        disk: DiskMeasurement::Measured {
            total:  73,
            target: 0,
            other:  73,
        },
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
    info.disk = DiskMeasurement::Measured {
        total:  0,
        target: 0,
        other:  74,
    };
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
    assert!(
        matching[0]
            .trim_end_matches(['│', ' '])
            .ends_with("No description")
    );
    assert!(!matching[0].contains('…'));
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
        disk: DiskMeasurement::Measured {
            total:  73,
            target: 0,
            other:  73,
        },
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
    info.disk = DiskMeasurement::Measured {
        total:  73,
        target: 0,
        other:  73,
    };
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
        disk: DiskMeasurement::TotalOnly(73),
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
        disk: DiskMeasurement::TotalOnly(73),
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
        disk: DiskMeasurement::TotalOnly(25_662_849_843),
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
        disk: DiskMeasurement::Measured {
            total:  73,
            target: 0,
            other:  73,
        },
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

#[test]
fn narrow_package_keeps_both_ci_counts_and_whole_empty_messages() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        ci_display: CiDisplay::Runs {
            ci_status:    Some(CiStatus::Failed),
            local:        7,
            github_total: GithubRunCount::Known(19),
        },
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    let text = test_support::rows(&buffer).join("\n");
    let ci = test_support::position(&buffer, "CI")?;
    let ci_rows = &test_support::rows(&buffer)[usize::from(ci.1)..];
    assert!(
        ci_rows.iter().take(3).any(|row| row.contains('7')),
        "{text}"
    );
    assert!(
        ci_rows.iter().take(3).any(|row| row.contains("19")),
        "{text}"
    );
    assert!(!text.contains("No description …"), "{text}");
    app.panes.package.set_content(PackageData::default());
    let empty = test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    let rows = test_support::rows(&empty);
    assert!(
        !rows.iter().any(|row| row.contains("No CI workflo…")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("No CI") && !row.contains('…')),
        "{rows:?}"
    );
    let (x, y) = test_support::position(&empty, "No")?;
    assert_eq!(x, 2);
    assert_eq!(test_support::position(&empty, "description")?, (x, y + 1));
    assert!(!rows[usize::from(y)].contains('…'));
    assert!(!rows[usize::from(y + 1)].contains('…'));
    Ok(())
}

#[test]
fn deleted_metadata_keeps_its_style_and_accurate_description() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let data = PackageData {
        title: "Package".to_string(),
        types: Some(vec![crate::project::ProjectType::Binary]),
        version: PackageVersion::Manifest("0.1.0".to_string()),
        edition: Some("2024".to_string()),
        package_presence: PackagePresence::Present,
        description: Some("A retained description".to_string()),
        ..PackageData::default()
    };
    app.panes.package.set_content(data.clone());
    let live = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    let mut deleted = data;
    deleted.deletion = PackageDeletion::Deleted;
    app.panes.package.set_content(deleted.clone());
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    for text in ["binary", "0.1.0", "2024"] {
        let before = test_support::position(&live, text)?;
        let after = test_support::position(&buffer, text)?;
        assert_eq!(live[before].style(), buffer[after].style(), "{text}");
    }
    assert!(
        !test_support::rows(&buffer)
            .join("\n")
            .contains("No description")
    );
    deleted.description = None;
    app.panes.package.set_content(deleted);
    let empty = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    test_support::position(&empty, "No description")?;
    Ok(())
}

fn workspace_data() -> PackageData {
    PackageData {
        title: "Workspace".to_string(),
        path: "/tmp/workspace".to_string(),
        types: Some(vec![crate::project::ProjectType::Workspace]),
        disk: DiskMeasurement::Measured {
            total:  73,
            target: 0,
            other:  73,
        },
        stats_rows: vec![("members", 8), ("lib", 3), ("binary", 5)],
        test_rows: vec![("unit", 12), ("integration", 7)],
        ..PackageData::default()
    }
}

#[test]
fn one_column_workspace_pages_include_structure_and_tests() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(workspace_data());
    app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
    let mut drawn = String::new();
    for position in 0..24 {
        app.panes.package.viewport.set_pos(position);
        let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Package, 17, 12)?;
        drawn.push_str(&test_support::rows(&buffer).join("\n"));
    }
    for text in [
        "Structure",
        "members",
        "lib",
        "binary",
        "Tests",
        "unit",
        "integration",
    ] {
        assert!(drawn.contains(text), "missing {text}: {drawn}");
    }
    Ok(())
}

#[test]
fn new_package_page_keeps_a_visible_selection() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(workspace_data());
    app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
    test_support::draw_pane_at_size(&mut app, PaneId::Package, 17, 10)?;
    app.panes.package.viewport.end();
    let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Package, 17, 10)?;
    let selected = tui_pane::selection_style(PaneFocusState::Active, ColorMode::Colored)
        .bg
        .context("selection fill")?;
    assert!(
        (1..buffer.area.bottom() - 1).any(|y| (1..buffer.area.right() - 1)
            .any(|x| buffer[(x, y)].bg == selected && !buffer[(x, y)].symbol().trim().is_empty())),
        "the new page needs a selected item: {:?}",
        test_support::rows(&buffer)
    );
    Ok(())
}

#[test]
fn package_selection_stays_in_its_own_column() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(workspace_data());
    app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
    let rows = panes::package_rows_from_data(app.panes.package.content().context("package")?);
    let path_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                crate::tui::panes::PackageRow::Field(crate::tui::panes::DetailField::Path)
            )
        })
        .context("Path row")?;
    app.panes.package.viewport.set_pos(path_index);
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 80)?;
    let path = test_support::position(&buffer, "Path")?;
    let members = test_support::position(&buffer, "members")?;
    assert_ne!(
        buffer[path].bg, buffer[members].bg,
        "the other column must keep its background"
    );
    Ok(())
}

#[test]
fn package_page_mark_stays_clear_of_the_column_join() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(workspace_data());
    let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Package, 40, 8)?;
    let bottom = test_support::rows(&buffer).pop().context("bottom border")?;
    assert!(
        bottom.contains("of"),
        "the fixture must draw a page mark: {bottom}"
    );
    assert!(
        bottom.contains('┴'),
        "the column join must remain intact: {bottom}"
    );
    assert!(!bottom.contains("▼┴") && !bottom.contains("▲┴"), "{bottom}");
    Ok(())
}

#[test]
fn disk_parts_keep_their_rows_before_measurement() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let mut positions = Vec::new();
    for disk in [
        DiskMeasurement::NotMeasured,
        DiskMeasurement::TotalOnly(73),
        DiskMeasurement::Measured {
            total:  73,
            target: 0,
            other:  73,
        },
    ] {
        app.panes.package.set_content(PackageData {
            disk,
            ..PackageData::default()
        });
        let buffer = test_support::draw_pane(&mut app, PaneId::Package, 40)?;
        let target = test_support::position(&buffer, "target/")?;
        let other = test_support::position(&buffer, "Other")?;
        let rows = test_support::rows(&buffer);
        let value = if matches!(disk, DiskMeasurement::Measured { .. }) {
            "0 B"
        } else {
            "…"
        };
        assert!(rows[usize::from(target.1)].contains(value), "{rows:?}");
        positions.push((target.1, other.1));
    }
    assert!(positions.windows(2).all(|pair| pair[0] == pair[1]));
    Ok(())
}

#[test]
fn empty_description_keeps_its_inset_without_a_cut_word() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData::default());
    for width in [17, 15, 20, 8, 5] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Package, width)?;
        let mut text = String::new();
        for y in 1..buffer.area.bottom() - 1 {
            let row: String = (2..buffer.area.right() - 2)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert_eq!(buffer[(1, y)].symbol(), " ");
            assert!(!row.contains('…'));
            text.extend(row.chars().filter(|ch| !ch.is_whitespace()));
            if text == "Nodescription" {
                break;
            }
        }
        assert_eq!(text, "Nodescription", "width {width}");
        if (15..18).contains(&width) {
            assert_eq!(test_support::position(&buffer, "No")?, (2, 1));
            assert_eq!(test_support::position(&buffer, "description")?, (2, 2));
        }
    }
    Ok(())
}

#[test]
fn narrow_test_counts_cut_the_label_before_the_count() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        test_rows: vec![("integration", 1273)],
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 17)?;
    let (x, y) = test_support::position(&buffer, "1273")?;
    assert!(x < 16);
    let row = &test_support::rows(&buffer)[usize::from(y)];
    assert!(row.contains("integra…"), "{row}");
    assert_eq!(buffer[(buffer.area.right() - 2, y)].symbol(), " ");
    Ok(())
}

#[test]
fn workspace_ci_counts_stay_whole_in_two_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        stats_rows: vec![("members", 8)],
        ci_display: CiDisplay::Runs {
            ci_status:    Some(CiStatus::Failed),
            local:        9,
            github_total: GithubRunCount::Known(17),
        },
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Package, 47)?;
    let rows = test_support::rows(&buffer);
    let row = rows
        .iter()
        .find(|row| row.contains("CI "))
        .context("CI row")?;
    assert!(row.contains("local 9"), "{rows:?}");
    let local = test_support::position(&buffer, "local 9")?;
    let github = test_support::position(&buffer, "gh 17")?;
    assert_eq!(github, (local.0, local.1 + 1));
    assert!(!row.contains('…'), "{row}");
    Ok(())
}

#[test]
fn late_total_only_message_keeps_measured_disk_parts() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let path = crate::project::AbsolutePath::from(PROJECT_PATH);
    test_support::draw_ui(&mut app, 160)?;
    let sender = app.background.background_sender();
    sender.send(BackgroundMsg::DiskUsageBatch {
        root_path: path.clone(),
        entries:   vec![(
            path.clone(),
            crate::scan::DirSizes {
                total:                 4194,
                in_project_target:     4096,
                in_project_non_target: 98,
                max_source_mtime:      None,
            },
        )],
    })?;
    app.poll_background();
    app.ensure_detail_cached();
    let measured = test_support::draw_ui(&mut app, 160)?;
    sender.send(BackgroundMsg::DiskUsage { path, bytes: 73 })?;
    app.poll_background();
    app.ensure_detail_cached();
    let late = test_support::draw_ui(&mut app, 160)?;
    for buffer in [&measured, &late] {
        let area = app.panes.tiled_layout.area(PaneId::Package);
        for value in ["4.0 KiB", "98 B"] {
            test_support::find_text_cell(buffer, area, value)
                .with_context(|| format!("{value}: {:?}", test_support::rows(buffer)))?;
        }
    }
    Ok(())
}

#[test]
fn workspace_ci_uses_counts_when_the_metadata_column_is_short() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        stats_rows: vec![("members", 8), ("lib", 3), ("binary", 5)],
        ci_display: CiDisplay::Runs {
            ci_status:    Some(CiStatus::Failed),
            local:        9,
            github_total: GithubRunCount::Known(27),
        },
        ..PackageData::default()
    });
    for width in [40, 47, 80] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Package, width)?;
        let rows = test_support::rows(&buffer);
        let row = rows
            .iter()
            .find(|row| row.contains("CI "))
            .context("CI row")?;
        assert!(row.contains("local 9"), "{width}: {row}");
        let local = test_support::position(&buffer, "local 9")?;
        let github = test_support::position(&buffer, "gh 27")?;
        assert_eq!(github, (local.0, local.1 + 1));
        assert!(!row.contains('…'), "{width}: {row}");
    }
    Ok(())
}

#[test]
fn empty_description_wraps_inside_the_inset_at_each_window_width() -> anyhow::Result<()> {
    for width in [160, 80, 60] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let buffer = test_support::draw_ui(&mut app, width)?;
        let area = app.panes.tiled_layout.area(PaneId::Package);
        let x = area.x + 2;
        let y = area.y + 1;
        assert_eq!(buffer[(x - 1, y)].symbol(), " ");
        let row = |y| {
            (x..area.right() - 2)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        if width == 160 {
            assert!(row(y).starts_with("No description available"), "{}", row(y));
        } else if area.width >= 18 {
            assert_eq!(row(y).trim_end(), "No description");
        } else {
            assert_eq!(row(y).trim_end(), "No");
            assert_eq!(row(y + 1).trim_end(), "description");
            assert_eq!(buffer[(x - 1, y + 1)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn narrow_statistics_pages_repeat_headings_and_align_labeled_counts() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        title: "Workspace".to_string(),
        stats_rows: vec![("members", 8), ("lib", 3), ("binary", 5)],
        test_rows: vec![
            ("unit", 5908),
            ("integration", 1273),
            ("doc", 10),
            ("(ignored)", 5),
            ("", 7191),
        ],
        ..PackageData::default()
    });
    app.set_focus_to_pane(PaneId::Package);
    let count =
        panes::package_rows_from_data(app.panes.package.content().context("Package data")?).len();
    let mut seen = std::collections::HashSet::new();
    for cursor in 0..count {
        app.panes.package.viewport.set_pos(cursor);
        app.ensure_fit_widths_cached();
        app.ensure_disk_cache();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30))?;
        let frame = terminal.draw(|frame| render::ui(frame, &mut app))?;
        let area = app.panes.tiled_layout.area(PaneId::Package);
        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(
            0,
            0,
            area.width,
            area.height,
        ));
        for y in 0..area.height {
            for x in 0..area.width {
                buffer[(x, y)] = frame.buffer[(area.x + x, area.y + y)].clone();
            }
        }
        let edge = area.width - 2;
        let rows = test_support::rows(&buffer);
        for (label, value) in [
            ("unit", "5908"),
            ("inte", "1273"),
            ("doc", "10"),
            ("(ignored)", "5"),
            ("Total", "7191"),
        ] {
            if let Some(y) = rows.iter().position(|row| row.contains(label)) {
                let row = &rows[y];
                let y = u16::try_from(y)?;
                let (x, _) = test_support::find_text_cell(
                    &buffer,
                    ratatui::layout::Rect::new(1, y, area.width - 2, 1),
                    value,
                )
                .context("count beside label")?;
                assert_eq!(usize::from(x) + value.len(), usize::from(edge), "{row}");
                assert_eq!(buffer[(edge, y)].symbol(), " ");
                assert!(
                    rows[1..usize::from(y)]
                        .iter()
                        .any(|row| row.contains("Tests")),
                    "{rows:?}"
                );
                seen.insert(value);
            }
        }
        for value in ["lib", "binary"] {
            if let Ok((_, y)) = test_support::position(&buffer, value) {
                assert!(
                    rows[1..usize::from(y)]
                        .iter()
                        .any(|row| row.contains("Structure")),
                    "{rows:?}"
                );
            }
        }
    }
    assert_eq!(seen.len(), 5);
    Ok(())
}
