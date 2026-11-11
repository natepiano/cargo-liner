use anyhow::Context;
use tui_pane::ColorMode;

use crate::project::Visibility;
use crate::tui::panes::PaneId;
use crate::tui::panes::RunTargetKind;
use crate::tui::panes::TargetEntry;
use crate::tui::test_support;

#[test]
fn narrow_targets_keep_target_and_kind_before_source() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    let buffer = test_support::draw_pane(&mut app, PaneId::Targets, 23)?;
    test_support::position(&buffer, "Target")?;
    test_support::position(&buffer, "Kind")?;
    test_support::position(&buffer, "first")?;
    assert!(!test_support::rows(&buffer).join("\n").contains("Source"));
    Ok(())
}

#[test]
fn targets_keep_the_count_at_eighty_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.tiled_layout.area(PaneId::Targets);
    assert!(!area.is_empty());
    let title: String = (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect();
    assert!(title.contains("(2)"), "{title}");
    Ok(())
}

#[test]
fn deleted_targets_name_the_state_once_and_leave_the_body_empty() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    app.project_list
        .at_path_mut(std::path::Path::new(test_support::PROJECT_PATH))
        .expect_found("the project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Targets, 80)?;
    test_support::position(&buffer, "Targets — deleted")?;
    assert_eq!(
        test_support::rows(&buffer)
            .join("\n")
            .matches("deleted")
            .count(),
        1
    );
    for y in 1..buffer.area.bottom() - 1 {
        for x in 1..buffer.area.right() - 1 {
            assert_eq!(buffer[(x, y)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn target_names_take_width_before_the_source_at_eighty_columns() -> anyhow::Result<()> {
    for name in [
        "proof_crate",
        "proof_crate_with_a_name_longer_than_the_pane",
    ] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let mut data = test_support::target_data();
        data.binaries.truncate(1);
        data.binaries[0].display_name = name.to_string();
        data.binaries[0].source =
            crate::tui::panes::TargetSource::workspace_root("proof_crate".to_string());
        app.panes.targets.set_content(data);
        let buffer = test_support::draw_ui(&mut app, 80)?;
        let area = app.panes.tiled_layout.area(PaneId::Targets);

        let (row_y, row) = (area.y + 2..area.bottom() - 1)
            .find_map(|y| {
                let text: String = (area.x + 1..area.right() - 1)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                text.contains("binary").then_some((y, text))
            })
            .context("Target row")?;
        let source = test_support::find_text_cell(&buffer, area, "Source");
        assert_eq!(source.is_some(), name == "proof_crate");
        let data_row = ratatui::layout::Rect::new(area.x, row_y, area.width, 1);
        let binary =
            test_support::find_text_cell(&buffer, data_row, "binary").context("Binary value")?;
        let source_x = source.map_or(binary.0, |position| position.0);
        let target: String = row
            .chars()
            .take(usize::from(source_x - area.x - 1))
            .collect();
        if name == "proof_crate" {
            assert!(target.contains(name), "{area:?} {row}");
        } else {
            assert!(target.trim_end().ends_with('…'), "{row}");
            assert!(target.contains("proof"), "{row}");
        }
    }
    Ok(())
}

#[test]
fn binary_kind_returns_a_cell_to_source_at_eighty_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let mut data = test_support::target_data();
    for (entry, source) in data
        .binaries
        .iter_mut()
        .zip(["cargo-berth", "cargo-handler"])
    {
        entry.display_name = "proof_crate".to_string();
        entry.source = crate::tui::panes::TargetSource::member(source.to_string());
    }
    app.panes.targets.set_content(data.clone());
    let buffer = test_support::draw_ui(&mut app, 80)?;
    let area = app.panes.tiled_layout.area(PaneId::Targets);
    for (whole, shortened) in [
        ("cargo-berth", "cargo-b…"),
        ("cargo-handler", "cargo-handl…"),
    ] {
        assert!(
            test_support::find_text_cell(&buffer, area, whole).is_some()
                || test_support::find_text_cell(&buffer, area, shortened).is_some(),
            "distinct source {whole}: {:?}",
            test_support::rows(&buffer)
        );
    }
    let compact = test_support::draw_pane(&mut app, PaneId::Targets, 31)?;
    for source in ["cargo-b…", "cargo-h…"] {
        test_support::position(&compact, source)?;
    }
    let wide = test_support::draw_pane(&mut app, PaneId::Targets, 160)?;
    for source in ["cargo-berth", "cargo-handler"] {
        test_support::position(&wide, source)?;
    }
    let narrow = test_support::draw_ui(&mut app, 60)?;
    let area = app.panes.tiled_layout.area(PaneId::Targets);
    assert!(area.is_empty(), "{:?}", test_support::rows(&narrow));
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));
    let hidden = test_support::draw_pane(&mut app, PaneId::Targets, 23)?;
    assert!(!test_support::rows(&hidden).join("\n").contains("Source"));
    data.examples.push(TargetEntry {
        run_target_kind: RunTargetKind::Example,
        display_name: "demo".to_string(),
        ..data.binaries[0].clone()
    });
    app.panes.targets.set_content(data);
    let mixed = test_support::draw_ui(&mut app, 80)?;
    test_support::position(&mixed, "example")?;
    Ok(())
}

#[test]
fn workspace_sources_keep_their_column_at_ninety_five_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let mut data = test_support::target_data();
    for (entry, source) in data.binaries.iter_mut().zip(["workspace_a", "workspace_b"]) {
        entry.display_name = "proof_crate_with_a_long_target_name".to_string();
        entry.source = crate::tui::panes::TargetSource::workspace_root(source.to_string());
    }
    app.panes.targets.set_content(data);
    let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Targets, 95, 29)?;
    assert!(test_support::find_text_cell(&buffer, buffer.area, "Source").is_none());
    let (_, header_y) = test_support::position(&buffer, "Kind")?;
    let data_row = ratatui::layout::Rect::new(0, header_y + 1, 29, 1);
    let binary =
        test_support::find_text_cell(&buffer, data_row, "binary").context("Binary value")?;
    for y in binary.1..=binary.1 + 1 {
        let name: String = (1..binary.0).map(|x| buffer[(x, y)].symbol()).collect();
        assert!(name.trim_end().ends_with('…'), "{name}");
    }
    Ok(())
}
