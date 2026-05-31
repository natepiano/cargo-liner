//! Tests for the framework-keymap path.
//!
//! - Bar snapshots assert that `tui_pane::render_status_bar` produces the expected pane-action
//!   labels when Package or Git is focused. They read `bar.pane_action` only — the global and nav
//!   regions are covered separately by the `AppGlobalAction` snapshots below.
//! - The `state` tests pin the `Shortcuts::state` rules that gray out `Activate` when the cursor
//!   sits on a row whose dispatch has no effect (Package's non-`CratesIo` rows; Git's flat fields
//!   and any remote without a URL).

use std::fs;
use std::path::Path;
use std::rc::Rc;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Span;
use toml::Table;
use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::BarPalette;
use tui_pane::FocusedPane;
use tui_pane::FrameworkFocusId;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::KeyBind;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;
use tui_pane::Visibility;
use tui_pane::render_status_bar;

use super::App;
use super::make_app;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::ci::FetchStatus;
use crate::config::CargoPortConfig;
use crate::config::NavigationKeys;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::project::HeadState;
use crate::project::ProjectType;
use crate::project::RootItem;
use crate::project::Submodule;
use crate::test_support;
use crate::tui::app::CargoPortToastAction;
use crate::tui::input;
use crate::tui::integration::AppGlobalAction;
use crate::tui::integration::AppPaneId;
use crate::tui::integration::CiRunsPane;
use crate::tui::integration::FinderPane;
use crate::tui::integration::GitPane;
use crate::tui::integration::NavAction;
use crate::tui::integration::PackagePane;
use crate::tui::keymap;
use crate::tui::keymap::CiRunsAction;
use crate::tui::keymap::GitAction;
use crate::tui::keymap::OutputAction;
use crate::tui::keymap::PackageAction;
use crate::tui::keymap_ui;
use crate::tui::panes;
use crate::tui::panes::CiData;
use crate::tui::panes::CiEmptyState;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitData;
use crate::tui::panes::LintsData;
use crate::tui::panes::PackageData;
use crate::tui::panes::PaneId;
use crate::tui::panes::PublishStatus;
use crate::tui::panes::RemoteRow;
use crate::tui::panes::TargetsData;
use crate::tui::render;
use crate::tui::settings::SettingOption;
use crate::tui::state::ServiceStatus;

const TAB_WALK_STEPS: usize = 6;
const SINGLE_RUN_COUNT: usize = 1;
const GLOBAL_SHORTCUTS_TEST_WIDTH: u16 = 100;
const GLOBAL_SHORTCUTS_TEST_HEIGHT: u16 = 40;

fn focus_app_pane_in_framework(app: &mut App, id: AppPaneId) {
    app.set_focus(FocusedPane::App(id));
}

fn flatten(spans: &[Span<'static>]) -> String {
    let mut out = String::new();
    for span in spans {
        out.push_str(&span.content);
    }
    out
}

fn assert_contains_in_order(text: &str, labels: &[&str]) {
    let mut start = 0;
    for label in labels {
        let Some(offset) = text[start..].find(label) else {
            panic!("{label:?} missing or out of order in {text:?}");
        };
        start += offset + label.len();
    }
}

fn make_app_with_keymap_toml(projects: &[RootItem], toml: &str) -> App {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(&toml_path, toml).expect("write keymap toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
    make_app(projects)
}

fn make_app_with_config_and_keymap_toml(
    projects: &[RootItem],
    cfg: &CargoPortConfig,
    toml: &str,
) -> App {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(&toml_path, toml).expect("write keymap toml");
    let keymap_path_guard = keymap::override_keymap_path_for_test(toml_path);
    let app = super::make_app_with_config(projects, cfg);
    drop(keymap_path_guard);
    app
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let event = Event::Key(KeyEvent::new(code, modifiers));
    input::handle_event(app, &event);
}

fn open_framework_overlay(app: &mut App, action: FrameworkGlobalAction) {
    let keymap = Rc::clone(&app.framework_keymap);
    keymap.dispatch_framework_global(action, app);
}

fn buffer_text_sized(app: &mut App, width: u16, height: u16) -> String {
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
    terminal
        .draw(|frame| render::ui(frame, app))
        .unwrap_or_else(|_| std::process::abort());
    let area = terminal.size().unwrap_or_else(|_| std::process::abort());
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

fn make_app_with_git_tabbable() -> App {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.git.set_content(GitData {
        head: Some(HeadState::Branch("main".to_string())),
        ..GitData::default()
    });
    app
}

fn package_data_no_version() -> PackageData {
    PackageData {
        package_title:            "Package".to_string(),
        title_name:               "demo".to_string(),
        worktree_group_summary:   None,
        primary_section:          None,
        path:                     "~/demo".to_string(),
        version:                  Some("0.1.0".to_string()),
        description:              None,
        crates_version:           None,
        crates_downloads:         None,
        publish_status:           PublishStatus::NotPublishable,
        crates_io_service:        ServiceStatus::Available,
        types:                    Some(vec![ProjectType::Library]),
        disk:                     Some(1_048_576),
        stats_rows:               Vec::new(),
        test_rows:                Vec::new(),
        has_package:              true,
        edition:                  None,
        license:                  None,
        homepage:                 None,
        repository:               None,
        in_project_target:        None,
        in_project_non_target:    None,
        out_of_tree_target_bytes: None,
        lint_display:             crate::tui::panes::LintDisplay::default(),
        ci_display:               crate::tui::panes::CiDisplay::default(),
    }
}

#[test]
fn focused_app_panes_render_expected_pane_action_labels() {
    type Setup = fn(&mut App);
    let cases: &[(AppPaneId, &[&str], Setup)] = &[
        (AppPaneId::Package, &["activate"], |app| {
            app.panes.package.set_content(package_data_no_version());
        }),
        (AppPaneId::Git, &["activate"], |app| {
            app.panes.git.set_content(GitData::default());
        }),
        (AppPaneId::Targets, &["run", "release", "kill"], |_| {}),
        (AppPaneId::Lints, &["open", "del history"], |_| {}),
        (
            AppPaneId::CiRuns,
            &["open", "fetch more", "all", "del cache"],
            |app| {
                app.ci.set_content(ci_data_with_runs(2));
                app.ci.viewport.set_pos(0);
            },
        ),
        (AppPaneId::Finder, &["go to", "close"], |_| {}),
    ];

    for (pane, expected_labels, setup) in cases {
        let project = super::make_project(Some("demo"), "~/demo");
        let mut app = make_app(&[project]);
        setup(&mut app);
        focus_app_pane_in_framework(&mut app, *pane);

        let palette = BarPalette::default();
        let bar = render_status_bar(
            &FocusedPane::App(*pane),
            &app,
            &app.framework_keymap,
            app.framework(),
            &palette,
        );
        let pane_action = flatten(&bar.pane_action);

        for label in *expected_labels {
            assert!(
                pane_action.contains(label),
                "{pane:?} bar must show label {label:?} (got {pane_action:?})",
            );
        }
    }
}

#[test]
fn package_activate_state_disabled_when_no_crates_version() {
    // `package_fields_from_data` omits the CratesIo row when
    // `crates_version` is `None`, so no cursor position lands on a
    // row whose Activate dispatch does anything — the state must be
    // Disabled regardless of where the cursor sits.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    let data = package_data_no_version();
    app.panes.package.set_content(data);
    app.panes.package.viewport.set_pos(0);

    let pane = PackagePane;
    assert_eq!(
        pane.state(PackageAction::Activate, &app),
        ShortcutState::Disabled,
        "Activate must be Disabled when crates_version is None — no actionable row exists",
    );
}

#[test]
fn package_activate_state_enabled_on_crates_io_with_version() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    let mut data = package_data_no_version();
    data.crates_version = Some("0.1.0".to_string());
    let rows = panes::package_rows_from_data(&data);
    let crates_io_pos = rows
        .iter()
        .position(|row| matches!(row, panes::PackageRow::Field(DetailField::CratesIo)))
        .expect("crates.io row must appear for a Rust package with title_name set");
    app.panes.package.set_content(data);
    app.panes.package.viewport.set_pos(crates_io_pos);

    let pane = PackagePane;
    assert_eq!(
        pane.state(PackageAction::Activate, &app),
        ShortcutState::Enabled,
        "Activate is Enabled on CratesIo when crates_version is known",
    );
}

fn git_remote_with_url(url: &str) -> RemoteRow {
    RemoteRow {
        name:            "origin".to_string(),
        icon:            "",
        display_url:     url.to_string(),
        tracked_ref:     String::new(),
        status:          String::new(),
        full_url:        Some(url.to_string()),
        push_annotation: None,
    }
}

#[test]
fn git_activate_state_disabled_when_cursor_not_on_remote() {
    // Default GitData has only the two rate-limit flat fields and no
    // remotes — the cursor at position 0 lands on a flat field whose
    // Activate dispatch is a no-op, so the state must be Disabled.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.git.set_content(GitData::default());
    app.panes.git.viewport.set_pos(0);

    let pane = GitPane;
    assert_eq!(
        pane.state(GitAction::Activate, &app),
        ShortcutState::Disabled,
        "Activate must be Disabled on a flat field row — only Remote rows dispatch",
    );
}

#[test]
fn git_activate_state_enabled_on_remote_with_url() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    let mut data = GitData::default();
    data.remotes
        .push(git_remote_with_url("https://github.com/natepiano/demo"));
    // Default GitData carries two flat rate-limit rows, so the first
    // remote row sits at index 2.
    let remote_pos = 2;
    app.panes.git.set_content(data);
    app.panes.git.viewport.set_pos(remote_pos);

    let pane = GitPane;
    assert_eq!(
        pane.state(GitAction::Activate, &app),
        ShortcutState::Enabled,
        "Activate is Enabled on a Remote row whose full_url is Some",
    );
}

fn ci_data_with_runs(count: usize) -> CiData {
    let runs = (0..count)
        .map(|i| CiRun {
            run_id:          1 + i as u64,
            created_at:      "2026-04-01T21:00:00-04:00".to_string(),
            branch:          "main".to_string(),
            url:             format!("https://example.com/run/{}", 1 + i),
            ci_status:       CiStatus::Passed,
            jobs:            Vec::new(),
            wall_clock_secs: Some(17),
            commit_title:    Some("commit".to_string()),
            updated_at:      None,
            fetched:         FetchStatus::Fetched,
        })
        .collect();
    CiData {
        runs,
        mode_label: None,
        current_branch: None,
        empty_state: CiEmptyState::NoRuns,
    }
}

fn lints_data_with_runs(count: usize) -> LintsData {
    let runs = (0..count)
        .map(|i| LintRun {
            run_id:        format!("lint-{i}"),
            started_at:    "2026-04-01T21:00:00-04:00".to_string(),
            finished_at:   None,
            duration_ms:   None,
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        })
        .collect();
    LintsData {
        runs,
        sizes: Vec::new(),
        is_rust: true,
    }
}

#[test]
fn ci_runs_activate_visibility_hidden_at_eol() {
    // CiRuns `pane.visibility(Activate, ctx)` returns
    // `Visibility::Hidden` when the cursor is at or beyond the end of
    // the visible runs.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.ci.set_content(ci_data_with_runs(2));
    // Cursor at index == runs.len() — past the last run.
    app.ci.viewport.set_pos(2);

    let pane = CiRunsPane;
    assert_eq!(
        pane.visibility(CiRunsAction::Activate, &app),
        Visibility::Hidden,
        "Activate must be Hidden when cursor is past the visible runs",
    );
    assert_eq!(
        pane.visibility(CiRunsAction::FetchMore, &app),
        Visibility::Visible,
        "FetchMore stays Visible regardless of cursor position",
    );
}

#[test]
fn ci_runs_activate_visibility_visible_on_run_row() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.ci.set_content(ci_data_with_runs(2));
    app.ci.viewport.set_pos(0);

    let pane = CiRunsPane;
    assert_eq!(
        pane.visibility(CiRunsAction::Activate, &app),
        Visibility::Visible,
        "Activate is Visible when cursor sits on a real run row",
    );
}

#[test]
fn focused_project_list_bar_renders_pane_action_and_nav_slots() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    focus_app_pane_in_framework(&mut app, AppPaneId::ProjectList);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::ProjectList),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );
    let pane_action = flatten(&bar.pane_action);
    let nav = flatten(&bar.nav);

    // ProjectList keeps row expand/collapse keys active, but does not
    // spend bar space advertising them. Only the all pair lands in
    // the Nav region; no pane-local actions remain after Clean moved
    // to the global scope.
    assert!(
        pane_action.is_empty(),
        "ProjectList has no pane-local actions (got {pane_action:?})",
    );
    assert!(
        !nav.contains(" expand"),
        "ProjectList nav region must not show row expand help (got {nav:?})",
    );
    assert!(
        nav.contains("=/- all"),
        "ProjectList nav region must include the paired all row (got {nav:?})",
    );
    assert_contains_in_order(&nav, &["nav", "all"]);
    assert!(
        !nav.contains(" home") && !nav.contains(" end"),
        "ProjectList nav region must stay compact and omit Home/End rows (got {nav:?})",
    );
}

// ── Output (Mode::Navigable) ──────────────────────────────────────

#[test]
fn focused_output_bar_renders_select_and_close_labels() {
    // OutputPane is Navigable: the Nav region shows (the cursor scrolls
    // the buffer) alongside PaneAction, which carries the
    // OutputAction::SelectLinewise label "select" and the Cancel label
    // "close".
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    focus_app_pane_in_framework(&mut app, AppPaneId::Output);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Output),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );
    let pane_action = flatten(&bar.pane_action);
    let nav = flatten(&bar.nav);

    assert!(
        pane_action.contains("close"),
        "Output bar must show the Cancel label \"close\" (got {pane_action:?})",
    );
    assert!(
        pane_action.contains("select"),
        "Output bar must show the SelectLinewise label \"select\" (got {pane_action:?})",
    );
    assert!(
        !nav.is_empty(),
        "Navigable Output must surface the Nav region (got {nav:?})",
    );
}

// ── Finder (Mode::TextInput when open) ────────────────────────────

#[test]
fn finder_pane_mode_navigable_when_closed() {
    let project = super::make_project(Some("demo"), "~/demo");
    let app = make_app(&[project]);
    let mode_fn = <FinderPane as Pane<App>>::mode();
    assert!(
        matches!(mode_fn(&app), Mode::Navigable),
        "Finder mode must be Navigable when overlay is closed",
    );
}

#[test]
fn finder_text_input_inserts_char_into_query() {
    // When Finder is open, a typed letter goes through the framework's
    // TextInput handler and into the search query — vim mode is bypassed
    // by Mode::TextInput. We exercise the handler directly via the `fn`
    // pointer carried inside `Mode::TextInput(...)`.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.overlays.open_finder();

    let mode = <FinderPane as Pane<App>>::mode()(&app);
    let Mode::TextInput(handler) = mode else {
        panic!("expected Mode::TextInput when finder is open");
    };
    handler(KeyBind::from('k'), &mut app);

    assert_eq!(
        app.project_list.finder.query, "k",
        "TextInput handler must insert the typed character into the query",
    );
}

#[test]
fn focused_finder_open_bar_suppresses_all_regions() {
    // Open Finder → Mode::TextInput suppresses every bar region.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.overlays.open_finder();
    focus_app_pane_in_framework(&mut app, AppPaneId::Finder);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Finder),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );

    assert!(
        flatten(&bar.nav).is_empty(),
        "Mode::TextInput must suppress Nav (got {:?})",
        flatten(&bar.nav),
    );
    assert!(
        flatten(&bar.pane_action).is_empty(),
        "Mode::TextInput must suppress PaneAction (got {:?})",
        flatten(&bar.pane_action),
    );
    assert!(
        flatten(&bar.global).is_empty(),
        "Mode::TextInput must suppress Global (got {:?})",
        flatten(&bar.global),
    );
    let cargo_port_right = render::cargo_port_right_text_for_test(&app, &bar.global);
    assert!(
        cargo_port_right.is_empty(),
        "cargo-port global override must preserve TextInput global suppression (got {cargo_port_right:?})",
    );
}

// ── AppGlobalAction four-variant bar snapshots ────────────────────

#[test]
fn focused_package_bar_renders_every_app_global() {
    // Walks `AppGlobalAction::ALL` so adding a variant automatically
    // extends the assertion. Previously hardcoded `{find, editor,
    // terminal, rescan}` and silently stayed green after `Clean` was
    // added but missed from the status-line strip — that is the bug
    // class this test now guards against, paired with the const checks
    // in `tui::render`.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.package.set_content(package_data_no_version());
    focus_app_pane_in_framework(&mut app, AppPaneId::Package);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Package),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );
    let global = flatten(&bar.global);

    for variant in AppGlobalAction::ALL {
        let label = variant.bar_label();
        assert!(
            global.contains(label),
            "Global region must include AppGlobalAction::{variant:?} \
             label {label:?} (got {global:?})",
        );
    }
}

#[test]
fn focused_package_status_line_collapses_globals_to_shortcuts_help() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.package.set_content(package_data_no_version());
    focus_app_pane_in_framework(&mut app, AppPaneId::Package);

    let global = render::cargo_port_global_text_for_test(&app);

    assert_contains_in_order(&global, &["?", "shortcuts"]);
    assert!(
        !global.contains("finder") && !global.contains("editor") && !global.contains("quit"),
        "normal app-pane global strip should advertise only the shortcut viewer (got {global:?})",
    );
}

// ── Base-pane navigation routed through framework keymap ──────────

/// Rebinding `NavAction::Down` to `'j'` (vim-off) moves the
/// project-list cursor when `'j'` is dispatched through the real
/// `src/tui/input.rs` key path. Validates that `handle_normal_key`
/// consults the framework keymap's navigation scope after the legacy
/// pane-scope match.
#[test]
fn ctrl_b_and_ctrl_f_page_the_project_list() {
    let projects: Vec<_> = (0..40)
        .map(|i| super::make_project(Some("p"), &format!("~/p{i}")))
        .collect();
    let mut app = make_app_with_keymap_toml(&projects, "");
    let _ = buffer_text_sized(&mut app, 120, 30);
    assert_eq!(app.project_list.cursor(), 0);

    press(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    let after_ctrl_f = app.project_list.cursor();
    assert!(after_ctrl_f > 0, "Ctrl-f paged down (got {after_ctrl_f})");

    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert!(
        app.project_list.cursor() < after_ctrl_f,
        "Ctrl-b paged up from {after_ctrl_f}",
    );
}

#[test]
fn navigation_action_rebound_to_j_moves_cursor_down() {
    let projects = vec![
        super::make_project(Some("alpha"), "~/alpha"),
        super::make_project(Some("beta"), "~/beta"),
    ];
    let mut app = make_app_with_keymap_toml(&projects, "[navigation]\ndown = \"j\"\n");
    let baseline = app.project_list.cursor();

    let event = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    input::handle_event(&mut app, &event);

    assert_eq!(
        app.project_list.cursor(),
        baseline + 1,
        "cursor must advance after `'j'` resolves to NavAction::Down",
    );
}

/// A stale on-disk keymap with `home = ""` must NOT unbind Home — the
/// framework owns the navigation defaults and an empty value keeps the
/// compiled default. This is the original live bug: an empty entry left
/// over from an older keymap silently disabled Home/End at startup.
#[test]
fn empty_navigation_entry_keeps_the_compiled_default() {
    let projects = vec![
        super::make_project(Some("alpha"), "~/alpha"),
        super::make_project(Some("beta"), "~/beta"),
        super::make_project(Some("gamma"), "~/gamma"),
    ];
    let mut app = make_app_with_keymap_toml(&projects, "[navigation]\nhome = \"\"\n");

    // Move down so Home has somewhere to return from.
    for _ in 0..2 {
        input::handle_event(
            &mut app,
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
    }
    assert!(app.project_list.cursor() > 0, "cursor moved down");

    input::handle_event(
        &mut app,
        &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
    );

    assert_eq!(
        app.project_list.cursor(),
        0,
        "Home stays bound to its compiled default despite the empty TOML entry",
    );
}

/// Reloading the bindings the TOML writer emits (page keys as a
/// `["pageup", "ctrl-b"]` array, half-page as `ctrl-u` / `ctrl-d`) must
/// round-trip: every key still resolves to its action and the extra
/// Ctrl aliases do not trip the cross-action collision check at build.
#[test]
fn generated_navigation_defaults_round_trip_without_collision() {
    let projects = vec![super::make_project(Some("alpha"), "~/alpha")];
    let app = make_app_with_keymap_toml(
        &projects,
        "[navigation]\n\
         page_up = [\"pageup\", \"ctrl-b\"]\n\
         page_down = [\"pagedown\", \"ctrl-f\"]\n\
         half_page_up = \"ctrl-u\"\n\
         half_page_down = \"ctrl-d\"\n",
    );

    let nav = app
        .framework_keymap
        .navigation()
        .expect("navigation scope is registered");

    assert_eq!(
        nav.action_for(&KeyBind::from(KeyCode::PageUp)),
        Some(NavAction::PageUp),
    );
    assert_eq!(nav.action_for(&KeyBind::ctrl('b')), Some(NavAction::PageUp));
    assert_eq!(
        nav.action_for(&KeyBind::ctrl('f')),
        Some(NavAction::PageDown)
    );
    assert_eq!(
        nav.action_for(&KeyBind::ctrl('u')),
        Some(NavAction::HalfPageUp)
    );
    assert_eq!(
        nav.action_for(&KeyBind::ctrl('d')),
        Some(NavAction::HalfPageDown),
    );
}

/// Vim navigation keys drive the output pane through the shared viewport
/// navigation — `k` scrolls up off the tail (freezing the view), `j` and
/// `G` return to the tail (resuming follow) — with no output-specific
/// motion code.
#[test]
fn output_pane_navigates_with_vim_keys() {
    let projects = vec![super::make_project(Some("alpha"), "~/alpha")];
    let mut cfg = CargoPortConfig::default();
    cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
    let mut app = make_app_with_config_and_keymap_toml(&projects, &cfg, "");
    app.set_example_output((0..30).map(|i| format!("line {i}")).collect());
    // Render once so the viewport learns its length and visible rows.
    let _ = buffer_text_sized(&mut app, 120, 20);

    assert_eq!(app.focused_pane_id(), PaneId::Output);
    assert!(
        app.panes.output.is_following(),
        "the view opens following the streaming tail",
    );

    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    let _ = buffer_text_sized(&mut app, 120, 20);
    assert!(
        !app.panes.output.is_following(),
        "`k` scrolls up off the tail and freezes the view",
    );

    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let _ = buffer_text_sized(&mut app, 120, 20);
    assert!(
        app.panes.output.is_following(),
        "`j` back at the tail resumes following",
    );

    // Scroll up again, then `G` jumps to the tail and follows.
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    let _ = buffer_text_sized(&mut app, 120, 20);
    assert!(!app.panes.output.is_following());
    press(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    let _ = buffer_text_sized(&mut app, 120, 20);
    assert!(
        app.panes.output.is_following(),
        "`G` jumps to the tail and resumes following",
    );
}

#[test]
fn generated_home_end_entries_do_not_disable_vim_home_end_navigation() {
    let projects = vec![
        super::make_project(Some("alpha"), "~/alpha"),
        super::make_project(Some("beta"), "~/beta"),
        super::make_project(Some("gamma"), "~/gamma"),
    ];
    let mut cfg = CargoPortConfig::default();
    cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
    let mut app = make_app_with_config_and_keymap_toml(
        &projects,
        &cfg,
        "[navigation]\nhome = \"home\"\nend = \"end\"\n",
    );

    app.project_list.set_cursor(2);
    press(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    assert_eq!(app.project_list.cursor(), 2);
    press(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    assert_eq!(app.project_list.cursor(), 0);

    press(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    assert_eq!(app.project_list.cursor(), 2);
}

/// Rebinding `ProjectListAction::ExpandRow` to `Tab` (with
/// `GlobalAction::NextPane` rebound away) expands the current row.
/// Validates that the legacy pane-scope match in `handle_normal_key`
/// drives `ExpandRow` through its match arm.
#[test]
fn project_list_action_expand_row_rebound_to_tab_expands() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root_dir = tmp.path().join("repo");
    let sub_dir = root_dir.join("submod");
    fs::create_dir_all(&sub_dir).expect("create_dir_all");
    let root_path = root_dir.to_string_lossy().to_string();
    let sub_path = sub_dir.to_string_lossy().to_string();

    let project = super::make_project(Some("repo"), &root_path);
    let mut app = make_app_with_keymap_toml(
        &[project],
        "[global]\nnext_pane = \"F12\"\n[project_list]\nexpand_row = \"Tab\"\n",
    );

    let root_info = app
        .project_list
        .at_path_mut(Path::new(&root_path))
        .expect("root info");
    root_info.submodules.push(Submodule {
        name:          "submod".to_string(),
        path:          crate::project::AbsolutePath::from(sub_path),
        relative_path: "submod".to_string(),
        url:           None,
        branch:        None,
        commit:        None,
        info:          crate::project::ProjectInfo::default(),
        git_repo:      None,
    });
    app.ensure_visible_rows_cached();
    app.project_list.set_cursor(0);
    let baseline_rows = app.project_list.row_count();

    let event = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    input::handle_event(&mut app, &event);
    app.ensure_visible_rows_cached();

    assert!(
        app.project_list.row_count() > baseline_rows,
        "expanding the parent must reveal additional rows (was {baseline_rows}, now {})",
        app.project_list.row_count(),
    );
}

// ── Output structural cancel uses framework keymap ────────────────

fn assert_output_cancel_binding(
    keymap_toml: &str,
    key: KeyCode,
    starting_focus: Option<PaneId>,
    expected_focus: Option<PaneId>,
) {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app_with_keymap_toml(&[project], keymap_toml);
    if let Some(focus) = starting_focus {
        app.set_focus_to_pane(focus);
    }
    let focus_before = app.focused_pane_id();
    app.inflight.example_output_mut().push("line".to_string());

    let event = Event::Key(KeyEvent::new(key, KeyModifiers::NONE));
    input::handle_event(&mut app, &event);

    assert!(app.inflight.example_output().is_empty());
    assert_eq!(
        app.focused_pane_id(),
        expected_focus.unwrap_or(focus_before),
        "unexpected focus after structural output cancel",
    );
}

#[test]
fn output_cancel_bindings_clear_output_and_handle_focus() {
    for (toml, key, starting_focus, expected_focus) in [
        ("[output]\ncancel = \"q\"\n", KeyCode::Char('q'), None, None),
        (
            "[output]\ncancel = \"q\"\n",
            KeyCode::Char('q'),
            Some(PaneId::Output),
            Some(PaneId::Targets),
        ),
        (
            "[output]\ncancel = [\"Esc\", \"q\"]\n",
            KeyCode::Esc,
            None,
            None,
        ),
        (
            "[output]\ncancel = [\"Esc\", \"q\"]\n",
            KeyCode::Char('q'),
            None,
            None,
        ),
    ] {
        assert_output_cancel_binding(toml, key, starting_focus, expected_focus);
    }
}

// ── Keymap UI backed by framework keymap ──────────────────────────

#[test]
fn framework_keymap_template_matches_golden_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
    let project = super::make_project(Some("demo"), "~/demo");
    let app = make_app(&[project]);
    let generated = keymap_ui::current_keymap_toml(&app);
    let expected = include_str!("../../../../tests/assets/default-keymap.toml");

    assert_eq!(
        test_support::normalize_line_endings(&generated),
        test_support::normalize_line_endings(expected),
    );
}

#[test]
fn keymap_template_omits_generated_vim_bindings() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut cfg = CargoPortConfig::default();
    cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
    let app = make_app_with_config_and_keymap_toml(&[project], &cfg, "");

    let generated = keymap_ui::current_keymap_toml(&app);

    assert!(generated.contains("down           = \"down\""));
    assert!(generated.contains("left           = \"left\""));
    assert!(generated.contains("collapse_row = \"left\""));
    assert!(generated.contains("expand_row   = \"right\""));
    assert!(!generated.contains("[\"down\", \"j\"]"));
    assert!(!generated.contains("[\"left\", \"h\"]"));
    assert!(!generated.contains("[\"shift-left\", \"h\"]"));
    assert!(!generated.contains("[\"shift-right\", \"l\"]"));
}

#[test]
fn startup_warns_for_ignored_reserved_vim_keymap_bindings() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut cfg = CargoPortConfig::default();
    cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
    let app = make_app_with_config_and_keymap_toml(
        &[project],
        &cfg,
        "[project_list]\ncollapse_row = [\"shift-left\", \"h\"]\nexpand_row = [\"shift-right\", \"l\"]\n",
    );

    let warnings = app
        .framework
        .toasts
        .active_now()
        .into_iter()
        .filter(|toast| toast.title() == "Keymap warnings")
        .collect::<Vec<_>>();

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].body().contains("project_list.expand_row"));
    assert!(warnings[0].body().contains("project_list.collapse_row"));
    assert!(!warnings[0].body().contains("using defaults"));
}

#[test]
fn keymap_ui_save_preserves_framework_owned_scopes() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(
        &toml_path,
        "[output]\ncancel = \"q\"\n\
         [finder]\nactivate = \"Tab\"\n\
         [overlay]\nstart_edit = \"F2\"\n",
    )
    .expect("write keymap toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    keymap_ui::save_current_keymap_to_disk(&mut app);
    let saved = fs::read_to_string(&toml_path).expect("read keymap toml");

    assert!(saved.contains("[finder]"));
    assert!(saved.contains("activate = \"tab\""));
    assert!(saved.contains("[output]"));
    // The output scope now aligns its keys (select_linewise is wider), so
    // match the preserved cancel binding without depending on padding.
    assert!(
        saved
            .lines()
            .any(|line| line.starts_with("cancel") && line.contains("\"q\"")),
        "custom output cancel binding must be preserved (got {saved:?})",
    );
    assert!(saved.contains("[overlay]"));
    assert!(saved.contains("start_edit = \"f2\""));
}

#[test]
fn external_keymap_reload_updates_framework_owned_scope() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(&toml_path, "[output]\ncancel = \"Esc\"\n").expect("write keymap toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    fs::write(
        &toml_path,
        "[output]\ncancel = \"q\"\n[finder]\nactivate = \"Tab\"\n",
    )
    .expect("rewrite keymap toml");
    app.maybe_reload_keymap_from_disk();

    assert_eq!(
        app.framework_keymap
            .key_for_toml_key(AppPaneId::Output, OutputAction::Cancel.toml_key()),
        Some(tui_pane::KeySequence::from(KeyBind {
            code: KeyCode::Char('q'),
            mods: KeyModifiers::NONE,
        })),
    );
}

#[test]
fn external_keymap_reload_missing_actions_does_not_rewrite_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    let keymap_path_guard = keymap::override_keymap_path_for_test(toml_path.clone());
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    keymap_ui::save_current_keymap_to_disk(&mut app);
    let edited = "[output]\n# cancel = \"Esc\"\n";
    fs::write(&toml_path, edited).expect("rewrite keymap toml");

    app.maybe_reload_keymap_from_disk();

    let saved = fs::read_to_string(&toml_path).expect("read keymap toml");
    assert_eq!(saved, edited);
    assert!(
        app.framework
            .toasts
            .active_now()
            .iter()
            .any(|toast| toast.title() == "Keymap warnings"),
        "missing entries should warn without rewriting the user's in-progress edit"
    );
    drop(keymap_path_guard);
}

#[test]
fn legacy_project_list_removed_actions_migrate_before_framework_load() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(
        &toml_path,
        "[project_list]\nopen_editor = \"E\"\nrescan = \"Ctrl+r\"\n",
    )
    .expect("write keymap toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
    let project = super::make_project(Some("demo"), "~/demo");
    let app = make_app(&[project]);

    let globals = app
        .framework_keymap
        .globals::<AppGlobalAction>()
        .expect("app globals registered");
    assert_eq!(
        globals.action_for(&KeyBind::from('E')),
        Some(AppGlobalAction::OpenEditor),
    );
    assert_eq!(
        globals.action_for(&KeyBind::ctrl('r')),
        Some(AppGlobalAction::Rescan),
    );

    let saved = fs::read_to_string(&toml_path).expect("read migrated keymap toml");
    let table: Table = saved.parse().expect("parse migrated keymap toml");
    let project_list = table
        .get("project_list")
        .and_then(toml::Value::as_table)
        .expect("project_list table");
    assert!(!project_list.contains_key("open_editor"));
    assert!(!project_list.contains_key("rescan"));
    let global = table
        .get("global")
        .and_then(toml::Value::as_table)
        .expect("global table");
    assert_eq!(
        global.get("open_editor").and_then(toml::Value::as_str),
        Some("E"),
    );
    assert_eq!(
        global.get("rescan").and_then(toml::Value::as_str),
        Some("ctrl-r"),
    );
}

#[test]
fn legacy_project_list_removed_action_does_not_override_framework_global() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    fs::write(
        &toml_path,
        "[global]\nopen_editor = \"E\"\n[project_list]\nopen_editor = \"Enter\"\n",
    )
    .expect("write keymap toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
    let project = super::make_project(Some("demo"), "~/demo");
    let app = make_app(&[project]);

    let globals = app
        .framework_keymap
        .globals::<AppGlobalAction>()
        .expect("app globals registered");
    assert_eq!(
        globals.action_for(&KeyBind::from('E')),
        Some(AppGlobalAction::OpenEditor),
    );
    assert_ne!(
        globals.action_for(&KeyBind::from(KeyCode::Enter)),
        Some(AppGlobalAction::OpenEditor),
    );
}

#[test]
fn keymap_popup_keeps_legacy_global_layout_compact() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

    let text = buffer_text_sized(&mut app, 120, 80);

    assert_contains_in_order(
        &text,
        &[
            "Global Navigation:",
            "Next pane",
            "Global Shortcuts:",
            "Dismiss overlay / output",
            "Open finder",
            "Open keymap viewer",
            "Show global shortcuts",
            "Project List:",
        ],
    );
    assert!(
        !text.contains("App Global Shortcuts:"),
        "app-owned globals must stay merged into the legacy Global Shortcuts section",
    );
    assert!(
        !text.contains("Close finder"),
        "the keymap popup should stay compact on tall terminals instead of exposing every section",
    );
}

#[test]
fn global_shortcuts_overlay_opens_with_question_mark_and_esc_closes() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);

    assert_eq!(
        app.framework.overlay(),
        Some(tui_pane::FrameworkOverlayId::GlobalShortcuts)
    );

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert_eq!(app.framework.overlay(), None);
}

#[test]
fn global_shortcuts_overlay_renders_all_global_shortcuts() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    open_framework_overlay(&mut app, FrameworkGlobalAction::OpenGlobalShortcuts);

    let text = buffer_text_sized(
        &mut app,
        GLOBAL_SHORTCUTS_TEST_WIDTH,
        GLOBAL_SHORTCUTS_TEST_HEIGHT,
    );

    assert_contains_in_order(
        &text,
        &[
            "Global Shortcuts",
            "Global Navigation:",
            "Next pane",
            "Global Shortcuts:",
            "Open finder",
            "Quit",
            "Show global shortcuts",
        ],
    );
}

#[test]
fn keymap_popup_renders_framework_overflow_affordance() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let toml_path = temp_dir.path().join("keymap.toml");
    let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

    let text = buffer_text_sized(&mut app, 120, 18);

    assert!(text.contains("Keymap"));
    assert!(
        text.contains("1 of"),
        "keymap overlay should render the framework-owned overflow marker"
    );
}

// ── Framework-owned live tab cycle ────────────────────────────────

#[test]
fn tab_from_package_lands_on_git_when_lang_is_unavailable() {
    let mut app = make_app_with_git_tabbable();
    app.set_focus(FocusedPane::App(AppPaneId::Package));

    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(app.focused_pane_id(), PaneId::Git);
    assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
}

#[test]
fn repeated_tab_never_lands_on_unavailable_lang() {
    let mut app = make_app_with_git_tabbable();
    app.set_focus(FocusedPane::App(AppPaneId::Package));

    for step in 0..TAB_WALK_STEPS {
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_ne!(app.focused_pane_id(), PaneId::Lang, "step {step}");
    }
}

#[test]
fn shift_tab_skips_unavailable_panes_in_reverse() {
    let mut app = make_app_with_git_tabbable();
    app.set_focus(FocusedPane::App(AppPaneId::Cpu));

    press(&mut app, KeyCode::Tab, KeyModifiers::SHIFT);

    assert_eq!(app.focused_pane_id(), PaneId::Git);
    assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
}

#[test]
fn output_active_excludes_diagnostics_and_reaches_output() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.targets.set_content(TargetsData {
        binaries: vec![crate::tui::panes::TargetEntry {
            name:              "demo".to_string(),
            display_name:      "demo".to_string(),
            kind:              crate::tui::panes::RunTargetKind::Binary,
            source:            crate::tui::panes::TargetSource::Workspace,
            src_path:          crate::project::AbsolutePath::from("/tmp/demo/src/main.rs"),
            required_features: Vec::new(),
        }],
        examples: Vec::new(),
        benches:  Vec::new(),
    });
    app.lint.set_content(lints_data_with_runs(SINGLE_RUN_COUNT));
    app.ci.set_content(ci_data_with_runs(SINGLE_RUN_COUNT));
    app.inflight.example_output_mut().push("line".to_string());
    app.set_focus(FocusedPane::App(AppPaneId::Targets));

    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(app.focused_pane_id(), PaneId::Output);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(AppPaneId::Output),
    );
}

#[test]
fn rebound_next_pane_uses_framework_filtered_tab_cycle() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app_with_keymap_toml(&[project], "[global]\nnext_pane = \"F8\"\n");
    app.panes.git.set_content(GitData {
        head: Some(HeadState::Branch("main".to_string())),
        ..GitData::default()
    });
    app.set_focus(FocusedPane::App(AppPaneId::Package));

    press(&mut app, KeyCode::F(8), KeyModifiers::NONE);

    assert_eq!(app.focused_pane_id(), PaneId::Git);
    assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
}

#[test]
fn settings_text_input_esc_wins_over_output_cancel_preflight() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    open_framework_overlay(&mut app, FrameworkGlobalAction::OpenSettings);
    app.framework
        .settings_pane
        .viewport_mut()
        .set_pos(SettingOption::CiRunCount as usize);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    app.inflight.example_output_mut().push("line".to_string());

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(
        !app.inflight.example_output().is_empty(),
        "settings edit cancel must not clear example output",
    );
    assert!(
        !app.framework.settings_pane.is_editing(),
        "Esc must still leave settings edit mode",
    );
}

// ── Overlay input/render ownership ────────────────────────────────

#[test]
fn finder_cancel_rebind_closes_finder_through_production_input() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app_with_keymap_toml(&[project], "[finder]\ncancel = \"q\"\n");
    input::open_finder(&mut app);

    press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);

    assert!(!app.overlays.is_finder_open());
    assert!(app.project_list.finder.query.is_empty());
}

#[test]
fn finder_text_input_keeps_vim_k_as_query_text() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut cfg = CargoPortConfig::default();
    cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
    let mut app = super::make_app_with_config(&[project], &cfg);
    input::open_finder(&mut app);

    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);

    assert_eq!(app.project_list.finder.query, "k");
}

#[test]
fn finder_activate_rebind_wins_over_global_tab_while_finder_is_open() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app_with_keymap_toml(
        &[project],
        "[global]\nnext_pane = \"Tab\"\n[finder]\nactivate = \"Tab\"\n",
    );
    input::open_finder(&mut app);
    app.project_list.finder.results = vec![0];
    app.project_list.finder.total = 1;
    let base_before = app.base_focus();

    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert!(!app.overlays.is_finder_open());
    assert_eq!(
        app.focused_pane_id(),
        base_before,
        "finder Activate must consume Tab before global pane cycling",
    );
}

#[test]
fn keymap_capture_rejects_navigation_key_through_production_input() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);

    assert!(app.framework.keymap_pane.is_capturing());
    assert!(
        app.overlays
            .inline_error()
            .is_some_and(|error| error.contains("reserved for navigation")),
    );
}

/// The `App::set_focus` override updates framework focus and records
/// app-pane visits for render selection styling.
#[test]
fn set_focus_override_updates_framework_focus_and_visits() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    app.set_focus(FocusedPane::App(AppPaneId::Targets));
    assert!(matches!(
        app.framework().focused(),
        FocusedPane::App(AppPaneId::Targets)
    ));
    assert_eq!(app.focused_pane_id(), panes::PaneId::Targets);

    app.set_focus(FocusedPane::App(AppPaneId::Git));
    assert!(matches!(
        app.framework().focused(),
        FocusedPane::App(AppPaneId::Git)
    ));
    assert_eq!(app.focused_pane_id(), panes::PaneId::Git);
    assert_eq!(
        app.pane_focus_state(panes::PaneId::Targets),
        tui_pane::PaneFocusState::Remembered
    );

    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
    assert!(matches!(
        app.framework().focused(),
        FocusedPane::Framework(FrameworkFocusId::Toasts),
    ));
    assert_eq!(app.focused_pane_id(), panes::PaneId::Toasts);
}

#[test]
fn focused_toasts_without_action_falls_through_to_app_globals() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app_with_keymap_toml(&[project], "[global]\nfind = \"Enter\"\n");
    let _ = app.framework.toasts.push("Build done", "ok");
    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));

    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(app.overlays.is_finder_open());
}

#[test]
fn enter_on_focused_toast_with_action_dispatches() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.config.current_mut().tui.editor = "/definitely/missing/cargo-port-editor".to_string();
    let action_path =
        crate::project::AbsolutePath::from(std::path::PathBuf::from("/tmp/cargo-port-keymap.toml"));
    let _ = app.framework.toasts.push_with_action(
        "Keymap errors",
        "bad binding",
        CargoPortToastAction::OpenPath(action_path),
    );
    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));

    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(
        app.framework
            .toasts
            .active_now()
            .iter()
            .any(|toast| toast.title() == "Toast action failed"),
        "Enter on a focused toast with an action should dispatch the cargo-port toast action"
    );
}

#[test]
fn focused_package_bar_nav_region_renders_arrow_keys() {
    // Lock the framework's nav-region rendering for a focused
    // Mode::Navigable pane. The nav region surfaces the pane-cycle row
    // plus the navigation defaults; the keymap's default for
    // `NavAction::Up` is `↑` so we look for that glyph as a
    // stable anchor.
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.package.set_content(package_data_no_version());
    focus_app_pane_in_framework(&mut app, AppPaneId::Package);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Package),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );
    let nav = flatten(&bar.nav);

    assert_contains_in_order(&nav, &["↑/↓", "nav", "tab", "pane"]);
}
