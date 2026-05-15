use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::ACCENT_COLOR;
use tui_pane::BLOCK_BORDER_WIDTH;
use tui_pane::BYTES_PER_GIB;
use tui_pane::BYTES_PER_KIB;
use tui_pane::BYTES_PER_MIB;
use tui_pane::BarPalette;
use tui_pane::ERROR_COLOR;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::LABEL_COLOR;
use tui_pane::PaneFocusState;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::ResolvedPaneLayout;
use tui_pane::SECONDARY_TEXT_COLOR;
use tui_pane::STATUS_BAR_COLOR;
use tui_pane::SUCCESS_COLOR;
use tui_pane::ShortcutState;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::TITLE_COLOR;
use tui_pane::render_status_line as render_framework_status_line;
use tui_pane::render_toasts;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::app::ConfirmAction;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::integration::AppGlobalAction;
use super::interaction;
use super::keymap_ui;
use super::overlays::PopupFrame;
use super::pane::PaneRenderCtx;
use super::panes;
use super::panes::PaneId;
use super::settings;
use crate::ci::CiStatus;
use crate::project;
use crate::project::AbsolutePath;

#[derive(Clone, Copy)]
pub(super) enum CiColumn {
    Fmt,
    Taplo,
    Clippy,
    Mend,
    Build,
    Test,
    Bench,
}

impl CiColumn {
    pub(super) fn matches(self, job_name: &str) -> bool {
        let lower = job_name.to_lowercase();
        match self {
            Self::Fmt => lower.contains("format") || lower.contains("fmt"),
            Self::Taplo => lower.contains("taplo"),
            Self::Clippy => lower.contains("clippy"),
            Self::Mend => lower.contains("mend"),
            Self::Build => lower.contains("build"),
            Self::Test => lower.contains("test"),
            Self::Bench => lower.contains("bench"),
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Taplo => "taplo",
            Self::Clippy => "clippy",
            Self::Mend => "mend",
            Self::Build => "build",
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

pub(super) fn format_bytes(bytes: u64) -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "display-only — sub-byte precision is irrelevant"
    )]
    if bytes >= BYTES_PER_GIB {
        format!("{:.1} GiB", bytes as f64 / BYTES_PER_GIB as f64)
    } else if bytes >= BYTES_PER_MIB {
        format!("{:.1} MiB", bytes as f64 / BYTES_PER_MIB as f64)
    } else if bytes >= BYTES_PER_KIB {
        format!("{:.1} KiB", bytes as f64 / BYTES_PER_KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn conclusion_style(ci_status: Option<CiStatus>) -> Style {
    match ci_status {
        Some(CiStatus::Passed) => Style::default().fg(SUCCESS_COLOR),
        Some(CiStatus::Failed) => Style::default().fg(ERROR_COLOR),
        _ => Style::default(),
    }
}

pub(super) fn ui(frame: &mut Frame, app: &mut App) {
    sync_hovered_pane_row(app);
    app.panes.tiled_layout = ResolvedPaneLayout::default();
    app.panes.project_list.body_rect = Rect::ZERO;
    app.prune_toasts();

    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let left_width =
        u16::try_from(app.project_list.cached_fit_widths.total_width() + BLOCK_BORDER_WIDTH + 1)
            .unwrap_or(u16::MAX);

    let bottom_row = if app.inflight.example_output().is_empty() {
        panes::BottomRow::Diagnostics
    } else {
        panes::BottomRow::Output
    };
    let core_count = app.panes.cpu.content().map_or(1, |usage| usage.cores.len());
    let tiled = panes::resolve_layout(outer_layout[0], left_width, core_count, bottom_row);

    // Stamp every renderable pane's focus snapshot before splitting
    // App so each pane can read its own focus state from `&mut self`
    // inside the trait-dispatched render loop.
    sync_pane_focus(app);

    // Build the CI lookup snapshot now, while we still hold `&app.ci`;
    // the upcoming split takes `&mut app.ci` for the registry, which
    // would alias an `&Ci` ref carried in the ctx.
    let ci_status_lookup = app.ci.status_lookup();

    // `selected_project_path` needs both `&Selection` and `&Scan`;
    // resolve and own it before the split releases those borrows.
    let selected_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let animation_elapsed = app.animation_started.elapsed();

    {
        let mut split = app.split_for_render(
            selected_path.as_deref(),
            animation_elapsed,
            &ci_status_lookup,
            None,
            None,
        );
        tui_pane::render_panes(frame, &mut split.registry, &tiled, &split.ctx);
    }
    app.panes.tiled_layout = tiled;

    render_status_bar(frame, app, outer_layout[1]);
    let toast_settings = app.framework.toast_settings();
    let active_toasts = app
        .framework
        .toasts
        .active_views(std::time::Instant::now(), toast_settings);
    let toast_result = render_toasts(
        frame,
        outer_layout[0],
        &active_toasts,
        toast_settings,
        app.focus_is(PaneId::Toasts),
        app.framework.toasts.focused_toast_id(),
    );
    app.framework.toasts.set_hits(toast_result.hitboxes);

    if app.framework.overlay() == Some(FrameworkOverlayId::Settings) {
        dispatch_settings_overlay(app, frame);
    }
    if app.framework.overlay() == Some(FrameworkOverlayId::Keymap) {
        dispatch_keymap_overlay(app, frame);
    }
    if app.overlays.is_finder_open() {
        dispatch_finder_render(app, frame);
    }
    if let Some(action) = app.confirm() {
        let body = confirm_action_body(app, action);
        let verifying = app.scan.confirm_verifying().is_some();
        render_confirm_popup(frame, action, &body, verifying);
    }

    sync_hovered_pane_row(app);
}

/// Maximum affected-checkout paths shown explicitly in the confirm
/// dialog before collapsing the tail into a `+N more` line (design
/// plan → "Confirm dialog → uniform rule").
const AFFECTED_EXTRAS_VISIBLE_CAP: usize = 5;

/// Body lines shown below the `Run cargo clean?` prompt — everything
/// from the resolved target dir to the "Also affected" list and the
/// nested-crate summary. Pre-formatted into strings so render stays
/// a dumb pass-through.
fn confirm_action_body(app: &App, action: &ConfirmAction) -> Vec<String> {
    match action {
        ConfirmAction::Clean(project_path) => {
            let target = app
                .scan
                .resolve_target_dir(project_path)
                .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join("target")));
            let mut lines = vec![project::home_relative_path(target.as_path())];

            // Report affected siblings (step 6d): projects that share
            // this target dir but are not the selection. The
            // TargetDirIndex is populated incrementally from
            // handle_cargo_metadata_msg, so early in startup it may
            // be empty — then these lists stay empty and the dialog
            // reverts to the Step 2 single-line layout.
            let selection = [project_path.clone()];
            append_sibling_lines(app, &target, &selection, &mut lines);

            lines
        },
        ConfirmAction::CleanGroup { primary, linked } => {
            let mut lines = vec!["Checkouts:".to_string()];
            // Render every checkout the fan-out will hit, capped so
            // large groups don't overflow the popup. Collapse the tail
            // behind `+N more` using the same cap as sibling lines.
            let all_paths: Vec<&AbsolutePath> = std::iter::once(primary).chain(linked).collect();
            for path in all_paths.iter().take(AFFECTED_EXTRAS_VISIBLE_CAP) {
                lines.push(format!("  {}", project::home_relative_path(path.as_path())));
            }
            if all_paths.len() > AFFECTED_EXTRAS_VISIBLE_CAP {
                let extra = all_paths.len() - AFFECTED_EXTRAS_VISIBLE_CAP;
                lines.push(format!("  +{extra} more"));
            }

            // Union of all siblings across every resolved target dir —
            // a group clean can affect sibling projects outside the
            // selection just like a single-project clean can.
            let selection: Vec<AbsolutePath> = all_paths.iter().copied().cloned().collect();
            let mut seen_targets: HashSet<AbsolutePath> = std::collections::HashSet::new();
            for path in &all_paths {
                let target = app
                    .scan
                    .resolve_target_dir(path)
                    .unwrap_or_else(|| AbsolutePath::from(path.as_path().join("target")));
                if seen_targets.insert(target.clone()) {
                    append_sibling_lines(app, &target, &selection, &mut lines);
                }
            }

            lines
        },
    }
}

/// Append the "Also affects:" block (sibling project paths + optional
/// nested-crate summary) for a single resolved target dir. Shared
/// between the `Clean` and `CleanGroup` body builders.
fn append_sibling_lines(
    app: &App,
    target: &AbsolutePath,
    selection: &[AbsolutePath],
    lines: &mut Vec<String>,
) {
    let siblings = app.scan.target_dir_index.siblings(target, selection);
    let project_siblings = siblings;
    if !project_siblings.is_empty() {
        lines.push("Also affects:".to_string());
        for sibling in project_siblings.iter().take(AFFECTED_EXTRAS_VISIBLE_CAP) {
            lines.push(format!(
                "  {}",
                project::home_relative_path(sibling.as_path())
            ));
        }
        if project_siblings.len() > AFFECTED_EXTRAS_VISIBLE_CAP {
            let extra = project_siblings.len() - AFFECTED_EXTRAS_VISIBLE_CAP;
            lines.push(format!("  +{extra} more"));
        }
    }
}

fn render_confirm_popup(
    frame: &mut Frame,
    action: &ConfirmAction,
    body: &[String],
    verifying: bool,
) {
    // While the fingerprint re-check is in flight we swap the prompt
    // + keys for a "Verifying target dir…" placeholder and drop the
    // (y/n) suffix — `y` is ignored by handle_confirm_key in that
    // state, and showing it enabled would mislead the user.
    let prompt = match action {
        ConfirmAction::Clean(_) => "Run cargo clean?",
        ConfirmAction::CleanGroup { .. } => "Run cargo clean on all checkouts?",
    };
    let keys_suffix = if verifying { "" } else { " (y/n)" };
    let prompt_text = if verifying {
        " Verifying target dir… ".to_string()
    } else {
        format!(" {prompt} {keys_suffix} ")
    };
    let prompt_width = prompt_text.len();
    let body_max = body.iter().map(String::len).max().unwrap_or(0);
    // leading " " + trailing " " around the widest body line.
    let body_width = if body_max == 0 { 0 } else { body_max + 2 };
    let width = u16::try_from(prompt_width.max(body_width) + 4).unwrap_or(u16::MAX);
    let body_height = u16::try_from(body.len()).unwrap_or(u16::MAX);
    let height = CONFIRM_DIALOG_HEIGHT.saturating_add(body_height);

    let inner = PopupFrame {
        title: None,
        border_color: TITLE_COLOR,
        width,
        height,
    }
    .render(frame);

    let mut lines = if verifying {
        vec![Line::from(vec![Span::styled(
            " Verifying target dir… ",
            Style::default()
                .fg(LABEL_COLOR)
                .add_modifier(Modifier::ITALIC),
        )])]
    } else {
        vec![Line::from(vec![
            Span::styled(format!(" {prompt}  "), Style::default().fg(Color::White)),
            Span::styled(
                "(y/n)",
                Style::default()
                    .fg(TITLE_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
        ])]
    };
    for body_line in body {
        lines.push(Line::from(vec![Span::styled(
            format!(" {body_line} "),
            Style::default().fg(LABEL_COLOR),
        )]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Dispatch the Keymap overlay popup via [`tui_pane::Renderable`].
///
/// The expensive `&App`-reading work (walking `framework_keymap`,
/// laying out rows, building lines) happens here before
/// `App::split_for_render`; the trait method on `KeymapPane` reads
/// the resulting [`crate::tui::keymap_ui::KeymapRenderInputs`] from
/// `PaneRenderCtx` and draws into `frame`.
fn dispatch_keymap_overlay(app: &mut App, frame: &mut Frame) {
    // Overlay focus is always `Active` while the popup is open.
    app.framework.keymap_pane.focus = RenderFocus {
        state:      PaneFocusState::Active,
        is_focused: true,
    };
    let inputs = keymap_ui::prepare_keymap_render_inputs(app);
    let animation_elapsed = app.animation_started.elapsed();
    let selected_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let ci_status_lookup = app.ci.status_lookup();
    let split = app.split_for_render(
        selected_path.as_deref(),
        animation_elapsed,
        &ci_status_lookup,
        Some(&inputs),
        None,
    );
    Renderable::render(split.registry.keymap_pane, frame, frame.area(), &split.ctx);
}

/// Dispatch the Settings overlay popup via [`tui_pane::Renderable`].
/// Mirror of [`dispatch_keymap_overlay`] — the precompute step calls
/// [`tui_pane::SettingsPane::render_rows`] (which mutates the pane)
/// before `App::split_for_render`, then the trait method draws the
/// popup.
fn dispatch_settings_overlay(app: &mut App, frame: &mut Frame) {
    app.framework.settings_pane.focus = RenderFocus {
        state:      PaneFocusState::Active,
        is_focused: true,
    };
    let frame_height = frame.area().height;
    let inputs = settings::prepare_settings_render_inputs(app, frame_height);
    let animation_elapsed = app.animation_started.elapsed();
    let selected_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let ci_status_lookup = app.ci.status_lookup();
    let split = app.split_for_render(
        selected_path.as_deref(),
        animation_elapsed,
        &ci_status_lookup,
        None,
        Some(&inputs),
    );
    Renderable::render(
        split.registry.settings_pane,
        frame,
        frame.area(),
        &split.ctx,
    );
}

fn dispatch_finder_render(app: &mut App, frame: &mut Frame) {
    let finder_focus = RenderFocus {
        state:      app.pane_focus_state(PaneId::Finder),
        is_focused: app.focus_is(PaneId::Finder),
    };
    app.overlays.finder_pane.focus = finder_focus;
    let animation_elapsed = app.animation_started.elapsed();
    let selected_project_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let ci_status_lookup = app.ci.status_lookup();
    let split = app.split_finder_for_render();
    let ctx = PaneRenderCtx {
        animation_elapsed,
        config: split.config,
        project_list: split.project_list,
        selected_project_path: selected_project_path.as_deref(),
        inflight: split.inflight,
        scan: split.scan,
        ci_status_lookup: &ci_status_lookup,
        keymap_render_inputs: None,
        settings_render_inputs: None,
        inline_error: split.inline_error,
    };
    // Finder body sizes the popup itself; area arg is unused.
    Renderable::render(split.finder_pane, frame, frame.area(), &ctx);
}

/// Stamp each renderable pane's [`tui_pane::RenderFocus`] snapshot
/// before [`tui_pane::render_panes`] dispatches the loop. After this,
/// every pane reads its own focus state from `&mut self` instead of
/// the shared [`PaneRenderCtx`] — which is what frees the ctx of any
/// per-pane field and lets the generic loop carry one ctx per frame.
fn sync_pane_focus(app: &mut App) {
    let ids = [
        PaneId::Package,
        PaneId::Lang,
        PaneId::Cpu,
        PaneId::Git,
        PaneId::Targets,
        PaneId::ProjectList,
        PaneId::Output,
        PaneId::Lints,
        PaneId::CiRuns,
    ];
    for id in ids {
        let focus = RenderFocus {
            state:      app.pane_focus_state(id),
            is_focused: app.focus_is(id),
        };
        match id {
            PaneId::Package => app.panes.package.focus = focus,
            PaneId::Lang => app.panes.lang.focus = focus,
            PaneId::Cpu => app.panes.cpu.focus = focus,
            PaneId::Git => app.panes.git.focus = focus,
            PaneId::Targets => app.panes.targets.focus = focus,
            PaneId::ProjectList => app.panes.project_list.focus = focus,
            PaneId::Output => app.panes.output.focus = focus,
            PaneId::Lints => app.lint.focus = focus,
            PaneId::CiRuns => app.ci.focus = focus,
            PaneId::Toasts | PaneId::Settings | PaneId::Finder | PaneId::Keymap => {},
        }
    }
}

fn sync_hovered_pane_row(app: &mut App) {
    let hovered = app
        .mouse_pos
        .and_then(|pos| interaction::hovered_pane_row_at(app, pos));
    app.panes.set_hover(hovered);
    app.apply_hovered_pane_row();
}

pub(super) fn truncate_to_width(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }

    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if next.width() > max_width {
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn truncate_with_ellipsis(text: &str, max_width: usize, ellipsis: &str) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width <= ellipsis.width() {
        return ellipsis.to_string();
    }
    let prefix = truncate_to_width(text, max_width.saturating_sub(ellipsis.width()));
    format!("{prefix}{ellipsis}")
}

/// Palette wiring `ACCENT_COLOR` / `SECONDARY_TEXT_COLOR` / `Modifier::BOLD`
/// to the framework bar so `tui_pane::render_status_line` output uses
/// cargo-port's key/label styling. The framework ships a theme-neutral
/// [`tui_pane::BarPalette::default`]; cargo-port supplies its own colors
/// here.
pub(super) fn cargo_port_bar_palette() -> BarPalette {
    let enabled_key_style = Style::default()
        .fg(ACCENT_COLOR)
        .add_modifier(Modifier::BOLD);
    let disabled_key_style = Style::default()
        .fg(SECONDARY_TEXT_COLOR)
        .add_modifier(Modifier::BOLD);
    let disabled_label_style = Style::default().fg(SECONDARY_TEXT_COLOR);
    BarPalette {
        status_line_style: Style::default().bg(STATUS_BAR_COLOR).fg(Color::White),
        status_activity_style: enabled_key_style,
        status_label_style: Style::default()
            .fg(TITLE_COLOR)
            .add_modifier(Modifier::BOLD),
        status_value_style: Style::default().fg(Color::White),
        enabled_key_style,
        enabled_label_style: Style::default(),
        disabled_key_style,
        disabled_label_style,
        separator_style: Style::default(),
    }
}

fn cargo_port_status_line_globals(app: &App) -> [StatusLineGlobal<AppGlobalAction>; 8] {
    let selected_project_is_deleted = app.project_list.selected_project_is_deleted();
    let terminal_command_configured = !app.config.terminal_command().trim().is_empty();
    let editor_state = if selected_project_is_deleted {
        ShortcutState::Disabled
    } else {
        ShortcutState::Enabled
    };
    let terminal_state = if terminal_command_configured && !selected_project_is_deleted {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    };
    [
        StatusLineGlobal::app(AppGlobalAction::Find),
        StatusLineGlobal::app(AppGlobalAction::OpenEditor).with_state(editor_state),
        StatusLineGlobal::app(AppGlobalAction::OpenTerminal).with_state(terminal_state),
        StatusLineGlobal::framework(FrameworkGlobalAction::OpenSettings),
        StatusLineGlobal::framework(FrameworkGlobalAction::OpenKeymap),
        StatusLineGlobal::app(AppGlobalAction::Rescan),
        StatusLineGlobal::framework(FrameworkGlobalAction::Quit),
        StatusLineGlobal::framework(FrameworkGlobalAction::Restart),
    ]
}

#[cfg(test)]
pub(super) fn cargo_port_global_text_for_test(app: &App) -> String {
    let globals = cargo_port_status_line_globals(app);
    tui_pane::status_line_global_spans::<App, AppGlobalAction>(
        &app.framework_keymap,
        &globals,
        &BarPalette::default(),
    )
    .iter()
    .map(|span| span.content.as_ref())
    .collect()
}

#[cfg(test)]
pub(super) fn cargo_port_right_text_for_test(
    app: &App,
    framework_global_spans: &[Span<'static>],
) -> String {
    if framework_global_spans.is_empty() {
        String::new()
    } else {
        cargo_port_global_text_for_test(app)
    }
}

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let globals = cargo_port_status_line_globals(app);
    let status = StatusLine::new(
        app.animation_started.elapsed().as_secs(),
        !app.scan.is_complete(),
        &globals,
    );
    render_framework_status_line::<App, AppGlobalAction>(
        frame,
        area,
        app,
        &app.framework_keymap,
        &app.framework,
        &cargo_port_bar_palette(),
        &status,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use crate::tui::panes;
    use crate::tui::panes::BottomRow;
    use crate::tui::panes::PaneId;

    #[test]
    fn resolved_layout_keeps_cpu_column_fixed() {
        let narrow = panes::resolve_layout(Rect::new(0, 0, 80, 30), 30, 12, BottomRow::Diagnostics);
        let wide = panes::resolve_layout(Rect::new(0, 0, 150, 30), 30, 12, BottomRow::Diagnostics);

        assert_eq!(narrow.area(PaneId::Cpu).width, super::panes::CPU_PANE_WIDTH);
        assert_eq!(wide.area(PaneId::Cpu).width, super::panes::CPU_PANE_WIDTH);
    }

    #[test]
    fn top_row_has_no_dead_space_above_targets() {
        let layout =
            panes::resolve_layout(Rect::new(0, 0, 120, 30), 30, 12, BottomRow::Diagnostics);
        let package = layout.area(PaneId::Package);
        let git = layout.area(PaneId::Git);
        let targets = layout.area(PaneId::Targets);
        let right_col = Rect::new(30, 0, 90, 30);

        assert_eq!(package.x, right_col.x);
        assert_eq!(
            git.x.saturating_add(git.width),
            right_col.x.saturating_add(right_col.width)
        );
        assert_eq!(package.width.saturating_add(git.width), right_col.width);
        assert_eq!(
            targets.x.saturating_add(targets.width),
            right_col.x.saturating_add(right_col.width)
        );
    }

    #[test]
    fn middle_row_expands_to_fit_all_cpu_rows_when_height_allows() {
        let layout =
            panes::resolve_layout(Rect::new(0, 0, 120, 40), 30, 12, BottomRow::Diagnostics);

        assert_eq!(
            layout.area(PaneId::Cpu).height,
            super::panes::cpu_required_pane_height(12)
        );
    }
}
