//! Tests for the framework-keymap path landed in Phase 14.2 and Phase 14.3.
//!
//! - Bar snapshots assert that `tui_pane::render_status_bar` produces the expected pane-action
//!   labels when Package or Git is focused. They read `bar.pane_action` only — the global and nav
//!   regions are covered separately by Phase 14.7's snapshots.
//! - The `state` tests pin the `Shortcuts::state` rules that gray out `Activate` when the cursor
//!   sits on a row whose dispatch has no effect (Package's non-`CratesIo` rows; Git's flat fields
//!   and any remote without a URL).

use ratatui::text::Span;
use tui_pane::AppContext;
use tui_pane::BarPalette;
use tui_pane::FocusedPane;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;
use tui_pane::render_status_bar;

use super::App;
use super::make_app;
use crate::keymap::GitAction;
use crate::keymap::PackageAction;
use crate::tui::framework_keymap::AppPaneId;
use crate::tui::framework_keymap::GitPane;
use crate::tui::framework_keymap::PackagePane;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitData;
use crate::tui::panes::PackageData;
use crate::tui::panes::RemoteRow;

fn focus_app_pane_in_framework(app: &mut App, id: AppPaneId) {
    app.framework_mut().set_focused(FocusedPane::App(id));
}

fn flatten(spans: &[Span<'static>]) -> String {
    let mut out = String::new();
    for span in spans {
        out.push_str(&span.content);
    }
    out
}

fn package_data_no_version() -> PackageData {
    PackageData {
        package_title:            "Package".to_string(),
        title_name:               "demo".to_string(),
        path:                     "~/demo".to_string(),
        version:                  "0.1.0".to_string(),
        description:              None,
        crates_version:           None,
        crates_downloads:         None,
        types:                    "lib".to_string(),
        disk:                     "1.0 MiB".to_string(),
        stats_rows:               Vec::new(),
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
fn focused_package_bar_renders_pane_action_labels() {
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
    let pane_action = flatten(&bar.pane_action);

    assert!(
        pane_action.contains("activate"),
        "Package pane bar must show the Activate label (got {pane_action:?})",
    );
    assert!(
        pane_action.contains("clean"),
        "Package pane bar must show the Clean label (got {pane_action:?})",
    );
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
    assert_eq!(
        pane.state(PackageAction::Clean, &app),
        ShortcutState::Enabled,
        "Clean is unaffected by the cursor-row rule",
    );
}

#[test]
fn package_activate_state_enabled_on_crates_io_with_version() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    let mut data = package_data_no_version();
    data.crates_version = Some("0.1.0".to_string());
    let fields = panes::package_fields_from_data(&data);
    let crates_io_pos = fields
        .iter()
        .position(|f| matches!(f, DetailField::CratesIo))
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
        name:        "origin".to_string(),
        icon:        "",
        display_url: url.to_string(),
        tracked_ref: String::new(),
        status:      String::new(),
        full_url:    Some(url.to_string()),
    }
}

#[test]
fn focused_git_bar_renders_pane_action_labels() {
    let project = super::make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.panes.git.set_content(GitData::default());
    focus_app_pane_in_framework(&mut app, AppPaneId::Git);

    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Git),
        &app,
        &app.framework_keymap,
        app.framework(),
        &palette,
    );
    let pane_action = flatten(&bar.pane_action);

    assert!(
        pane_action.contains("activate"),
        "Git pane bar must show the Activate label (got {pane_action:?})",
    );
    assert!(
        pane_action.contains("clean"),
        "Git pane bar must show the Clean label (got {pane_action:?})",
    );
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
    assert_eq!(
        pane.state(GitAction::Clean, &app),
        ShortcutState::Enabled,
        "Clean is unaffected by the cursor-row rule",
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
