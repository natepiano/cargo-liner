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
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::app::ConfirmAction;
use super::constants::ACCENT_COLOR;
use super::constants::BLOCK_BORDER_WIDTH;
use super::constants::BYTES_PER_GIB;
use super::constants::BYTES_PER_KIB;
use super::constants::BYTES_PER_MIB;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::ERROR_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SECONDARY_TEXT_COLOR;
use super::constants::STATUS_BAR_COLOR;
use super::constants::SUCCESS_COLOR;
use super::constants::TITLE_COLOR;
use super::finder;
use super::interaction;
use super::keymap_ui;
use super::pane;
use super::pane::PaneRenderCtx;
use super::panes;
use super::panes::DispatchArgs;
use super::panes::LayoutCache;
use super::panes::PaneId;
use super::panes::Panes;
use super::panes::RenderStyles;
use super::popup::PopupFrame;
use super::settings;
use super::shortcuts;
use super::shortcuts::Shortcut;
use super::shortcuts::ShortcutState;
use super::toasts;
use crate::ci::Conclusion;
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

pub(super) fn conclusion_style(conclusion: Option<Conclusion>) -> Style {
    match conclusion {
        Some(Conclusion::Success) => Style::default().fg(SUCCESS_COLOR),
        Some(Conclusion::Failure) => Style::default().fg(ERROR_COLOR),
        _ => Style::default(),
    }
}

pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub(super) fn ui(frame: &mut Frame, app: &mut App) {
    sync_hovered_pane_row(app);
    *app.layout_cache_mut() = LayoutCache::default();
    app.prune_toasts();

    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let left_width = u16::try_from(app.cached_fit_widths().total_width() + BLOCK_BORDER_WIDTH + 1)
        .unwrap_or(u16::MAX);

    let bottom_row = if app.example_output().is_empty() {
        panes::BottomRow::Diagnostics
    } else {
        panes::BottomRow::Output
    };
    let core_count = app
        .panes()
        .cpu()
        .content()
        .map_or(1, |usage| usage.cores.len());
    let tiled = panes::resolve_layout(outer_layout[0], left_width, core_count, bottom_row);

    for resolved in tiled.panes() {
        render_tiled_pane(frame, app, resolved.pane, resolved.area);
    }
    app.layout_cache_mut().tiled = tiled;

    render_status_bar(frame, app, outer_layout[1]);
    let toast_result = toasts::render_toasts(
        frame,
        outer_layout[0],
        &app.toasts().active_now(),
        app.focus().is(PaneId::Toasts),
        app.focused_toast_id(),
    );
    app.toasts_mut().set_hits(toast_result.hitboxes);

    if app.overlays().is_settings_open() {
        settings::render_settings_popup(frame, app);
    }
    if app.overlays().is_keymap_open() {
        keymap_ui::render_keymap_popup(frame, app);
    }
    if app.overlays().is_finder_open() {
        finder::render_finder_popup(frame, app);
    }
    if let Some(action) = app.confirm() {
        let body = confirm_action_body(app, action);
        let verifying = app.scan().confirm_verifying().is_some();
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
                .scan()
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
                    .scan()
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
    let siblings = app.scan().target_dir_index().siblings(target, selection);
    let project_siblings: Vec<&AbsolutePath> =
        siblings.iter().map(|member| &member.project_root).collect();
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
    // Step 6e: while the fingerprint re-check is in flight we swap
    // the prompt + keys for a "Verifying target dir…" placeholder
    // and drop the (y/n) suffix — `y` is ignored by handle_confirm_key
    // in that state, and showing it enabled would lie to the user.
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

fn render_left_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    panes::render_project_list(frame, app, area);
}

fn pane_render_styles() -> RenderStyles {
    RenderStyles {
        readonly_label: Style::default().fg(LABEL_COLOR),
        chrome:         pane::default_pane_chrome(),
    }
}

fn dispatch_via_trait(
    app: &mut App,
    area: Rect,
    id: PaneId,
    frame: &mut Frame,
    dispatcher: fn(&mut Panes, &mut Frame, Rect, &DispatchArgs<'_>),
) {
    let focus_state = app.focus().pane_state(id);
    let is_focused = app.focus().is(id);
    let animation_elapsed = app.animation_elapsed();
    // Compute `selected_project_path` before the split-borrow — it
    // crosses Selection + Scan via `path_for_row`, so the
    // dispatcher's typed refs alone can't reproduce it.
    let selected_project_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let (panes, _layout_cache, config, projects) = app.split_panes_for_render();
    let args = DispatchArgs {
        focus_state,
        is_focused,
        animation_elapsed,
        config,
        project_list: projects,
        selected_project_path: selected_project_path.as_deref(),
    };
    dispatcher(panes, frame, area, &args);
}

fn render_lints_pane(app: &mut App, frame: &mut Frame, area: Rect) {
    let focus_state = app.focus().pane_state(PaneId::Lints);
    let is_focused = app.focus().is(PaneId::Lints);
    let animation_elapsed = app.animation_elapsed();
    let selected_project_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let (lint, config, projects) = app.split_lint_for_render();
    let ctx = PaneRenderCtx {
        focus_state,
        is_focused,
        animation_elapsed,
        config,
        project_list: projects,
        selected_project_path: selected_project_path.as_deref(),
    };
    panes::render_lints_pane_body(frame, area, lint, &ctx);
}

fn render_ci_pane(app: &mut App, frame: &mut Frame, area: Rect) {
    let focus_state = app.focus().pane_state(PaneId::CiRuns);
    let is_focused = app.focus().is(PaneId::CiRuns);
    let animation_elapsed = app.animation_elapsed();
    let selected_project_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let (ci, config, projects) = app.split_ci_for_render();
    let ctx = PaneRenderCtx {
        focus_state,
        is_focused,
        animation_elapsed,
        config,
        project_list: projects,
        selected_project_path: selected_project_path.as_deref(),
    };
    panes::render_ci_pane_body(frame, area, ci, &ctx);
}

fn render_tiled_pane(frame: &mut Frame, app: &mut App, pane: PaneId, area: Rect) {
    match pane {
        PaneId::ProjectList => render_left_panel(frame, app, area),
        PaneId::Package => dispatch_via_trait(
            app,
            area,
            PaneId::Package,
            frame,
            panes::Panes::dispatch_package_render,
        ),
        PaneId::Git => dispatch_via_trait(
            app,
            area,
            PaneId::Git,
            frame,
            panes::Panes::dispatch_git_render,
        ),
        PaneId::Lang => dispatch_via_trait(
            app,
            area,
            PaneId::Lang,
            frame,
            panes::Panes::dispatch_lang_render,
        ),
        PaneId::Cpu => dispatch_via_trait(
            app,
            area,
            PaneId::Cpu,
            frame,
            panes::Panes::dispatch_cpu_render,
        ),
        PaneId::Targets => {
            if let Some(targets_data) = app.panes().targets().content().cloned()
                && targets_data.has_targets()
            {
                panes::render_targets_panel(frame, app, &targets_data, &pane_render_styles(), area);
            } else {
                panes::render_empty_targets_panel(frame, app, area);
            }
        },
        PaneId::Lints => render_lints_pane(app, frame, area),
        PaneId::CiRuns => render_ci_pane(app, frame, area),
        PaneId::Output => panes::render_output_panel(frame, app, area),
        PaneId::Toasts | PaneId::Settings | PaneId::Finder | PaneId::Keymap => {},
    }
}

fn sync_hovered_pane_row(app: &mut App) {
    let hovered = app
        .mouse_pos()
        .and_then(|pos| interaction::hovered_pane_row_at(app, pos));
    app.set_hovered_pane_row(hovered);
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

fn shortcut_spans(shortcuts: &[Shortcut]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for shortcut in shortcuts {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        let (key_style, description_style) = match shortcut.state {
            ShortcutState::Enabled => (
                Style::default()
                    .fg(ACCENT_COLOR)
                    .add_modifier(Modifier::BOLD),
                Style::default(),
            ),
            ShortcutState::Disabled => (
                Style::default()
                    .fg(SECONDARY_TEXT_COLOR)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(SECONDARY_TEXT_COLOR),
            ),
        };
        spans.push(Span::styled(format!(" {}", shortcut.key), key_style));
        spans.push(Span::styled(
            format!(" {}", shortcut.description),
            description_style,
        ));
    }
    spans
}

fn shortcut_display_width(shortcuts: &[Shortcut]) -> usize {
    if shortcuts.is_empty() {
        return 0;
    }
    let content: usize = shortcuts
        .iter()
        .map(|s| 1 + s.key.len() + 1 + s.description.len())
        .sum();
    // separators between items (2 chars each, count - 1 gaps)
    content + (shortcuts.len() - 1) * 2
}

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let bar_style = Style::default().bg(STATUS_BAR_COLOR).fg(Color::White);

    // Fill the entire bar with the background color
    frame.render_widget(Paragraph::new("").style(bar_style), area);

    let context = app.input_context();
    let enter_action = app.enter_action();
    // Clean shortcut uses `clean_selection` (design plan → gating
    // fix): the flag is whether *some* clean is possible from the
    // current row, not whether the Root item is Rust — that old
    // heuristic disabled Clean on WorktreeEntry rows.
    let clean_enabled = app.clean_selection().is_some();
    let clear_lint_action = app
        .selected_project_path()
        .and_then(|path| app.lint_at_path(path))
        .filter(|lr| !lr.runs().is_empty())
        .map(|_| "clear cache");
    let groups = shortcuts::for_status_bar(
        context,
        enter_action,
        clean_enabled,
        clear_lint_action,
        app.keymap().current(),
        app.config().terminal_command_configured(),
        app.selected_project_is_deleted(),
    );

    let mut left_spans = Vec::new();
    if !app.scan().is_complete() {
        let key_style = Style::default()
            .fg(ACCENT_COLOR)
            .add_modifier(Modifier::BOLD);
        left_spans.push(Span::styled(" ⟳ scanning… ", key_style));
    }
    let uptime_secs = app.animation_elapsed().as_secs();
    let uptime_label_style = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);
    let uptime_value_style = Style::default().fg(Color::White);
    left_spans.push(Span::styled(" Uptime: ", uptime_label_style));
    left_spans.push(Span::styled(
        format!("{} ", super::duration_fmt::format_progressive(uptime_secs)),
        uptime_value_style,
    ));
    left_spans.extend(shortcut_spans(&groups.navigation));

    let center_spans = shortcut_spans(&groups.actions);
    let right_spans = shortcut_spans(&groups.global);

    let total_width = area.width as usize;
    let left_width = left_spans.iter().map(Span::width).sum::<usize>();
    let center_width = shortcut_display_width(&groups.actions);
    let right_width = shortcut_display_width(&groups.global);

    // Left section
    if !left_spans.is_empty() {
        let left_area = Rect {
            x:      area.x,
            y:      area.y,
            width:  area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(left_spans)).style(bar_style),
            left_area,
        );
    }

    // Center section
    if !center_spans.is_empty() {
        let center_start = total_width.saturating_sub(center_width) / 2;
        // Only render if it doesn't overlap with the left section
        if center_start >= left_width {
            let center_area = Rect {
                x:      area.x + u16::try_from(center_start).unwrap_or(u16::MAX),
                y:      area.y,
                width:  u16::try_from((total_width - center_start).min(center_width + 1))
                    .unwrap_or(u16::MAX),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(center_spans)).style(bar_style),
                center_area,
            );
        }
    }

    // Right section
    if !right_spans.is_empty() {
        let right_start = total_width.saturating_sub(right_width + 1);
        let right_area = Rect {
            x:      area.x + u16::try_from(right_start).unwrap_or(u16::MAX),
            y:      area.y,
            width:  u16::try_from(right_width + 1).unwrap_or(u16::MAX),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(right_spans)).style(bar_style),
            right_area,
        );
    }
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
