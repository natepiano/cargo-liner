use std::collections::HashMap;
use std::path::Path;

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
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::app::ConfirmAction;
use super::app::DiscoveryRowKind;
use super::app::ExpandKey;
use super::app::ResolvedWidths;
use super::app::VisibleRow;
use super::constants::BLOCK_BORDER_WIDTH;
use super::constants::BYTES_PER_GIB;
use super::constants::BYTES_PER_KIB;
use super::constants::BYTES_PER_MIB;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::DETAIL_PANEL_HEIGHT;
use super::constants::SEARCH_BAR_HEIGHT;
use super::detail::DetailInfo;
use super::interaction::UiSurface::Content;
use super::shortcuts::Shortcut;
use super::shortcuts::ShortcutState;
use super::types::ACTIVE_FOCUS_COLOR;
use super::types::LayoutCache;
use super::types::Pane;
use super::types::PaneId;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::lint::LintRun;
use crate::project;
use crate::project::GitOrigin;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::scan;

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

// ── Row prefix strings ───────────────────────────────────────────────
// Single source of truth: width calc and render both reference these.

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼ ";
pub(super) const PREFIX_ROOT_COLLAPSED: &str = "▶ ";
pub(super) const PREFIX_ROOT_LEAF: &str = "  ";
pub(super) const PREFIX_MEMBER_INLINE: &str = "    ";
pub(super) const PREFIX_MEMBER_NAMED: &str = "        ";
pub(super) const PREFIX_VENDORED: &str = "    ";
pub(super) const PREFIX_GROUP_EXPANDED: &str = "    ▼ ";
pub(super) const PREFIX_GROUP_COLLAPSED: &str = "    ▶ ";
pub(super) const PREFIX_WT_EXPANDED: &str = "    ▼ ";
pub(super) const PREFIX_WT_COLLAPSED: &str = "    ▶ ";
pub(super) const PREFIX_WT_FLAT: &str = "    ";
pub(super) const PREFIX_WT_GROUP_EXPANDED: &str = "        ▼ ";
pub(super) const PREFIX_WT_GROUP_COLLAPSED: &str = "        ▶ ";
pub(super) const PREFIX_WT_MEMBER_INLINE: &str = "        ";
pub(super) const PREFIX_WT_MEMBER_NAMED: &str = "            ";
pub(super) const PREFIX_WT_VENDORED: &str = "        ";

pub(super) fn conclusion_style(conclusion: Option<Conclusion>) -> Style {
    match conclusion {
        Some(Conclusion::Success) => Style::default().fg(Color::Green),
        Some(Conclusion::Failure) => Style::default().fg(Color::Red),
        _ => Style::default(),
    }
}

pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Compute a color for a disk value: green (smallest) → white (middle) → red (largest).
#[allow(
    clippy::cast_precision_loss,
    reason = "display-only — index-to-float ratio for color interpolation"
)]
/// Compute the percentile rank of `bytes` within `sorted_values` (0.0 to 1.0).
pub(super) fn disk_percentile(bytes: Option<u64>, sorted_values: &[u64]) -> Option<f64> {
    let bytes = bytes?;
    if sorted_values.len() <= 1 {
        return None;
    }
    let rank = sorted_values
        .iter()
        .position(|&v| v >= bytes)
        .unwrap_or(sorted_values.len() - 1);
    Some(rank as f64 / (sorted_values.len() - 1) as f64)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0.0..=255.0 before cast"
)]
pub(super) fn disk_color(percentile: Option<f64>) -> Style {
    let Some(pos) = percentile else {
        return Style::default().fg(Color::DarkGray);
    };

    // Green (0.0) → White (0.5) → Red (1.0)
    let (r, g, b) = if pos < 0.5 {
        // Green to white: increase R and B
        let t = pos * 2.0;
        (
            155.0f64.mul_add(t, 100.0).clamp(0.0, 255.0) as u8,
            35.0f64.mul_add(t, 220.0).clamp(0.0, 255.0) as u8,
            155.0f64.mul_add(t, 100.0).clamp(0.0, 255.0) as u8,
        )
    } else {
        // White to red: decrease G and B
        let t = (pos - 0.5) * 2.0;
        let gb = 155.0f64.mul_add(-t, 255.0).clamp(0.0, 255.0) as u8;
        (255, gb, gb)
    };

    Style::default().fg(Color::Rgb(r, g, b))
}

pub(super) fn ui(frame: &mut Frame, app: &mut App) {
    *app.layout_cache_mut() = LayoutCache::default();
    app.prune_toasts();

    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let left_width = u16::try_from(app.cached_fit_widths().total_width() + BLOCK_BORDER_WIDTH)
        .unwrap_or(u16::MAX);

    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(outer_layout[0]);

    render_left_panel(frame, app, main_layout[0]);
    render_right_panel(frame, app, main_layout[1]);
    render_status_bar(frame, app, outer_layout[1]);
    let toast_result = super::toasts::render_toasts(
        frame,
        outer_layout[0],
        &app.active_toasts(),
        app.is_focused(PaneId::Toasts),
        app.focused_toast_id(),
    );
    super::interaction::register_toast_hitboxes(app, &toast_result.hitboxes);

    if app.is_settings_open() {
        super::settings::render_settings_popup(frame, app);
    }
    if app.is_keymap_open() {
        super::keymap_ui::render_keymap_popup(frame, app);
    }
    if app.is_finder_open() {
        super::finder::render_finder_popup(frame, app);
    }
    if let Some(action) = app.confirm() {
        render_confirm_popup(frame, action);
    }
}

fn render_confirm_popup(frame: &mut Frame, action: &ConfirmAction) {
    let prompt = match action {
        ConfirmAction::Clean(_) => "Run cargo clean?",
    };

    let text = format!(" {prompt}  (y/n) ");
    let width = u16::try_from(text.len() + 4).unwrap_or(u16::MAX);

    let inner = super::popup::PopupFrame {
        title: None,
        border_color: Color::Yellow,
        width,
        height: CONFIRM_DIALOG_HEIGHT,
    }
    .render(frame);

    let line = Line::from(vec![
        Span::styled(format!(" {prompt}  "), Style::default().fg(Color::White)),
        Span::styled(
            "(y/n)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

fn render_left_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let search_height = if app.is_searching() {
        SEARCH_BAR_HEIGHT
    } else {
        0
    };
    let left_constraints = vec![Constraint::Length(search_height), Constraint::Min(1)];
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(left_constraints)
        .split(area);

    if app.is_searching() {
        render_search_bar(frame, app, left_layout[0]);
    }

    render_project_list(frame, app, left_layout[1]);
}

fn render_right_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.render_widget(Clear, area);

    let detail_info = app.cached_detail().map(|c| c.info.clone());
    let selected_ci_state = app.selected_ci_state();
    let selected_has_ci_owner = app.selected_ci_path().is_some();
    let has_workflows = app
        .selected_project_path()
        .and_then(|path| app.git_info_for(path))
        .is_some_and(|g| g.workflows.is_present());
    let has_ci = selected_ci_state.is_some() && has_workflows;
    let detail_lint_runs = app
        .selected_project_path()
        .and_then(|path| app.lint_runs().get(path))
        .cloned()
        .unwrap_or_default();
    let detail_ci_runs: Vec<CiRun> = app.selected_ci_runs();
    let has_example_output = !app.example_output().is_empty();

    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(DETAIL_PANEL_HEIGHT),
            Constraint::Min(3),
        ])
        .split(area);

    super::detail::render_detail_panel(frame, app, detail_info.as_ref(), right_layout[0]);
    sync_detail_pane_hitboxes(app, detail_info.as_ref());

    // Running output replaces the bottom panels; Esc restores them.
    if has_example_output {
        render_example_output(frame, app, right_layout[1]);
    } else {
        render_bottom_right_panel(
            frame,
            app,
            &detail_lint_runs,
            &detail_ci_runs,
            right_layout[1],
            has_ci,
            selected_has_ci_owner,
        );
    }
}

fn sync_detail_pane_hitboxes(app: &mut App, detail_info: Option<&DetailInfo>) {
    if detail_info.is_some() {
        register_detail_pane_hitboxes(app);
        return;
    }

    reset_pane(app.package_pane_mut());
    reset_pane(app.git_pane_mut());
    reset_pane(app.targets_pane_mut());
}

fn register_detail_pane_hitboxes(app: &mut App) {
    let package_pane = app.package_pane().clone();
    super::interaction::register_pane_row_hitboxes(app, PaneId::Package, &package_pane, Content);

    if app
        .selected_project_path()
        .and_then(|path| app.git_info_for(path))
        .is_some()
    {
        let git_pane = app.git_pane().clone();
        super::interaction::register_pane_row_hitboxes(app, PaneId::Git, &git_pane, Content);
    } else {
        reset_pane(app.git_pane_mut());
    }

    if app.cached_detail().is_some_and(|cached| {
        cached.info.is_binary || !cached.info.examples.is_empty() || !cached.info.benches.is_empty()
    }) {
        let targets_pane = app.targets_pane().clone();
        super::interaction::register_pane_row_hitboxes(
            app,
            PaneId::Targets,
            &targets_pane,
            Content,
        );
    } else {
        reset_pane(app.targets_pane_mut());
    }
}

fn render_bottom_right_panel(
    frame: &mut Frame,
    app: &mut App,
    detail_lint_runs: &[LintRun],
    detail_ci_runs: &[CiRun],
    area: Rect,
    has_ci: bool,
    selected_has_ci_owner: bool,
) {
    let bottom_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    super::detail::render_lints_panel(frame, app, detail_lint_runs, bottom_split[0]);

    if has_ci {
        super::detail::render_ci_panel(frame, app, detail_ci_runs, bottom_split[1]);
    } else {
        render_empty_ci_panel(
            frame,
            app,
            app.selected_project_path(),
            selected_has_ci_owner,
            bottom_split[1],
        );
        reset_pane(app.ci_pane_mut());
    }

    if let Some(message) = app.unreachable_service_message() {
        render_unreachable_overlay(frame, bottom_split[1], &message);
    }
}

const fn reset_pane(pane: &mut Pane) {
    pane.set_len(0);
    pane.set_content_area(Rect::ZERO);
    pane.set_scroll_offset(0);
}

fn render_unreachable_overlay(frame: &mut Frame, area: Rect, msg: &str) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    let width = u16::try_from(msg.len() + 4).unwrap_or(u16::MAX);
    let overlay_area = centered_rect(width, 3, area);
    frame.render_widget(Clear, overlay_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )))
        .alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

fn render_empty_ci_panel(
    frame: &mut Frame,
    app: &App,
    project_path: Option<&Path>,
    selected_has_ci_owner: bool,
    area: Rect,
) {
    let has_git = project_path.is_some_and(|path| app.git_info_for(path).is_some());
    let has_url = project_path
        .filter(|_| selected_has_ci_owner)
        .and_then(|path| app.git_info_for(path))
        .is_some_and(|g| g.url.is_some());
    let is_local = project_path
        .filter(|_| selected_has_ci_owner)
        .and_then(|path| app.git_info_for(path))
        .is_some_and(|g| g.origin == GitOrigin::Local);
    let has_workflows = project_path
        .and_then(|path| app.git_info_for(path))
        .is_some_and(|g| g.workflows.is_present());

    let title = if project_path.is_some() && !selected_has_ci_owner {
        " CI Runs — shown on branch/worktree rows "
    } else if !has_git {
        " CI Runs — not a git repository "
    } else if is_local || !has_url {
        " CI Runs — requires a GitHub origin remote "
    } else if !has_workflows {
        " CI Runs — no .yml or .yaml in .github/workflows/ "
    } else if !app.is_scan_complete() {
        " CI Runs — loading… "
    } else {
        " No CI Runs "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(Color::DarkGray))
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(block, area);
}

pub(super) fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let search_focused = app.is_focused(PaneId::Search);
    let search_style = if search_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let search_text = if search_focused {
        if app.search_query().is_empty() {
            "…".to_string()
        } else {
            app.search_query().to_string()
        }
    } else {
        "/ to search".to_string()
    };

    let search_bar = Paragraph::new(Line::from(vec![
        Span::styled(" 🔍 ", Style::default().fg(Color::Yellow)),
        Span::styled(search_text, search_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if search_focused {
                Style::default().fg(ACTIVE_FOCUS_COLOR)
            } else {
                Style::default().fg(Color::DarkGray)
            }),
    );

    frame.render_widget(search_bar, area);
}

pub(super) fn render_project_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let (mut items, header, summary_line, row_width) = {
        let widths = app.cached_fit_widths();
        let items: Vec<ListItem> = if app.is_searching() && !app.search_query().is_empty() {
            render_filtered_items(app, widths)
        } else {
            render_tree_items(app, widths)
        };
        let total_str = format_bytes(
            app.projects()
                .iter()
                .filter_map(RootItem::disk_usage_bytes)
                .sum(),
        );
        let header = super::columns::header_line(widths, "Projects");
        let summary = super::columns::build_summary_cells(widths, &total_str);
        let summary_line = Some(super::columns::row_to_line(&summary, widths));
        let row_width = u16::try_from(widths.total_width()).unwrap_or(u16::MAX);
        (items, header, summary_line, row_width)
    };

    let total_project_rows = items.len();

    let title = project_panel_title(app, area.width.saturating_sub(2).into());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.is_focused(PaneId::ProjectList) {
            Style::default().fg(ACTIVE_FOCUS_COLOR)
        } else {
            Style::default()
        })
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        app.layout_cache_mut().project_list = Rect::ZERO;
        return;
    }

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::DarkGray)),
        header_area,
    );

    let content_area = if inner.height > 1 {
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1)
    } else {
        Rect::new(inner.x, inner.y, inner.width, 0)
    };
    if content_area.height == 0 {
        app.layout_cache_mut().project_list = Rect::ZERO;
        return;
    }

    let pin_summary = should_pin_project_summary(
        total_project_rows,
        summary_line.is_some(),
        content_area.height,
    );

    if !pin_summary && let Some(ref line) = summary_line {
        items.push(ListItem::new(line.clone()));
    }

    let list_area = if pin_summary && content_area.height > 1 {
        Rect::new(
            content_area.x,
            content_area.y,
            content_area.width,
            content_area.height - 1,
        )
    } else {
        content_area
    };
    let project_list_focus = app.pane_focus_state(PaneId::ProjectList);
    let project_list = List::new(items).highlight_style(Pane::selection_style(project_list_focus));

    frame.render_stateful_widget(project_list, list_area, app.list_state_mut());
    app.layout_cache_mut().project_list = list_area;
    app.layout_cache_mut().project_list_offset = app.list_state().offset();
    super::interaction::register_project_list_hitboxes(app, list_area, row_width);

    if pin_summary && let Some(line) = summary_line {
        let footer_area = Rect::new(
            content_area.x,
            content_area.y + content_area.height.saturating_sub(1),
            content_area.width,
            1,
        );
        frame.render_widget(Paragraph::new(line), footer_area);
    }
}

fn project_panel_title(app: &App, max_width: usize) -> String {
    let prefix = "roots: ";
    if max_width <= prefix.width() {
        return truncate_to_width(prefix, max_width);
    }
    let roots = scan::resolve_include_dirs(app.scan_root(), &app.current_config().tui.include_dirs)
        .into_iter()
        .map(|path| project::home_relative_path(path.as_path()))
        .collect::<Vec<_>>();
    format!(
        "{prefix}{}",
        truncate_root_title(&roots, max_width.saturating_sub(prefix.width()))
    )
}

fn truncate_root_title(roots: &[String], max_width: usize) -> String {
    if roots.is_empty() || max_width == 0 {
        return String::new();
    }

    let ellipsis = "…";
    let mut title = String::new();
    for (index, root) in roots.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        let candidate = format!("{title}{separator}{root}");
        if candidate.width() <= max_width {
            title = candidate;
            continue;
        }
        if title.is_empty() {
            return truncate_with_ellipsis(root, max_width, ellipsis);
        }
        let remaining = max_width.saturating_sub(title.width() + separator.width());
        let truncated = truncate_with_ellipsis(root, remaining, ellipsis);
        if !truncated.is_empty() {
            return format!("{title}{separator}{truncated}");
        }
        let with_ellipsis = format!("{title}{separator}{ellipsis}");
        if with_ellipsis.width() <= max_width {
            return with_ellipsis;
        }
        return truncate_with_ellipsis(&title, max_width, ellipsis);
    }
    title
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

fn should_pin_project_summary(project_rows: usize, has_summary: bool, inner_height: u16) -> bool {
    has_summary && project_rows.saturating_add(1) > usize::from(inner_height)
}

fn render_example_output(frame: &mut Frame, app: &App, area: Rect) {
    let title = app.example_running().map_or_else(
        || " Output (Esc to close) ".to_string(),
        |n| format!(" Running: {n} "),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if app.example_running().is_some() {
            Style::default().fg(ACTIVE_FOCUS_COLOR)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let lines: Vec<Line> = app
        .example_output()
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                format!(" {l}"),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let inner_height = area.height.saturating_sub(2);
    let total_lines = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let scroll_offset = total_lines.saturating_sub(inner_height);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);
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
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                Style::default(),
            ),
            ShortcutState::Disabled => (
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(Color::Gray),
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
    let bar_style = Style::default().bg(Color::DarkGray).fg(Color::White);

    // Fill the entire bar with the background color
    frame.render_widget(Paragraph::new("").style(bar_style), area);

    let context = app.input_context();
    let enter_action = app.enter_action();
    let is_rust = app.selected_item().is_some_and(RootItem::is_rust);
    let groups = super::shortcuts::for_status_bar(
        context,
        enter_action,
        is_rust,
        app.current_keymap(),
        app.terminal_command_configured(),
    );

    let mut left_spans = Vec::new();
    if !app.is_scan_complete() {
        let key_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        left_spans.push(Span::styled(" ⟳ scanning… ", key_style));
    }
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

/// Build a `ListItem` for a root-level project node.
fn render_root_item(
    app: &App,
    node_index: usize,
    root_labels: &[String],
    root_sorted: &[u64],
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let name = &root_labels[node_index];
    let disk = App::formatted_disk_for_item(item);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
    let ci = app.ci_for_item(item);
    let lang = item.lang_icon();
    let lint = app.lint_icon_for_root(node_index);
    let origin_sync = app.git_sync(item.path());
    let main_sync = app.git_main(item.path());
    let git_path_state = app.git_path_state_for(item.path());
    let prefix = if item.has_children() {
        if app.expanded().contains(&ExpandKey::Node(node_index)) {
            PREFIX_ROOT_EXPANDED
        } else {
            PREFIX_ROOT_COLLAPSED
        }
    } else {
        PREFIX_ROOT_LEAF
    };
    let deleted = app.is_deleted(item.path());
    let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
        (
            "0.0",
            Some(" [x]"),
            Some(Style::default().fg(Color::DarkGray)),
        )
    } else {
        (disk.as_str(), None, None)
    };
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
            item.path(),
            name,
            git_path_state,
            DiscoveryRowKind::Root,
        ),
        git_path_state,
        lint_icon: lint,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
}

/// Build a `ListItem` for a child project (workspace member or worktree).
fn render_child_item(
    app: &App,
    project: &project::RustProject<project::Package>,
    name: &str,
    child_sorted: &[u64],
    prefix: &'static str,
    inherited_deleted: bool,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let path = project.path();
    let disk = app.formatted_disk(path);
    let disk_bytes = project.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, child_sorted));
    let lang = project::RustProject::<project::Package>::lang_icon();
    let cargo_active = app.is_cargo_active_path(path);
    let lint = if cargo_active {
        app.lint_icon(path)
    } else {
        " "
    };
    let ci = if cargo_active { app.ci_for(path) } else { None };
    let hide_git_status = app.is_workspace_member_path(path);
    let origin_sync = if hide_git_status
        || matches!(
            app.git_path_state_for(path),
            crate::project::GitPathState::Untracked | crate::project::GitPathState::Ignored
        ) {
        String::new()
    } else {
        app.git_sync(path)
    };
    let main_sync = if hide_git_status
        || matches!(
            app.git_path_state_for(path),
            crate::project::GitPathState::Untracked | crate::project::GitPathState::Ignored
        ) {
        String::new()
    } else {
        app.git_main(path)
    };
    let deleted = inherited_deleted || app.is_deleted(project.path());
    let git_path_state = app.git_path_state_for(path);
    let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
        (
            "0.0",
            Some(" [x]"),
            Some(Style::default().fg(Color::DarkGray)),
        )
    } else {
        (disk.as_str(), None, None)
    };
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
            path,
            name,
            git_path_state,
            DiscoveryRowKind::PathOnly,
        ),
        git_path_state,
        lint_icon: lint,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
}

fn render_worktree_entry<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'a> {
    let item = &app.projects()[ni];
    let display_path = app.display_path_for_row(VisibleRow::WorktreeEntry {
        node_index:     ni,
        worktree_index: wi,
    });
    let dp = display_path.unwrap_or_default().to_string();
    let abs_path = app.abs_path_for_row(VisibleRow::WorktreeEntry {
        node_index:     ni,
        worktree_index: wi,
    });
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let wt_name = match item {
        crate::project::RootItem::WorkspaceWorktrees(wtg) => {
            let ws = if wi == 0 {
                wtg.primary()
            } else {
                wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
            };
            ws.worktree_name()
                .map_or_else(|| ws.display_name(), str::to_string)
        },
        crate::project::RootItem::PackageWorktrees(wtg) => {
            let pkg = if wi == 0 {
                wtg.primary()
            } else {
                wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
            };
            pkg.worktree_name()
                .map_or_else(|| pkg.display_name(), str::to_string)
        },
        _ => dp,
    };

    let has_expandable_children = match item {
        crate::project::RootItem::WorkspaceWorktrees(wtg) => {
            let ws = if wi == 0 {
                wtg.primary()
            } else {
                wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
            };
            ws.has_members()
        },
        _ => false,
    };

    let prefix = if has_expandable_children {
        if app.expanded().contains(&ExpandKey::Worktree(ni, wi)) {
            PREFIX_WT_EXPANDED
        } else {
            PREFIX_WT_COLLAPSED
        }
    } else {
        PREFIX_WT_FLAT
    };
    let wt_abs = abs_path.as_deref().unwrap_or_else(|| Path::new(""));
    let disk = app.formatted_disk(wt_abs);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, sorted));
    let lang = item.lang_icon();
    let lint = app.lint_icon_for_worktree(ni, wi);
    let ci = app.ci_for(wt_abs);
    let origin_sync = app.git_sync(wt_abs);
    let main_sync = app.git_main(wt_abs);
    let deleted = app.is_deleted(wt_abs);
    let git_path_state = app.git_path_state_for(wt_abs);
    let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
        (
            "0.0",
            Some(" [x]"),
            Some(Style::default().fg(Color::DarkGray)),
        )
    } else {
        (disk.as_str(), None, None)
    };
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix,
        name: &wt_name,
        name_segments: app.discovery_name_segments_for_path(
            wt_abs,
            &wt_name,
            git_path_state,
            DiscoveryRowKind::WorktreeEntry,
        ),
        git_path_state,
        lint_icon: lint,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
}

fn render_wt_group_header<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    widths: &ResolvedWidths,
) -> ListItem<'a> {
    let item = &app.projects()[ni];
    let (group_name, member_count) = match item {
        crate::project::RootItem::WorkspaceWorktrees(wtg) => {
            let ws = if wi == 0 {
                wtg.primary()
            } else {
                wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
            };
            let group = &ws.groups()[gi];
            (group.group_name().to_string(), group.members().len())
        },
        _ => (String::new(), 0),
    };
    let prefix = if app
        .expanded()
        .contains(&ExpandKey::WorktreeGroup(ni, wi, gi))
    {
        PREFIX_WT_GROUP_EXPANDED
    } else {
        PREFIX_WT_GROUP_COLLAPSED
    };
    let label = format!("{group_name} ({member_count})");
    let row = super::columns::build_group_header_cells(prefix, &label);
    ListItem::new(super::columns::row_to_line(&row, widths))
}

fn render_wt_member<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    mi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'a> {
    let item = &app.projects()[ni];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (member, member_name, is_named_group) = match item {
        crate::project::RootItem::WorkspaceWorktrees(wtg) => {
            let ws = if wi == 0 {
                wtg.primary()
            } else {
                wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
            };
            let group = &ws.groups()[gi];
            let m = &group.members()[mi];
            (Some(m), m.display_name(), group.is_named())
        },
        _ => (None, String::new(), false),
    };
    let indent = if is_named_group {
        PREFIX_WT_MEMBER_NAMED
    } else {
        PREFIX_WT_MEMBER_INLINE
    };
    member.map_or_else(
        || {
            let row = super::columns::build_group_header_cells(indent, &member_name);
            ListItem::new(super::columns::row_to_line(&row, widths))
        },
        |m| {
            let inherited_deleted = match item {
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    let ws = if wi == 0 {
                        wtg.primary()
                    } else {
                        wtg.linked().get(wi - 1).unwrap_or_else(|| wtg.primary())
                    };
                    app.is_deleted(ws.path())
                },
                _ => false,
            };
            render_child_item(
                app,
                m,
                &member_name,
                sorted,
                indent,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_member_item(
    app: &App,
    node_index: usize,
    group_index: usize,
    member_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (member, member_name, is_named) = match item {
        crate::project::RootItem::Workspace(ws) => {
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.display_name(), group.is_named())
        },
        crate::project::RootItem::WorkspaceWorktrees(wtg) if !wtg.renders_as_group() => {
            let ws = wtg.single_live().unwrap_or_else(|| wtg.primary());
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.display_name(), group.is_named())
        },
        _ => (None, String::new(), false),
    };
    let indent = if is_named {
        PREFIX_MEMBER_NAMED
    } else {
        PREFIX_MEMBER_INLINE
    };
    member.map_or_else(
        || {
            let row = super::columns::build_group_header_cells(indent, &member_name);
            ListItem::new(super::columns::row_to_line(&row, widths))
        },
        |m| {
            let inherited_deleted = app.is_deleted(item.path());
            render_child_item(
                app,
                m,
                &member_name,
                sorted,
                indent,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_vendored_item(
    app: &App,
    node_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (vendored, vendored_display_name) = match item {
        crate::project::RootItem::Workspace(ws) => {
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.display_name())
        },
        crate::project::RootItem::WorkspaceWorktrees(wtg) if !wtg.renders_as_group() => {
            let ws = wtg.single_live().unwrap_or_else(|| wtg.primary());
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.display_name())
        },
        crate::project::RootItem::Package(pkg) => {
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.display_name())
        },
        crate::project::RootItem::PackageWorktrees(wtg) if !wtg.renders_as_group() => {
            let pkg = wtg.single_live().unwrap_or_else(|| wtg.primary());
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.display_name())
        },
        _ => (None, String::new()),
    };
    let name = format!("{vendored_display_name} (vendored)");
    vendored.map_or_else(
        || {
            let row = super::columns::build_group_header_cells(PREFIX_VENDORED, &name);
            ListItem::new(super::columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = app.is_deleted(item.path());
            render_child_item(
                app,
                v,
                &name,
                sorted,
                PREFIX_VENDORED,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_wt_vendored_item(
    app: &App,
    node_index: usize,
    worktree_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let vendored_pkg = match item {
        crate::project::RootItem::WorkspaceWorktrees(wtg) => {
            let ws = if worktree_index == 0 {
                wtg.primary()
            } else {
                wtg.linked()
                    .get(worktree_index - 1)
                    .unwrap_or_else(|| wtg.primary())
            };
            ws.vendored().get(vendored_index)
        },
        crate::project::RootItem::PackageWorktrees(wtg) => {
            let pkg = if worktree_index == 0 {
                wtg.primary()
            } else {
                wtg.linked()
                    .get(worktree_index - 1)
                    .unwrap_or_else(|| wtg.primary())
            };
            pkg.vendored().get(vendored_index)
        },
        _ => None,
    };
    let vendored_display_name =
        vendored_pkg.map_or_else(String::new, crate::project::RustProject::display_name);
    let name = format!("{vendored_display_name} (vendored)");
    vendored_pkg.map_or_else(
        || {
            let row = super::columns::build_group_header_cells(PREFIX_WT_VENDORED, &name);
            ListItem::new(super::columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = match item {
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    let ws = if worktree_index == 0 {
                        wtg.primary()
                    } else {
                        wtg.linked()
                            .get(worktree_index - 1)
                            .unwrap_or_else(|| wtg.primary())
                    };
                    app.is_deleted(ws.path())
                },
                crate::project::RootItem::PackageWorktrees(wtg) => {
                    let pkg = if worktree_index == 0 {
                        wtg.primary()
                    } else {
                        wtg.linked()
                            .get(worktree_index - 1)
                            .unwrap_or_else(|| wtg.primary())
                    };
                    app.is_deleted(pkg.path())
                },
                _ => false,
            };
            render_child_item(
                app,
                v,
                &name,
                sorted,
                PREFIX_WT_VENDORED,
                inherited_deleted,
                widths,
            )
        },
    )
}

pub(super) fn render_tree_items(app: &App, widths: &ResolvedWidths) -> Vec<ListItem<'static>> {
    let root_sorted = app.cached_root_sorted();
    let child_sorted = app.cached_child_sorted();
    let root_labels = app
        .projects()
        .resolved_root_labels(app.include_non_rust().includes_non_rust());

    let rows = app.visible_rows();
    rows.iter()
        .map(|row| match row {
            VisibleRow::Root { node_index } => {
                render_root_item(app, *node_index, &root_labels, root_sorted, widths)
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                let item = &app.projects()[*node_index];
                let (group_name, member_count) = match item {
                    crate::project::RootItem::Workspace(ws) => {
                        let group = &ws.groups()[*group_index];
                        (group.group_name().to_string(), group.members().len())
                    },
                    _ => (String::new(), 0),
                };
                let prefix = if app
                    .expanded()
                    .contains(&ExpandKey::Group(*node_index, *group_index))
                {
                    PREFIX_GROUP_EXPANDED
                } else {
                    PREFIX_GROUP_COLLAPSED
                };
                let label = format!("{group_name} ({member_count})");
                let row = super::columns::build_group_header_cells(prefix, &label);
                ListItem::new(super::columns::row_to_line(&row, widths))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => render_member_item(
                app,
                *node_index,
                *group_index,
                *member_index,
                child_sorted,
                widths,
            ),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => render_vendored_item(app, *node_index, *vendored_index, child_sorted, widths),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => render_worktree_entry(app, *node_index, *worktree_index, child_sorted, widths),
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => render_wt_group_header(app, *node_index, *worktree_index, *group_index, widths),
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => render_wt_member(
                app,
                *node_index,
                *worktree_index,
                *group_index,
                *member_index,
                child_sorted,
                widths,
            ),
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => render_wt_vendored_item(
                app,
                *node_index,
                *worktree_index,
                *vendored_index,
                child_sorted,
                widths,
            ),
        })
        .collect()
}

pub(super) fn render_filtered_items(app: &App, widths: &ResolvedWidths) -> Vec<ListItem<'static>> {
    let root_sorted = app.cached_root_sorted();
    app.filtered()
        .iter()
        .map(|hit| {
            let abs = hit.abs_path.as_path();
            let metadata = app.projects().find_searchable_by_abs_path(abs);
            let cargo_active = app.is_cargo_active_path(abs);
            let disk_bytes = metadata.as_ref().and_then(|item| item.disk_usage_bytes);
            let disk = format_bytes(disk_bytes.unwrap_or(0));
            let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
            let lang = if metadata.as_ref().is_some_and(|item| item.is_rust) || hit.is_rust {
                crate::project::RustProject::<crate::project::Package>::lang_icon()
            } else {
                "  "
            };
            let lint = if cargo_active {
                app.lint_icon(abs)
            } else {
                " "
            };
            let ci = if cargo_active { app.ci_for(abs) } else { None };
            let hide_git_status = app.is_workspace_member_path(abs);
            let origin_sync = if hide_git_status
                || matches!(
                    app.git_path_state_for(abs),
                    crate::project::GitPathState::Untracked | crate::project::GitPathState::Ignored
                ) {
                String::new()
            } else {
                app.git_sync(abs)
            };
            let main_sync = if hide_git_status
                || matches!(
                    app.git_path_state_for(abs),
                    crate::project::GitPathState::Untracked | crate::project::GitPathState::Ignored
                ) {
                String::new()
            } else {
                app.git_main(abs)
            };
            let deleted = metadata
                .as_ref()
                .is_some_and(|item| matches!(item.visibility, Visibility::Deleted));
            let git_path_state = app.git_path_state_for(abs);
            let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
                (
                    "0.0",
                    Some(" [x]"),
                    Some(Style::default().fg(Color::DarkGray)),
                )
            } else {
                (disk.as_str(), None, None)
            };
            let row = super::columns::build_row_cells(super::columns::ProjectRow {
                prefix: "  ",
                name: &hit.name,
                name_segments: app.discovery_name_segments_for_path(
                    abs,
                    &hit.name,
                    git_path_state,
                    DiscoveryRowKind::Search,
                ),
                git_path_state,
                lint_icon: lint,
                disk: disk_text,
                disk_style: ds,
                disk_suffix,
                disk_suffix_style,
                lang_icon: lang,
                git_origin_sync: &origin_sync,
                git_main: &main_sync,
                ci,
                deleted,
            });
            ListItem::new(super::columns::row_to_line(&row, widths))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::should_pin_project_summary;

    #[test]
    fn project_summary_stays_inline_when_everything_fits() {
        assert!(!should_pin_project_summary(5, true, 6));
    }

    #[test]
    fn project_summary_pins_when_list_overflows() {
        assert!(should_pin_project_summary(6, true, 6));
    }

    #[test]
    fn project_summary_does_not_pin_without_summary_content() {
        assert!(!should_pin_project_summary(100, false, 6));
    }
}
