use std::path::Path;

use anyhow::Context;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;

use crate::project::LangEntry;
use crate::project::LanguageStats;
use crate::project::Visibility;
use crate::tui::app::App;
use crate::tui::panes::PaneId;
use crate::tui::test_support;
use crate::tui::test_support::PROJECT_PATH;

#[test]
fn languages_keep_the_header_and_identify_parent_and_child_rows() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let entry = |language: &str, code| LangEntry {
        language: language.to_string(),
        files: 1,
        code,
        comments: 2,
        blanks: 1,
        children: Vec::new(),
    };
    let mut parent = entry("Rust", 10);
    parent.children.push(entry("Markdown", 6));
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .language_stats = Some(LanguageStats {
        entries: vec![parent],
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 50)?;
    test_support::position(&buffer, "Comments")?;
    test_support::position(&buffer, "Rust")?;
    test_support::position(&buffer, "└")?;
    test_support::position(&buffer, "Markdown")?;
    let row_text = test_support::rows(&buffer);
    let footer = row_text
        .iter()
        .rev()
        .find(|row| row.contains("13"))
        .context("parent-only total")?;
    assert!(
        !footer.contains("22"),
        "child totals must not be counted twice"
    );
    Ok(())
}

fn language_app() -> App {
    let mut app = test_support::project_app();
    let child = LangEntry {
        language: "code".to_string(),
        files:    1,
        code:     6,
        comments: 2,
        blanks:   1,
        children: Vec::new(),
    };
    let parent = LangEntry {
        language: "Rust".to_string(),
        children: vec![child],
        files:    1,
        code:     10,
        comments: 2,
        blanks:   1,
    };
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .language_stats = Some(LanguageStats {
        entries: vec![parent],
    });
    app
}

#[test]
fn language_headers_use_sentence_case() -> anyhow::Result<()> {
    let mut app = language_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    for header in ["Files", "Code", "Comments", "Blanks", "Total"] {
        test_support::position(&buffer, header)?;
    }
    Ok(())
}

#[test]
fn language_subtotals_name_the_parent_language() -> anyhow::Result<()> {
    let mut app = language_app();
    app.framework.color_mode = ColorMode::Colored;
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&buffer, "Rust")?;
    test_support::position(&buffer, "└ code")?;
    assert!(
        !test_support::rows(&buffer)
            .join("\n")
            .contains("Rust: code")
    );
    Ok(())
}

#[test]
fn language_title_counts_only_parents() -> anyhow::Result<()> {
    let mut app = language_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&buffer, "Languages (1)")?;
    Ok(())
}

#[test]
fn language_footer_names_its_total() -> anyhow::Result<()> {
    let mut app = language_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    let (_, y) = test_support::position(&buffer, "13")?;
    let footer = test_support::rows(&buffer)
        .into_iter()
        .skip(usize::from(y) + 1)
        .find(|row| row.contains("13"))
        .context("the total row")?;
    assert!(footer.contains("Total"), "{footer}");
    Ok(())
}

#[test]
fn language_position_counts_the_parent_for_children_and_later_parents() -> anyhow::Result<()> {
    let mut app = language_app();
    let info = app
        .project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("fixture project");
    info.language_stats
        .as_mut()
        .context("language stats")?
        .entries
        .push(LangEntry {
            language: "Python".to_string(),
            files:    1,
            code:     3,
            comments: 0,
            blanks:   0,
            children: Vec::new(),
        });
    app.panes.lang.focus.pane_focus_state = PaneFocusState::Active;
    test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    app.panes.lang.viewport.down();
    let child = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&child, "Languages (1 of 2)")?;
    app.panes.lang.viewport.down();
    let parent = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&parent, "Languages (2 of 2)")?;
    Ok(())
}

#[test]
fn narrow_language_subtotals_keep_the_category_name() -> anyhow::Result<()> {
    let mut app = language_app();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 49)?;
    test_support::position(&buffer, "└ code")?;
    Ok(())
}

#[test]
fn deleted_languages_do_not_present_cached_counts_as_current() -> anyhow::Result<()> {
    let mut app = language_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    let text = test_support::rows(&buffer).join("\n");
    assert!(text.to_lowercase().contains("deleted"), "{text}");
    assert!(
        !text.contains("Rust"),
        "cached language rows remain: {text}"
    );
    Ok(())
}

#[test]
fn deleted_languages_leave_the_body_empty() -> anyhow::Result<()> {
    let mut app = language_app();
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&buffer, "Languages — deleted")?;
    for y in 1..buffer.area.bottom() - 1 {
        for x in 1..buffer.area.right() - 1 {
            assert_eq!(buffer[(x, y)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn phase4e_language_names_keep_their_meaning_at_normal_width() -> anyhow::Result<()> {
    let mut app = language_app();
    app.framework.color_mode = ColorMode::Colored;
    let info = app
        .project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project");
    let entries = &mut info
        .language_stats
        .as_mut()
        .context("language stats")?
        .entries;
    entries[0].children[0].language = "integration".to_string();
    for language in ["Python", "Markdown", "Autoconf"] {
        entries.push(LangEntry {
            language: language.to_string(),
            files:    1,
            code:     3,
            comments: 0,
            blanks:   0,
            children: Vec::new(),
        });
    }
    let frame = test_support::draw_ui(&mut app, 160)?;
    let area = app.panes.tiled_layout.area(PaneId::Lang);
    test_support::find_text_cell(&frame, area, "└ integration")
        .context("whole child name at 160 columns")?;
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 60)?;
    let area = buffer.area;
    for name in ["└ integration", "Python", "Markdown", "Autoconf"] {
        test_support::find_text_cell(&buffer, area, name).with_context(|| {
            format!(
                "whole language name {name}: {:?}",
                test_support::rows(&buffer)
            )
        })?;
    }
    Ok(())
}

#[test]
fn phase4e_empty_deleted_languages_yield_focus_like_targets() -> anyhow::Result<()> {
    let mut app = language_app();
    app.framework.color_mode = ColorMode::Colored;
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project")
        .visibility = Visibility::Deleted;
    app.scan.bump_generation();
    app.ensure_detail_cached();
    app.set_focus_to_pane(PaneId::Lang);
    let buffer = test_support::draw_ui(&mut app, 160)?;
    test_support::position(&buffer, "Languages — deleted")?;
    let area = app.panes.tiled_layout.area(PaneId::Lang);
    let title =
        test_support::find_text_cell(&buffer, area, "Languages").context("language title")?;
    assert!(
        !buffer[title]
            .modifier
            .contains(ratatui::style::Modifier::BOLD),
        "an empty deleted pane must yield focus"
    );
    assert_ne!(app.focused_pane_id(), PaneId::Lang);
    Ok(())
}

#[test]
fn language_width_uses_real_names_and_numeric_values() -> anyhow::Result<()> {
    let mut app = language_app();
    app.framework.color_mode = ColorMode::Colored;
    let info = app
        .project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the project");
    let stats = info.language_stats.as_mut().context("language stats")?;
    stats.entries[0].files = 582;
    stats.entries[0].code = 163_601;
    stats.entries[0].comments = 4734;
    stats.entries[0].blanks = 11_798;
    stats.entries[0].children[0].language = "unit tests".to_string();
    let mut integration = stats.entries[0].children[0].clone();
    integration.language = "integration".to_string();
    stats.entries[0].children.push(integration);
    let frame = test_support::draw_ui(&mut app, 160)?;
    let area = app.panes.tiled_layout.area(PaneId::Lang);
    for label in ["└ unit tests", "└ integration", "163601"] {
        test_support::find_text_cell(&frame, area, label)
            .with_context(|| format!("{label}: {:?}", test_support::rows(&frame)))?;
    }
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 60)?;
    let area = buffer.area;
    for label in ["└ unit tests", "└ integration", "163601"] {
        test_support::find_text_cell(&buffer, area, label)
            .with_context(|| format!("{label}: {:?}", test_support::rows(&buffer)))?;
    }
    Ok(())
}

#[test]
fn long_language_names_preserve_cpu_and_targets_widths() -> anyhow::Result<()> {
    let mut app = test_support::sampled_cpu_app(32);
    app.framework.color_mode = ColorMode::Colored;
    app.project_list =
        crate::tui::project_list_state::ProjectList::new(vec![test_support::make_package(
            "cargo-liner-port-with-worktrees",
            Path::new(PROJECT_PATH),
        )]);
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("project")
        .language_stats = Some(LanguageStats {
        entries: vec![LangEntry {
            language: "Rust".to_string(),
            files:    1149,
            code:     510_420,
            comments: 41081,
            blanks:   40202,
            children: vec![LangEntry {
                language: "integration tests".to_string(),
                files:    69,
                code:     82775,
                comments: 2757,
                blanks:   5180,
                children: Vec::new(),
            }],
        }],
    });
    let mut targets = test_support::target_data();
    targets.binaries[0].name = "cargo-berth".to_string();
    targets.binaries[0].display_name = "cargo-berth".to_string();
    targets.binaries[1].name = "cargo-handler".to_string();
    targets.binaries[1].display_name = "cargo-handler".to_string();
    app.panes.targets.set_content(targets);
    let buffer = test_support::draw_ui(&mut app, 160)?;
    let cpu = app.panes.tiled_layout.area(PaneId::Cpu);
    let targets = app.panes.tiled_layout.area(PaneId::Targets);
    assert_eq!(cpu.width, crate::tui::panes::constants::CPU_PANE_WIDTH);
    for label in ["CPU cores", "32", "50%"] {
        test_support::find_text_cell(&buffer, cpu, label)
            .with_context(|| format!("{label}: {:?}", test_support::rows(&buffer)))?;
    }
    for name in ["cargo-berth", "cargo-handler"] {
        test_support::find_text_cell(&buffer, targets, name)
            .with_context(|| format!("{name}: {:?}", test_support::rows(&buffer)))?;
    }
    Ok(())
}

#[test]
fn language_totals_keep_a_blank_cell_before_the_border() -> anyhow::Result<()> {
    for (window, pane_width) in [(80, 50), (95, 60), (160, 80)] {
        let mut app = language_app();
        app.framework.color_mode = ColorMode::Colored;
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Lang, window, pane_width)?;
        let header = test_support::position(&buffer, "Total")?;
        let rust = test_support::position(&buffer, "Rust")?;
        for y in header.1..=rust.1 + 2 {
            assert_eq!(
                buffer[(pane_width - 2, y)].symbol(),
                " ",
                "{window}: {:?}",
                test_support::rows(&buffer)
            );
            assert_eq!(buffer[(pane_width - 1, y)].symbol(), "│");
        }
    }
    Ok(())
}

#[test]
fn ron_icon_keeps_all_three_letters_beside_the_language() -> anyhow::Result<()> {
    let mut app = language_app();
    app.framework.color_mode = ColorMode::Colored;
    app.project_list
        .at_path_mut(Path::new(PROJECT_PATH))
        .expect_found("the fixture project")
        .language_stats = Some(LanguageStats {
        entries: vec![LangEntry {
            language: "Rusty Object Notation".to_string(),
            files:    1,
            code:     10,
            comments: 0,
            blanks:   0,
            children: Vec::new(),
        }],
    });
    let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Lang, 80, 50)?;
    let icon = test_support::position(&buffer, " RON")?;
    assert_eq!(icon.0, 1);
    Ok(())
}
