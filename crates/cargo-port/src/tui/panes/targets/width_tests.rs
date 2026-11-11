use anyhow::Context;
use ratatui::layout::Rect;
use tui_pane::ColorMode;

use super::constants::SOURCE_HEADER;
use super::render;
use crate::tui::columns;
use crate::tui::columns::LintColumnStatus;
use crate::tui::columns::ProjectColumnLayout;
use crate::tui::panes;
use crate::tui::panes::BottomRow;
use crate::tui::panes::PaneId;
use crate::tui::panes::RunTargetKind;
use crate::tui::panes::TargetEntry;
use crate::tui::panes::TargetSource;
use crate::tui::test_support;

#[test]
fn targets_keep_the_source_header_before_names_at_ninety_five_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for width in [95, 160] {
            app.framework.color_mode = mode;
            let mut data = test_support::target_data();
            for (entry, name) in data
                .binaries
                .iter_mut()
                .zip(["cargo-handler", "cargo-berth"])
            {
                entry.display_name = name.to_string();
                entry.source = TargetSource::member(name.to_string());
            }
            app.panes.targets.set_content(data);
            let buffer = test_support::draw_ui(&mut app, width)?;
            let area = app.panes.tiled_layout.area(PaneId::Targets);
            for text in ["Target", "Kind", "binary"] {
                test_support::find_text_cell(&buffer, area, text).with_context(|| {
                    format!(
                        "{width}, {mode:?}: missing {text}: {:?}",
                        test_support::rows(&buffer)
                    )
                })?;
            }
            let (_, header_y) =
                test_support::find_text_cell(&buffer, area, "Kind").context("Kind header")?;
            let header: String = (area.x + 1..area.right() - 1)
                .map(|x| buffer[(x, header_y)].symbol())
                .collect();
            let source = test_support::find_text_cell(&buffer, area, "Source");
            assert_eq!(source.is_some(), width == 160, "{header:?}");
            let data_row = Rect::new(area.x, header_y + 1, area.width, 1);
            let binary = test_support::find_text_cell(&buffer, data_row, "binary")
                .context("Binary value")?;
            let source_x = source.map_or(binary.0, |position| position.0);
            let names: Vec<_> = (header_y + 1..area.bottom() - 1)
                .take(2)
                .map(|y| {
                    (area.x + 1..source_x)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .trim()
                        .to_string()
                })
                .collect();
            let expected = ["cargo-handler", "cargo-berth"];
            assert_eq!(names, expected, "{width}, {mode:?}");
        }
    }
    Ok(())
}

#[test]
fn every_source_kind_keeps_six_source_cells_before_target_names_are_cut() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for (source, expected_source) in [
            (
                TargetSource::workspace_root("workspace_root".into()),
                "works…",
            ),
            (TargetSource::member("workspace_member".into()), "works…"),
            (TargetSource::worktree("worktree_checkout".into()), "workt…"),
        ] {
            for pane_width in [31, 30, 27, 26] {
                let mut data = test_support::target_data();
                data.binaries.truncate(1);
                data.binaries[0].display_name = "cargo-handler".into();
                data.binaries[0].source = source.clone();
                app.panes.targets.set_content(data);
                let buffer =
                    test_support::draw_pane_in_window(&mut app, PaneId::Targets, 95, pane_width)?;
                let (_, header_y) = test_support::position(&buffer, "Kind")?;
                let source_position = test_support::find_text_cell(&buffer, buffer.area, "Source");
                assert_eq!(source_position.is_some(), pane_width == 31);
                let data_row = Rect::new(0, header_y + 1, pane_width, 1);
                let binary_x = test_support::find_text_cell(&buffer, data_row, "binary")
                    .context("Binary value")?
                    .0;
                let name_end = source_position.map_or(binary_x, |position| position.0);
                let y = header_y + 1;
                let name: String = (1..name_end).map(|x| buffer[(x, y)].symbol()).collect();
                assert_eq!(name.trim(), "cargo-handler", "{mode:?}, width={pane_width}");
                if let Some((source_x, _)) = source_position {
                    let source_width = u16::try_from(columns::display_width(SOURCE_HEADER))?;
                    let value: String = (source_x..source_x + source_width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    assert_eq!(
                        value.trim(),
                        expected_source,
                        "{mode:?}, width={pane_width}"
                    );
                }
                test_support::position(&buffer, "binary")?;
            }
        }
    }
    Ok(())
}

#[test]
fn preferred_width_measures_all_target_kinds_without_heap_allocations() {
    let mut data = test_support::target_data();
    data.examples.push(TargetEntry {
        display_name: "example_with_a_long_name".to_string(),
        run_target_kind: RunTargetKind::Example,
        ..data.binaries[0].clone()
    });
    data.benches.push(TargetEntry {
        display_name: "bench_with_an_even_longer_name".to_string(),
        run_target_kind: RunTargetKind::Bench,
        ..data.binaries[0].clone()
    });
    let allocations = crate::test_support::count_allocations(|| {
        std::hint::black_box(render::preferred_width(&data))
    })
    .1;
    assert_eq!(allocations, 0);
}

#[test]
fn empty_targets_keep_the_project_column_at_its_measured_width() {
    let data = panes::TargetsData::default();
    let preferred = render::preferred_width(&data);
    assert_eq!(preferred, super::minimum_width());
    for project_width in [66, 69, 80] {
        let layout = panes::resolve_layout(
            ratatui::layout::Rect::new(0, 0, 160, 44),
            ProjectColumnLayout {
                preferred_width: project_width,
                minimum_width:   46,
                shown_width:     46,
                lint_column:     LintColumnStatus::Hidden,
            },
            32,
            BottomRow::Diagnostics,
            14,
            panes::MiddlePaneNeeds::minimum(preferred),
        );
        assert_eq!(layout.area(PaneId::ProjectList).width, project_width);
    }
}

#[test]
fn wide_layout_reserves_target_names_before_project_names_grow() -> anyhow::Result<()> {
    let names = [
        "cargo-berth",
        "cargo-handler",
        "cargo-mend",
        "cargo-port",
        "cargo-tile",
    ];
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for (window_width, project_width, shown_width) in [
            (160, 66, 46),
            (160, 69, 46),
            (160, 80, 64),
            (163, 80, 64),
            (200, 80, 64),
        ] {
            let mut app = test_support::sampled_cpu_app(32);
            app.framework.color_mode = mode;
            let mut data = test_support::target_data();
            let entry = data.binaries[0].clone();
            data.binaries = names
                .iter()
                .map(|name| TargetEntry {
                    name: (*name).into(),
                    display_name: (*name).into(),
                    source: TargetSource::member((*name).into()),
                    ..entry.clone()
                })
                .collect();
            app.panes.targets.set_content(data);
            let preferred = app.panes.targets.preferred_width();
            let layout = panes::resolve_layout(
                ratatui::layout::Rect::new(0, 0, window_width, 44),
                ProjectColumnLayout {
                    preferred_width: project_width,
                    minimum_width: 46,
                    shown_width,
                    lint_column: app.config.current().lint.enabled.into(),
                },
                32,
                BottomRow::Diagnostics,
                14,
                panes::MiddlePaneNeeds::minimum(preferred),
            );
            let area = layout.area(PaneId::Targets);
            let buffer = test_support::draw_pane_in_window(
                &mut app,
                PaneId::Targets,
                window_width,
                area.width,
            )?;
            let (_, header_y) = test_support::position(&buffer, "Kind")?;
            let header: String = (1..area.width - 1)
                .map(|x| buffer[(x, header_y)].symbol())
                .collect();
            let source = test_support::find_text_cell(&buffer, buffer.area, "Source");
            let capture = window_width == 160 && shown_width == 64;
            assert_eq!(source.is_none(), capture, "{header}");
            let data_row = Rect::new(0, header_y + 1, area.width, 1);
            let binary = test_support::find_text_cell(&buffer, data_row, "binary")
                .context("Binary value")?;
            let source_x = source.map_or(binary.0, |position| position.0);
            let drawn: Vec<_> = (header_y + 1..buffer.area.bottom() - 1)
                .take(names.len())
                .map(|y| {
                    (1..source_x)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .trim()
                        .to_owned()
                })
                .collect();
            assert_eq!(drawn, names, "{mode:?}, Targets={area:?}");
            if capture {
                let row: String = (1..area.width - 1)
                    .map(|x| buffer[(x, header_y + 2)].symbol())
                    .collect();
                assert_eq!(row, " cargo-handler     binary ");
            } else if window_width != 163 {
                assert!(
                    area.width >= preferred,
                    "{mode:?}: {area:?}, preferred={preferred}"
                );
            }
            assert!(!test_support::rows(&buffer).join("\n").contains("cargo…"));
            assert!(layout.area(PaneId::ProjectList).width >= shown_width);
            assert!(layout.area(PaneId::Lang).width >= 49);
            assert!(!layout.area(PaneId::Lang).is_empty());
            assert_eq!(
                layout.area(PaneId::Cpu).width,
                crate::tui::panes::constants::CPU_PANE_WIDTH
            );
        }
    }
    Ok(())
}
