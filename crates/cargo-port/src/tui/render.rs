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
use super::constants::ACCENT_COLOR;
use super::constants::ACTIVE_BORDER_COLOR;
use super::constants::BLOCK_BORDER_WIDTH;
use super::constants::BYTES_PER_GIB;
use super::constants::BYTES_PER_KIB;
use super::constants::BYTES_PER_MIB;
use super::constants::COLUMN_HEADER_COLOR;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::ERROR_COLOR;
use super::constants::INACTIVE_BORDER_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SEARCH_BAR_HEIGHT;
use super::constants::SECONDARY_TEXT_COLOR;
use super::constants::STATUS_BAR_COLOR;
use super::constants::SUCCESS_COLOR;
use super::constants::TITLE_COLOR;
use super::detail::DetailInfo;
use super::interaction::UiSurface::Content;
use super::shortcuts::Shortcut;
use super::shortcuts::ShortcutState;
use super::types::LayoutCache;
use super::types::Pane;
use super::types::PaneId;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::project;
use crate::project::GitOrigin;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;

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

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼";
pub(super) const PREFIX_ROOT_COLLAPSED: &str = "▶";
pub(super) const PREFIX_ROOT_LEAF: &str = " ";
pub(super) const PREFIX_MEMBER_INLINE: &str = "   ";
pub(super) const PREFIX_MEMBER_NAMED: &str = "       ";
pub(super) const PREFIX_SUBMODULE: &str = "   ";
pub(super) const PREFIX_VENDORED: &str = "   ";
pub(super) const PREFIX_GROUP_EXPANDED: &str = "   ▼";
pub(super) const PREFIX_GROUP_COLLAPSED: &str = "   ▶";
pub(super) const PREFIX_WT_EXPANDED: &str = "   ▼";
pub(super) const PREFIX_WT_COLLAPSED: &str = "   ▶";
pub(super) const PREFIX_WT_FLAT: &str = "   ";
pub(super) const PREFIX_WT_GROUP_EXPANDED: &str = "       ▼";
pub(super) const PREFIX_WT_GROUP_COLLAPSED: &str = "       ▶";
pub(super) const PREFIX_WT_MEMBER_INLINE: &str = "       ";
pub(super) const PREFIX_WT_MEMBER_NAMED: &str = "           ";
pub(super) const PREFIX_WT_VENDORED: &str = "       ";

/// Returns `ACCENT_COLOR` style when lint is running (spinner), default otherwise.
fn lint_style_for(app: &App, path: &std::path::Path) -> Style {
    let is_running = app
        .lint_at_path(path)
        .is_some_and(|lr| matches!(lr.status(), crate::lint::LintStatus::Running(_)));
    if is_running {
        Style::default().fg(ACCENT_COLOR)
    } else {
        Style::default()
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
        return Style::default().fg(LABEL_COLOR);
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

    // Split into 3 rows: top (detail row 1), middle (detail row 2), bottom (lint + CI).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ])
        .split(outer_layout[0]);

    // Row 1: Project List (left, spans row 1+2) | Package + Languages (right)
    // Row 2: (PL continues)                     | Targets + Git (right)
    let top_two_rows = Rect::new(
        rows[0].x,
        rows[0].y,
        rows[0].width,
        rows[0].height + rows[1].height,
    );
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(top_two_rows);

    // Left column (rows 1+2): Project List with search bar
    render_left_panel(frame, app, top_cols[0]);

    // Right column row 1: Package | Languages (detail panel)
    let right_row1 = Rect::new(top_cols[1].x, rows[0].y, top_cols[1].width, rows[0].height);
    // Right column row 2: Targets | Git
    let right_row2 = Rect::new(top_cols[1].x, rows[1].y, top_cols[1].width, rows[1].height);

    render_detail_row1(frame, app, right_row1);
    render_detail_row2(frame, app, right_row2);

    // Register hitboxes after all panes have their content_area set.
    let detail_info = app.cached_detail().map(|c| c.info.clone());
    sync_detail_pane_hitboxes(app, detail_info.as_ref());

    // Row 3: output covers full width, or Lint Runs (PL width) | CI Runs (rest).
    if app.example_output().is_empty() {
        let bottom_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_width), Constraint::Min(20)])
            .split(rows[2]);
        render_bottom_panel(frame, app, bottom_cols[0], bottom_cols[1]);
    } else {
        render_example_output(frame, app, rows[2]);
    }
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
        border_color: TITLE_COLOR,
        width,
        height: CONFIRM_DIALOG_HEIGHT,
    }
    .render(frame);

    let line = Line::from(vec![
        Span::styled(format!(" {prompt}  "), Style::default().fg(Color::White)),
        Span::styled(
            "(y/n)",
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

/// Left column: Project List (spans rows 1+2).
fn render_left_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let search_height = if app.is_searching() {
        SEARCH_BAR_HEIGHT
    } else {
        0
    };
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(search_height), Constraint::Min(1)])
        .split(area);
    if app.is_searching() {
        render_search_bar(frame, app, left_layout[0]);
    }
    render_project_list(frame, app, left_layout[1]);
}

/// Right column row 1: Package Details | Targets.
fn render_detail_row1(frame: &mut Frame, app: &mut App, area: Rect) {
    let detail_info = app.cached_detail().map(|cache| cache.info.clone());
    // render_detail_panel renders Package + Targets (was Package + Lang).
    super::detail::render_detail_panel(frame, app, detail_info.as_ref(), area);
}

/// Right column row 2: Languages | Git.
fn render_detail_row2(frame: &mut Frame, app: &mut App, area: Rect) {
    let detail_info = app.cached_detail().map(|cache| cache.info.clone());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Languages (left half of row 2).
    if let Some(info) = detail_info.as_ref() {
        let title_style = Style::default()
            .fg(TITLE_COLOR)
            .add_modifier(Modifier::BOLD);
        let styles = super::detail::RenderStyles {
            readonly_label:  Style::default().fg(LABEL_COLOR),
            active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
            inactive_border: Style::default(),
            title:           title_style,
        };
        super::detail::render_lang_panel_standalone(frame, app, info, &styles, cols[0]);
    }

    // Git (right half of row 2).
    super::detail::render_git_panel(frame, app, detail_info.as_ref(), cols[1]);
}

/// Bottom row: Lint Runs (left) | CI Runs (right).
fn render_bottom_panel(frame: &mut Frame, app: &mut App, lint_area: Rect, ci_area: Rect) {
    let detail_lint_runs = app
        .selected_project_path()
        .and_then(|path| app.lint_at_path(path))
        .map(|lr| lr.runs().to_vec())
        .unwrap_or_default();
    let detail_ci_runs: Vec<CiRun> = app.selected_ci_runs();

    super::detail::render_lints_panel(frame, app, &detail_lint_runs, lint_area);

    let selected_has_ci_owner = app.selected_ci_path().is_some();
    let has_workflows = app
        .selected_project_path()
        .and_then(|path| app.git_info_for(path))
        .is_some_and(|g| g.workflows.is_present());
    let selected_ci_state = app.selected_ci_state();
    let has_ci = selected_ci_state.is_some() && has_workflows;

    if has_ci {
        super::detail::render_ci_panel(frame, app, &detail_ci_runs, ci_area);
    } else {
        render_empty_ci_panel(
            frame,
            app,
            app.selected_project_path(),
            selected_has_ci_owner,
            ci_area,
        );
    }
}

fn sync_detail_pane_hitboxes(app: &mut App, detail_info: Option<&DetailInfo>) {
    if detail_info.is_some() {
        register_detail_pane_hitboxes(app);
        return;
    }

    reset_pane(&mut app.pane_manager_mut().package);
    reset_pane(&mut app.pane_manager_mut().git);
}

/// Register row hitboxes for panes with row-based content.
fn register_detail_pane_hitboxes(app: &mut App) {
    register_hitbox_for_pane(app, PaneId::Package);
    register_hitbox_for_pane(app, PaneId::Lang);
    register_hitbox_for_pane(app, PaneId::Git);
    register_hitbox_for_pane(app, PaneId::Targets);
}

/// Exhaustive match on `PaneId` — adding a variant forces you to decide
/// whether the pane gets hitboxes.
fn register_hitbox_for_pane(app: &mut App, id: PaneId) {
    match id {
        PaneId::Package | PaneId::Lang => {
            let pane = app.pane_manager().by_id(id).clone();
            super::interaction::register_pane_row_hitboxes(app, id, &pane, Content);
        },
        PaneId::Git => {
            if app
                .selected_project_path()
                .and_then(|path| app.git_info_for(path))
                .is_some()
            {
                let pane = app.pane_manager().git.clone();
                super::interaction::register_pane_row_hitboxes(app, id, &pane, Content);
            } else {
                reset_pane(&mut app.pane_manager_mut().git);
            }
        },
        PaneId::Targets => {
            if app.cached_detail().is_some_and(|cached| {
                cached.info.is_binary
                    || !cached.info.examples.is_empty()
                    || !cached.info.benches.is_empty()
            }) {
                let pane = app.pane_manager().targets.clone();
                super::interaction::register_pane_row_hitboxes(app, id, &pane, Content);
            } else {
                reset_pane(&mut app.pane_manager_mut().targets);
            }
        },
        // These panes register their own hitboxes during rendering
        // or don't have row-based hitboxes.
        PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Output
        | PaneId::Toasts
        | PaneId::Search
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap => {},
    }
}

const fn reset_pane(pane: &mut Pane) {
    pane.set_len(0);
    pane.set_content_area(Rect::ZERO);
    pane.set_scroll_offset(0);
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
        " No CI workflow configured "
    } else if !app.is_scan_complete() {
        " CI Runs — loading… "
    } else {
        " No CI Runs "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR));
    frame.render_widget(block, area);
}

pub(super) fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let search_focused = app.is_focused(PaneId::Search);
    let search_style = if search_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(LABEL_COLOR)
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
        Span::styled(" 🔍 ", Style::default().fg(TITLE_COLOR)),
        Span::styled(search_text, search_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if search_focused {
                Style::default().fg(ACTIVE_BORDER_COLOR)
            } else {
                Style::default().fg(LABEL_COLOR)
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
        let header = super::columns::header_line(widths, " Projects");
        let summary = super::columns::build_summary_cells(widths, &total_str);
        let summary_line = Some(super::columns::row_to_line(&summary, widths));
        let row_width = u16::try_from(widths.total_width()).unwrap_or(u16::MAX);
        (items, header, summary_line, row_width)
    };

    let total_project_rows = items.len();

    let title = project_panel_title_with_counts(app, area.width.saturating_sub(2).into());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.is_focused(PaneId::ProjectList) {
            Style::default().fg(ACTIVE_BORDER_COLOR)
        } else {
            Style::default()
        })
        .title_style(
            Style::default()
                .fg(TITLE_COLOR)
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
        Paragraph::new(header).style(Style::default().fg(COLUMN_HEADER_COLOR)),
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

fn project_panel_title_with_counts(app: &App, max_width: usize) -> String {
    let focused = app.is_focused(PaneId::ProjectList);
    let cursor = app.list_state().selected().unwrap_or(0);
    let roots = app.resolved_dirs();

    // Count visible rows per root directory and determine which root the
    // cursor is in.
    let mut root_counts: Vec<(String, usize, usize)> = Vec::new(); // (name, count, start_row)
    for root_path in &roots {
        let name = project::home_relative_path(root_path.as_path());
        let count = app
            .projects()
            .iter()
            .filter(|item| item.path().starts_with(root_path.as_path()))
            .count();
        let start_row = root_counts
            .last()
            .map_or(0, |(_, prev_count, prev_start)| prev_start + prev_count);
        root_counts.push((name, count, start_row));
    }

    let prefix = "Roots: ";
    let inner_max = max_width.saturating_sub(2);
    if inner_max <= prefix.len() {
        return format!(" {prefix} ");
    }

    let section_indicator = |section_start: usize, section_len: usize| -> String {
        if focused && cursor >= section_start && cursor < section_start + section_len {
            crate::tui::types::scroll_indicator(cursor - section_start, section_len)
        } else {
            section_len.to_string()
        }
    };

    let parts: Vec<String> = root_counts
        .iter()
        .map(|(name, count, start)| format!("{name} ({})", section_indicator(*start, *count)))
        .collect();

    let body = parts.join(", ");
    let full = format!(" {prefix}{body} ");
    if full.len() <= max_width + 2 {
        return full;
    }
    // Truncate if too long.
    format!(
        " {prefix}{} ",
        truncate_to_width(&body, inner_max.saturating_sub(prefix.len()))
    )
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

    let border_color = if app.is_focused(PaneId::Output) {
        ACTIVE_BORDER_COLOR
    } else {
        LABEL_COLOR
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(border_color));

    let lines: Vec<Line> = app
        .example_output()
        .iter()
        .map(|l| {
            let padded = format!(" {l}");
            ansi_to_tui::IntoText::into_text(&padded).map_or_else(
                |_| Line::from(Span::raw(padded.clone())),
                |text| {
                    text.lines
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| Line::from(""))
                },
            )
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
    let is_rust = app.selected_item().is_some_and(RootItem::is_rust);
    let clear_lint_action = app
        .selected_project_path()
        .and_then(|path| app.lint_at_path(path))
        .filter(|lr| !lr.runs().is_empty())
        .map(|_| "clear cache");
    let groups = super::shortcuts::for_status_bar(
        context,
        enter_action,
        is_rust,
        clear_lint_action,
        app.current_keymap(),
        app.terminal_command_configured(),
    );

    let mut left_spans = Vec::new();
    if !app.is_scan_complete() {
        let key_style = Style::default()
            .fg(ACCENT_COLOR)
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
    let lang = if item.is_rust() {
        item.lang_icon()
    } else {
        app.projects()
            .at_path(item.path())
            .and_then(|p| p.language_stats.as_ref())
            .and_then(|ls| ls.entries.first())
            .map_or("  ", |e| crate::project::language_icon(&e.language))
    };
    let lint = app.lint_icon_for_root(node_index);
    let origin_sync = app.git_sync(item.path());
    let main_sync = app.git_main(item.path());
    let git_path_state = app.git_path_state_for_item(item);
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
    let wt_health = item.worktree_health();
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
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
        lint_style: lint_style_for(app, item.path()),
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: wt_health,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
}

/// Build a `ListItem` for a child project (workspace member or worktree).
fn render_child_item(
    app: &App,
    project: &project::PackageProject,
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
    let lang = project::PackageProject::lang_icon();
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
        ("0.0", Some(" [x]"), Some(Style::default().fg(LABEL_COLOR)))
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
        lint_style: lint_style_for(app, path),
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: project.worktree_health(),
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

    let (wt_name, has_expandable_children) = worktree_entry_name_and_expandable(item, wi, &dp);

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
    let wt_health = worktree_health_for_entry(item, wi);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
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
        lint_style: lint_style_for(app, wt_abs),
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: wt_health,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
}

fn worktree_entry_name_and_expandable(
    item: &RootItem,
    wi: usize,
    fallback: &str,
) -> (String, bool) {
    let name = match item {
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            ws.worktree_name()
                .map_or_else(|| ws.root_directory_name().into_string(), str::to_string)
        },
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
            primary,
            linked,
            ..
        }) => {
            let pkg = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            pkg.worktree_name()
                .map_or_else(|| pkg.root_directory_name().into_string(), str::to_string)
        },
        _ => fallback.to_string(),
    };

    let expandable = match item {
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            ws.has_members()
        },
        _ => false,
    };

    (name, expandable)
}

fn disk_suffix_for_state(
    disk: &str,
    deleted: bool,
    health: project::WorktreeHealth,
) -> (&str, Option<&'static str>, Option<Style>) {
    if deleted {
        ("0.0", Some(" [x]"), Some(Style::default().fg(LABEL_COLOR)))
    } else if matches!(health, project::WorktreeHealth::Broken) {
        (
            disk,
            Some(" [broken]"),
            Some(Style::default().fg(Color::White).bg(ERROR_COLOR)),
        )
    } else {
        (disk, None, None)
    }
}

fn worktree_health_for_entry(item: &RootItem, wi: usize) -> WorktreeHealth {
    match item {
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            if wi == 0 {
                primary.worktree_health()
            } else {
                linked
                    .get(wi - 1)
                    .map_or(Normal, ProjectFields::worktree_health)
            }
        },
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
            primary,
            linked,
            ..
        }) => {
            if wi == 0 {
                primary.worktree_health()
            } else {
                linked
                    .get(wi - 1)
                    .map_or(Normal, ProjectFields::worktree_health)
            }
        },
        _ => Normal,
    }
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
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
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
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            let group = &ws.groups()[gi];
            let m = &group.members()[mi];
            (Some(m), m.package_name().into_string(), group.is_named())
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
                crate::project::RootItem::Worktrees(
                    crate::project::WorktreeGroup::Workspaces {
                        primary, linked, ..
                    },
                ) => {
                    let ws = if wi == 0 {
                        primary
                    } else {
                        linked.get(wi - 1).unwrap_or(primary)
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
        crate::project::RootItem::Rust(crate::project::RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string(), group.is_named())
        },
        crate::project::RootItem::Worktrees(
            wtg @ crate::project::WorktreeGroup::Workspaces { primary, .. },
        ) if !wtg.renders_as_group() => {
            let ws = wtg.single_live_workspace().unwrap_or(primary);
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string(), group.is_named())
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
        crate::project::RootItem::Rust(crate::project::RustProject::Workspace(ws)) => {
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        crate::project::RootItem::Worktrees(
            wtg @ crate::project::WorktreeGroup::Workspaces { primary, .. },
        ) if !wtg.renders_as_group() => {
            let ws = wtg.single_live_workspace().unwrap_or(primary);
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        crate::project::RootItem::Rust(crate::project::RustProject::Package(pkg)) => {
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        crate::project::RootItem::Worktrees(
            wtg @ crate::project::WorktreeGroup::Packages { primary, .. },
        ) if !wtg.renders_as_group() => {
            let pkg = wtg.single_live_package().unwrap_or(primary);
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        _ => (None, String::new()),
    };
    let name = format!("{vendored_display_name} (v)");
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

fn render_submodule_item(
    app: &App,
    node_index: usize,
    submodule_index: usize,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let Some(submodule) = item.submodules().get(submodule_index) else {
        let row = super::columns::build_group_header_cells(PREFIX_SUBMODULE, "");
        return ListItem::new(super::columns::row_to_line(&row, widths));
    };
    let path = submodule.path.as_path();
    let name = format!("{} (s)", submodule.name);
    let git_path_state = app.git_path_state_for(path);
    let deleted = app.is_deleted(item.path());
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix: PREFIX_SUBMODULE,
        name: &name,
        name_segments: app.discovery_name_segments_for_path(
            path,
            &name,
            git_path_state,
            DiscoveryRowKind::PathOnly,
        ),
        git_path_state,
        lint_icon: " ",
        lint_style: Style::default(),
        disk: "",
        disk_style: Style::default(),
        disk_suffix: None,
        disk_suffix_style: None,
        lang_icon: "  ",
        git_origin_sync: "",
        git_main: "",
        ci: None,
        deleted,
        worktree_health: Normal,
    });
    ListItem::new(super::columns::row_to_line(&row, widths))
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
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
            primary,
            linked,
            ..
        }) => {
            let ws = if worktree_index == 0 {
                primary
            } else {
                linked.get(worktree_index - 1).unwrap_or(primary)
            };
            ws.vendored().get(vendored_index)
        },
        crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
            primary,
            linked,
            ..
        }) => {
            let pkg = if worktree_index == 0 {
                primary
            } else {
                linked.get(worktree_index - 1).unwrap_or(primary)
            };
            pkg.vendored().get(vendored_index)
        },
        _ => None,
    };
    let vendored_display_name =
        vendored_pkg.map_or_else(String::new, |p| p.package_name().into_string());
    let name = format!("{vendored_display_name} (v)");
    vendored_pkg.map_or_else(
        || {
            let row = super::columns::build_group_header_cells(PREFIX_WT_VENDORED, &name);
            ListItem::new(super::columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = match item {
                crate::project::RootItem::Worktrees(
                    crate::project::WorktreeGroup::Workspaces {
                        primary, linked, ..
                    },
                ) => {
                    let ws = if worktree_index == 0 {
                        primary
                    } else {
                        linked.get(worktree_index - 1).unwrap_or(primary)
                    };
                    app.is_deleted(ws.path())
                },
                crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
                    primary,
                    linked,
                    ..
                }) => {
                    let pkg = if worktree_index == 0 {
                        primary
                    } else {
                        linked.get(worktree_index - 1).unwrap_or(primary)
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
                    crate::project::RootItem::Rust(crate::project::RustProject::Workspace(ws)) => {
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
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => render_submodule_item(app, *node_index, *submodule_index, widths),
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
                crate::project::PackageProject::lang_icon()
            } else {
                app.projects()
                    .at_path(abs)
                    .and_then(|p| p.language_stats.as_ref())
                    .and_then(|ls| ls.entries.first())
                    .map_or("  ", |e| crate::project::language_icon(&e.language))
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
                ("0.0", Some(" [x]"), Some(Style::default().fg(LABEL_COLOR)))
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
                lint_style: lint_style_for(app, abs),
                disk: disk_text,
                disk_style: ds,
                disk_suffix,
                disk_suffix_style,
                lang_icon: lang,
                git_origin_sync: &origin_sync,
                git_main: &main_sync,
                ci,
                deleted,
                worktree_health: metadata.as_ref().map_or(Normal, |m| m.worktree_health),
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
