use anyhow::Context;

use crate::sccache::SccacheSummary;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::PackageData;
use crate::tui::panes::PaneId;
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
