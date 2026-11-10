use std::path::Path;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::Denominator;
use tui_pane::PhaseLifecycle;

use crate::config::LintIndicator;
use crate::http::RateLimitQuota;
use crate::project::AheadBehind;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::DiskMeasurement;
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
use crate::tui::panes::DescriptionState;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::GitProbe;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackagePresence;
use crate::tui::panes::PackageVersion;
use crate::tui::panes::PaneId;
use crate::tui::panes::QuotaObservation;
use crate::tui::panes::WorktreeInfo;
use crate::tui::project_list::ExpandKey;
use crate::tui::sccache;
use crate::tui::test_support;
use crate::tui::test_support::PROJECT;
use crate::tui::test_support::WIDTH;

fn complete_startup_rows(app: &mut App) -> anyhow::Result<()> {
    let completed_at = std::time::Instant::now()
        .checked_sub(crate::tui::constants::STARTUP_ROW_MIN_VISIBLE)
        .context("the startup visibility floor fits the test clock")?;
    for (lifecycle, first_seen) in [
        (
            &mut app.startup.disk.lifecycle,
            &mut app.startup.disk.first_seen,
        ),
        (
            &mut app.startup.git.lifecycle,
            &mut app.startup.git.first_seen,
        ),
        (
            &mut app.startup.repo.lifecycle,
            &mut app.startup.repo.first_seen,
        ),
        (
            &mut app.startup.crates_io.lifecycle,
            &mut app.startup.crates_io.first_seen,
        ),
        (
            &mut app.startup.metadata.lifecycle,
            &mut app.startup.metadata.first_seen,
        ),
        (
            &mut app.startup.lint_phase.lifecycle,
            &mut app.startup.lint_phase.first_seen,
        ),
        (
            &mut app.startup.languages.lifecycle,
            &mut app.startup.languages.first_seen,
        ),
        (
            &mut app.startup.tests.lifecycle,
            &mut app.startup.tests.first_seen,
        ),
    ] {
        *lifecycle = PhaseLifecycle::Completed { at: completed_at };
        *first_seen = Some(completed_at);
    }
    app.startup.details_declared.lifecycle = PhaseLifecycle::Completed { at: completed_at };
    Ok(())
}

#[test]
fn a_startup_percent_row_uses_the_space_its_hidden_detail_leaves() -> anyhow::Result<()> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        draw_short_ui(&mut app)?;
        app.initialize_startup_phase_tracker();
        app.startup.metadata.lifecycle = PhaseLifecycle::Completed {
            at: std::time::Instant::now(),
        };
        app.startup.repo.lifecycle = PhaseLifecycle::Running;
        app.startup.repo.expected = Denominator::Growing(std::collections::HashSet::new());
        let pane = Rect::new(46, 22, 34, 7);
        app.panes
            .tiled_layout
            .panes
            .iter_mut()
            .find(|pane| pane.pane == PaneId::CiRuns)
            .context("CI runs pane")?
            .area = pane;
        let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
        terminal.draw(|frame| {
            frame.render_widget(
                tui_pane::default_pane_chrome(mode).block("CI runs".to_owned(), false),
                pane,
            );
            super::render_toasts_over_panes(frame, &mut app, Rect::new(0, 0, 80, 29));
            mode.clear_colors(frame.buffer_mut());
        })?;
        let buffer = terminal.backend().buffer();
        let card = app
            .framework
            .toasts
            .hits()
            .first()
            .context("startup card")?
            .card_rect;
        let text: String = (card.left() + 1..card.right() - 1)
            .map(|x| buffer[(x, card.top() + 1)].symbol())
            .collect();
        assert!(text.contains("Cargo metadata"), "{mode:?}: {text}");
        assert!(text.ends_with("100% "), "{mode:?}: {text}");
        assert!(text.contains('▓'), "{mode:?}: {text}");
        assert!(!text.contains("▓▓▓▓▓▓▓▓"), "{mode:?}: {text}");
    }
    Ok(())
}

#[test]
fn closing_startup_rows_fit_each_resized_frame() -> anyhow::Result<()> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        app.framework
            .toasts
            .settings_mut()
            .animation
            .entrance_duration = tui_pane::ToastDuration::try_from_secs("entrance_duration", 0.0)?;
        app.initialize_startup_phase_tracker();
        complete_startup_rows(&mut app)?;
        draw_short_ui(&mut app)?;
        let id = app
            .framework
            .toasts
            .hits()
            .first()
            .context("startup card")?
            .id;
        app.tick_startup_panel();
        assert!(!app.startup.is_collecting());

        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
        terminal.draw(|frame| super::ui(frame, &mut app))?;
        let card = app
            .framework
            .toasts
            .hits()
            .iter()
            .find(|hit| hit.id == id)
            .context("closing startup card")?
            .card_rect;
        let buffer = terminal.backend().buffer();
        let row = |y| {
            (card.left()..card.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(
            row(card.y + 1).contains("Cargo metadata"),
            "{mode:?}: {}",
            row(card.y + 1)
        );
        assert!(row(card.bottom() - 1).contains("Closing in"));

        let buffer = draw_short_ui(&mut app)?;
        let card = app
            .framework
            .toasts
            .hits()
            .iter()
            .find(|hit| hit.id == id)
            .context("resized closing startup card")?
            .card_rect;
        for y in card.top() + 1..card.bottom() - 1 {
            let text: String = (card.left() + 1..card.right() - 1)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert!(text.contains("100%"), "{mode:?}: {text}");
            assert!(text.contains('▓'), "{mode:?}: {text}");
            assert!(!text.contains("▓▓▓▓▓▓▓▓"), "{mode:?}: {text}");
        }
        let view = app
            .framework
            .toasts
            .active_now()
            .into_iter()
            .find(|toast| toast.id() == id)
            .context("closing startup view")?;
        assert_eq!(view.body().lines().count(), 8);
    }
    Ok(())
}

#[test]
fn startup_rows_keep_their_bar_percent_and_page_count_at_eighty_columns() -> anyhow::Result<()> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        app.framework
            .toasts
            .settings_mut()
            .animation
            .entrance_duration = tui_pane::ToastDuration::try_from_secs("entrance_duration", 0.0)?;
        app.initialize_startup_phase_tracker();
        let completed_at = std::time::Instant::now();
        for lifecycle in [
            &mut app.startup.disk.lifecycle,
            &mut app.startup.git.lifecycle,
            &mut app.startup.repo.lifecycle,
            &mut app.startup.crates_io.lifecycle,
            &mut app.startup.metadata.lifecycle,
            &mut app.startup.lint_phase.lifecycle,
            &mut app.startup.languages.lifecycle,
            &mut app.startup.tests.lifecycle,
        ] {
            *lifecycle = PhaseLifecycle::Completed { at: completed_at };
        }
        let buffer = draw_short_ui(&mut app)?;
        let card = app
            .framework
            .toasts
            .hits()
            .first()
            .context("startup card")?
            .card_rect;
        let mut shown = 0;
        for y in card.top() + 1..card.bottom() - 1 {
            let text: String = (card.left() + 1..card.right() - 1)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert!(text.contains("100%"), "{mode:?}: {text}");
            assert!(text.contains('▓'), "{mode:?}: {text}");
            assert!(!text.contains("▓▓▓▓▓▓▓▓"), "{mode:?}: {text}");
            shown += 1usize;
        }
        let body = app.framework.toasts.active_views(std::time::Instant::now());
        let startup = body
            .iter()
            .find(|toast| toast.title() == "Startup")
            .context("startup view")?;
        let pages = startup.body().lines().count().div_ceil(shown);
        let bottom: String = (card.left()..card.right())
            .map(|x| buffer[(x, card.bottom() - 1)].symbol())
            .collect();
        assert!(
            bottom.contains(&format!("1 of {pages} ▼")),
            "{mode:?}: {bottom}"
        );
    }
    Ok(())
}

#[test]
fn startup_toast_stays_inside_the_corner_pane_at_eighty_columns() -> anyhow::Result<()> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        let before = draw_short_ui(&mut app)?;
        let corner = app.panes.tiled_layout.area(PaneId::CiRuns);
        let lints = app.panes.tiled_layout.area(PaneId::Lints);
        app.framework.toasts.push_timed(
            "Startup",
            "Cargo metadata\ncrates.io\nDisk usage\nGitHub repos\nLanguages\nLint history\nLocal git repos\nTest counts",
            std::time::Duration::from_secs(30),
            8,
        );
        let after = draw_short_ui(&mut app)?;
        let card = app
            .framework
            .toasts
            .hits()
            .first()
            .context("the startup toast draws")?
            .card_rect;
        assert!(
            card.left() > corner.left(),
            "{mode:?}: {card:?}, {corner:?}"
        );
        assert!(
            card.right() < corner.right(),
            "{mode:?}: {card:?}, {corner:?}"
        );
        assert!(card.top() > corner.top(), "{mode:?}: {card:?}, {corner:?}");
        assert!(
            card.bottom() < corner.bottom(),
            "{mode:?}: {card:?}, {corner:?}"
        );
        assert_eq!(after[(card.x, card.y)].symbol(), "┌");
        assert_eq!(after[(card.right() - 1, card.bottom() - 1)].symbol(), "┘");
        for area in [lints, corner] {
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if area == lints
                        || x == area.x
                        || x == area.right() - 1
                        || y == area.y
                        || y == area.bottom() - 1
                    {
                        assert_eq!(
                            after[(x, y)].symbol(),
                            before[(x, y)].symbol(),
                            "{mode:?}: the pane keeps cell {x},{y}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn a_toast_keeps_a_one_row_content_divider_outside_its_card() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.show_timed_toast("Startup", "Read projects");
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 45))?;
    terminal.draw(|frame| {
        frame.render_widget(
            ratatui::widgets::Paragraph::new("left │ right"),
            Rect::new(2, 2, 20, 1),
        );
        super::render_toasts_over_panes(frame, &mut app, frame.area());
    })?;
    let buffer = terminal.backend().buffer();
    assert!(test_support::find_text_cell(buffer, buffer.area, "left │ right").is_some());
    assert_ne!(app.framework.toasts.hits(), []);
    Ok(())
}

#[test]
fn startup_toast_leaves_whole_titles_and_connected_borders() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.show_timed_toast("Startup", "Scan projects\nRead Git data\nLoad lint results");
    let buffer = test_support::draw_ui(&mut app, WIDTH)?;
    let cards: Vec<_> = app
        .framework
        .toasts
        .hits()
        .iter()
        .map(|hit| hit.card_rect)
        .collect();
    assert!(!cards.is_empty(), "the startup toast must draw");
    let rows = test_support::rows(&buffer);
    for y in 1..buffer.area.bottom() - 2 {
        assert!(
            !rows[usize::from(y)].trim().is_empty(),
            "blank row {y}: {rows:?}"
        );
        for x in 1..buffer.area.right() - 1 {
            if buffer[(x, y)].symbol() != "│" {
                continue;
            }
            let connected = [y - 1, y + 1].into_iter().any(|neighbor| {
                matches!(
                    buffer[(x, neighbor)].symbol(),
                    "│" | "├"
                        | "┤"
                        | "┼"
                        | "┬"
                        | "┴"
                        | "┌"
                        | "┐"
                        | "└"
                        | "┘"
                        | "╭"
                        | "╮"
                        | "╰"
                        | "╯"
                )
            });
            assert!(connected, "an isolated border remains at {x},{y}: {rows:?}");
        }
    }
    let area = app.panes.tiled_layout.area(PaneId::Lints);
    let title_covered =
        (area.x..area.right()).all(|x| cards.iter().any(|card| card.contains((x, area.y).into())));
    assert!(
        title_covered || test_support::find_text_cell(&buffer, area, "No lint runs").is_some(),
        "the Lints title must remain whole: {rows:?}"
    );
    Ok(())
}

#[test]
fn short_git_description_keeps_five_rows_and_aligned_dividers() -> anyhow::Result<()> {
    let mut app = app_with_narrow_detail_fields();
    app.framework.color_mode = ColorMode::Colored;
    let description = "A collection of tools for Rust projects with builds, tests, worktrees, caches and continuous integration";
    let mut package = app
        .panes
        .package
        .content()
        .context("Package content")?
        .clone();
    package.description = DescriptionState::Known(description.to_string());
    app.panes.package.set_content(package);
    let mut git = app.panes.git.content().context("Git content")?.clone();
    let quota = QuotaObservation::Observed(RateLimitQuota {
        limit:     5000,
        used:      42,
        remaining: 4958,
        reset_at:  None,
    });
    git.rate_limit_core = quota;
    git.rate_limit_graphql = quota;
    git.description = DescriptionState::Known(description.to_string());
    app.panes.git.set_content(git);
    app.set_focus_to_pane(PaneId::Git);
    app.panes.git.viewport.home();
    let first = draw_short_ui(&mut app)?;
    let git_area = app.panes.tiled_layout.area(PaneId::Git);
    let package_area = app.panes.tiled_layout.area(PaneId::Package);
    let divider_y = (git_area.y + 1..git_area.bottom() - 1)
        .find(|y| matches!(first[(git_area.x, *y)].symbol(), "├" | "┝"))
        .context("Git divider")?;
    assert!(
        divider_y <= git_area.y + 3,
        "the description takes at most two rows: {:?}",
        test_support::rows(&first)
    );
    let last_description: String = (git_area.x + 1..git_area.right() - 1)
        .map(|x| first[(x, divider_y - 1)].symbol())
        .collect();
    assert!(
        last_description.trim_end().ends_with('…'),
        "{last_description}"
    );
    assert!(
        matches!(first[(package_area.x, divider_y)].symbol(), "├" | "┝"),
        "the two description dividers must align"
    );
    assert!(
        app.panes.git.viewport.content_area().height >= 5,
        "five data rows must remain"
    );
    assert_visible_field_is_whole(&first, git_area, "Status", "✨ clean")?;
    assert_visible_field_is_whole(&first, git_area, "Branch", "master")?;
    let first_rows = test_support::rows(&first);
    assert!(
        first_rows[usize::from(git_area.bottom() - 1)].contains("1 of"),
        "{first_rows:?}"
    );
    app.panes.git.viewport.end();
    let last = draw_short_ui(&mut app)?;
    let last_rows = test_support::rows(&last);
    assert!(
        last_rows[usize::from(git_area.bottom() - 1)].contains('▲'),
        "{last_rows:?}"
    );
    assert!(
        (git_area.x..git_area.right()).all(|x| last[(x, git_area.bottom() - 1)].symbol() != "▼"),
        "{last_rows:?}"
    );
    app.panes.git.viewport.home();
    let again = draw_short_ui(&mut app)?;
    assert_eq!(test_support::rows(&again), first_rows);
    let tall = test_support::draw_pane_at_size(&mut app, PaneId::Git, git_area.width, 30)?;
    let tall_divider = (1..tall.area.bottom() - 1)
        .find(|y| matches!(tall[(0, *y)].symbol(), "├" | "┝"))
        .context("tall Git divider")?;
    assert!(tall_divider > 3, "the tall pane keeps the full description");
    Ok(())
}

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
        description: DescriptionState::KnownEmpty,
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
        description: DescriptionState::KnownEmpty,
        title: "Workspace".to_string(),
        name: PROJECT.to_string(),
        path: "/tmp/workspace".to_string(),
        disk: DiskMeasurement::TotalOnly(73),
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
        description: DescriptionState::Known(
            "A collection of useful development tools".to_string(),
        ),
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
        description: DescriptionState::KnownEmpty,
        title: "Workspace".to_string(),
        path: "/tmp/workspace".to_string(),
        disk: DiskMeasurement::TotalOnly(73),
        ..PackageData::default()
    });
    app.panes.git.set_content(GitData {
        description: DescriptionState::Known("A fleet of useful tools".to_string()),
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
fn a_cpu_pane_too_short_for_bars_keeps_the_page_mark_in_its_border() -> anyhow::Result<()> {
    let mut app = test_support::sampled_cpu_app(64);
    let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Cpu, 24, 10)?;
    let (_, y) = test_support::position(&buffer, "▼")?;
    assert_eq!(y, buffer.area.bottom() - 1);
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
        description: DescriptionState::KnownEmpty,
        path: "/tmp/long-package-path".to_string(),
        disk: DiskMeasurement::Measured {
            total:  26_521_488_179,
            target: 25_769_803_776,
            other:  751_684_403,
        },
        types: Some(vec![crate::project::ProjectType::Binary]),
        version: PackageVersion::Manifest("0.1.0".to_string()),
        package_presence: PackagePresence::Present,
        ..PackageData::default()
    });
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
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
    let area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(2),
        ..area
    };
    let label_position = test_support::find_text_cell(buffer, area, label);
    let value_position = test_support::find_text_cell(buffer, area, value);
    if let (Some((_, label_y)), Some((_, value_y))) = (label_position, value_position)
        && label_y == value_y
    {
        return Ok(());
    }
    if let Some((_, y)) = test_support::find_text_cell(buffer, area, label) {
        let value_area = ratatui::layout::Rect::new(area.x, y + 1, area.width, 1);
        assert!(y + 1 < area.bottom(), "{label} has no value row");
        test_support::find_text_cell(buffer, value_area, value).context("value below its label")?;
    }
    if let Some((_, y)) = test_support::find_text_cell(buffer, area, value) {
        assert!(y > area.y, "{value} has no label row");
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
        if let Some((_, y)) = test_support::find_text_cell(&buffer, area, "Sync") {
            let header = ratatui::layout::Rect::new(area.x, y, area.width, 1);
            test_support::find_text_cell(&buffer, header, "Branch")
                .context("whole Branch header")?;
            let text: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert!(!text.contains('…'), "{text}");
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
    data.description = DescriptionState::Known("a fleet of cargo tools for projects, tests, builds, caches, worktrees and continuous integration".to_string());
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
    data.rate_limit_core = QuotaObservation::Observed(quota);
    data.rate_limit_graphql = QuotaObservation::Observed(quota);
    let fields = panes::git_fields_from_data(&data);
    app.panes.git.set_content(data);
    app.set_focus_to_pane(PaneId::Git);
    for (index, field) in fields.iter().enumerate() {
        app.panes.git.viewport.set_pos(index + 1);
        let buffer = draw_short_ui(&mut app)?;
        let area = app.panes.git.viewport.content_area();
        assert!(
            area.height >= 5,
            "the short description leaves five Git rows"
        );
        let (x, y) =
            test_support::find_text_cell(&buffer, area, field.label()).with_context(|| {
                format!(
                    "whole label {}: {:?}",
                    field.label(),
                    test_support::rows(&buffer)
                )
            })?;
        let value_x = x + u16::try_from(field.label().len())?;
        let inline_value = (value_x..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        let value = if inline_value.trim().is_empty() {
            assert!(
                y + 1 < area.bottom(),
                "the value must stay on the label page"
            );
            (area.x..area.right())
                .map(|x| buffer[(x, y + 1)].symbol())
                .collect::<String>()
        } else {
            inline_value
        };
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
        package.disk = DiskMeasurement::TotalOnly(1 << 30);
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
        description: DescriptionState::KnownEmpty,
        title: "Project".to_string(),
        name: "proof_crate".to_string(),
        path: root.display().to_string(),
        disk: DiskMeasurement::TotalOnly(26_521_488_179),
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
    let quota = QuotaObservation::Observed(RateLimitQuota {
        limit:     5000,
        used:      42,
        remaining: 4958,
        reset_at:  None,
    });
    let mut git = app.panes.git.content().context("Git content")?.clone();
    git.rate_limit_core = quota;
    git.rate_limit_graphql = quota;
    app.panes.git.set_content(git);
    app.set_focus_to_pane(PaneId::Git);
    app.panes.git.viewport.home();
    let first = draw_short_ui(&mut app)?;
    let area = app.panes.tiled_layout.area(PaneId::Git);
    let expected_width = if app.config.lint_enabled() == LintIndicator::Enabled {
        16
    } else {
        17
    };
    assert_eq!(
        area.width, expected_width,
        "the full UI keeps whole Git divider titles"
    );
    assert_pane_page(&first, area, "1 of 3 ▼");
    let first_worktree =
        panes::git_fields_from_data(app.panes.git.content().context("Git content")?).len();
    app.panes.git.viewport.set_pos(first_worktree - 1);
    let second = draw_short_ui(&mut app)?;
    let title = test_support::find_text_cell(&second, area, "Worktrees")
        .with_context(|| format!("worktree divider: {:?}", test_support::rows(&second)))?;
    assert_eq!(second[(area.x, title.1)].symbol(), "├");
    assert_eq!(second[(area.right() - 1, title.1)].symbol(), "┤");
    assert_pane_page(&second, area, "▲ 2 of 3 ▼");
    test_support::find_text_cell(&second, area, "master").context("worktree table on page 2")?;
    app.panes.git.viewport.home();
    let first_again = draw_short_ui(&mut app)?;
    assert_pane_page(&first_again, area, "1 of 3 ▼");
    let mut data = app.panes.git.content().context("Git content")?.clone();
    let sample = data.worktrees[0].clone();
    data.worktrees = (0..13)
        .map(|index| WorktreeInfo {
            name: format!("checkout-{index}"),
            branch: Some(format!("b{index:02}")),
            ..sample.clone()
        })
        .collect();
    data.description =
        DescriptionState::Known("Tools for projects, builds, tests and worktrees".to_string());
    app.panes.git.set_content(data);
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        let mut seen = std::collections::HashSet::new();
        let count =
            panes::git_fields_from_data(app.panes.git.content().context("Git content")?).len();
        for cursor in count..count + 14 {
            app.panes.git.viewport.set_pos(cursor);
            let buffer = draw_short_ui(&mut app)?;
            let area = app.panes.tiled_layout.area(PaneId::Git);
            assert!(app.panes.git.viewport.visible_rows() > 1);
            if let Some((_, y)) = test_support::find_text_cell(&buffer, area, "Worktrees (13)") {
                let (left, right) = match mode {
                    ColorMode::Colored => ("├", "┤"),
                    ColorMode::NoColor => ("┣", "┫"),
                };
                assert_eq!(buffer[(area.x, y)].symbol(), left);
                assert_eq!(buffer[(area.right() - 1, y)].symbol(), right);
            }
            for index in 0..13 {
                if test_support::find_text_cell(&buffer, area, &format!("b{index:02}")).is_some() {
                    seen.insert(index);
                    test_support::find_text_cell(&buffer, area, "Worktrees (13)")
                        .context("whole divider on the worktree page")?;
                }
            }
        }
        assert_eq!(seen.len(), 13);
    }
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
