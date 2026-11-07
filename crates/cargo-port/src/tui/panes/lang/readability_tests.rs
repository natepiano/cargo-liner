use std::path::Path;

use anyhow::Context;
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
    let buffer = test_support::draw_pane(&mut app, PaneId::Lang, 80)?;
    test_support::position(&buffer, "Rust: code")?;
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
