use anyhow::Context;
use tui_pane::ColorMode;

use super::constants::SOURCE_HEADER;
use super::render;
use crate::tui::columns;
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
            let source_header = header.split_whitespace().nth(1).context("Source header")?;
            assert_eq!(source_header, "Source", "{header:?}");
            let source_x = area.x
                + 1
                + u16::try_from(
                    header
                        .chars()
                        .position(|ch| ch == 'S')
                        .context("Source start")?,
                )?;
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
            let expected = if width == 95 {
                ["cargo-hand…", "cargo-berth"]
            } else {
                ["cargo-handler", "cargo-berth"]
            };
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
            for (pane_width, expected_name) in [
                (31, "cargo-handler"),
                (30, "cargo-handl…"),
                (27, "cargo-ha…"),
                (26, "cargo-h…"),
            ] {
                let mut data = test_support::target_data();
                data.binaries.truncate(1);
                data.binaries[0].display_name = "cargo-handler".into();
                data.binaries[0].source = source.clone();
                app.panes.targets.set_content(data);
                let buffer =
                    test_support::draw_pane_in_window(&mut app, PaneId::Targets, 95, pane_width)?;
                let (source_x, header_y) = test_support::position(&buffer, "Source")?;
                test_support::position(&buffer, "Kind")?;
                let y = header_y + 1;
                let name: String = (1..source_x).map(|x| buffer[(x, y)].symbol()).collect();
                let source_width = u16::try_from(columns::display_width(SOURCE_HEADER))?;
                let value: String = (source_x..source_x + source_width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                assert_eq!(name.trim(), expected_name, "{mode:?}, width={pane_width}");
                assert_eq!(
                    value.trim(),
                    expected_source,
                    "{mode:?}, width={pane_width}"
                );
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
            project_width,
            46,
            32,
            BottomRow::Diagnostics,
            14,
            preferred,
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
        for project_width in [66, 69] {
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
                ratatui::layout::Rect::new(0, 0, 160, 44),
                project_width,
                46,
                32,
                BottomRow::Diagnostics,
                14,
                preferred,
            );
            let area = layout.area(PaneId::Targets);
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::Targets, 160, area.width)?;
            let (_, header_y) = test_support::position(&buffer, "Kind")?;
            let header: String = (1..area.width - 1)
                .map(|x| buffer[(x, header_y)].symbol())
                .collect();
            let source_x = 1 + u16::try_from(
                header
                    .chars()
                    .position(|ch| ch == 'S')
                    .context("Source header")?,
            )?;
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
            assert!(
                area.width >= preferred,
                "{mode:?}: {area:?}, preferred={preferred}"
            );
            assert!(!layout.area(PaneId::Lang).is_empty());
            assert_eq!(
                layout.area(PaneId::Cpu).width,
                crate::tui::panes::constants::CPU_PANE_WIDTH
            );
        }
    }
    Ok(())
}
