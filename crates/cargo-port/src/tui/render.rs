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
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::app::ConfirmAction;
use super::app::DiscoveryRowKind;
use super::app::ExpandKey;
use super::app::ResolvedWidths;
use super::app::VisibleRow;
use super::constants::ACCENT_COLOR;
use super::constants::BLOCK_BORDER_WIDTH;
use super::constants::BYTES_PER_GIB;
use super::constants::BYTES_PER_KIB;
use super::constants::BYTES_PER_MIB;
use super::constants::COLUMN_HEADER_COLOR;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::ERROR_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SECONDARY_TEXT_COLOR;
use super::constants::STATUS_BAR_COLOR;
use super::constants::SUCCESS_COLOR;
use super::constants::TITLE_COLOR;
use super::interaction::UiSurface::Content;
use super::pane;
use super::pane::PaneTitleCount;
use super::pane::PaneTitleGroup;
use super::panes;
use super::panes::LayoutCache;
use super::panes::PaneId;
use super::shortcuts::Shortcut;
use super::shortcuts::ShortcutState;
use crate::ci::Conclusion;
use crate::project;
use crate::project::ProjectFields;
use crate::project::RootItem;
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
        .pane_data()
        .cpu
        .as_ref()
        .map_or(1, |snapshot| snapshot.cores.len());
    let tiled = panes::resolve_layout(outer_layout[0], left_width, core_count, bottom_row);

    for resolved in tiled.panes() {
        render_tiled_pane(frame, app, resolved.pane, resolved.area);
    }
    sync_layout_pane_hitboxes(app, &tiled);
    app.layout_cache_mut().tiled = tiled;

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

    sync_hovered_pane_row(app);
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

fn render_left_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    render_project_list(frame, app, area);
}

fn pane_render_styles() -> panes::RenderStyles {
    panes::RenderStyles {
        readonly_label: Style::default().fg(LABEL_COLOR),
        chrome:         pane::default_pane_chrome(),
    }
}

fn render_tiled_pane(frame: &mut Frame, app: &mut App, pane: PaneId, area: Rect) {
    match pane {
        PaneId::ProjectList => render_left_panel(frame, app, area),
        PaneId::Package => panes::render_package_panel(frame, app, area),
        PaneId::Git => panes::render_git_panel(frame, app, area),
        PaneId::Lang => {
            panes::render_lang_panel_standalone(frame, app, &pane_render_styles(), area);
        },
        PaneId::Cpu => panes::render_cpu_panel(frame, app, &pane_render_styles(), area),
        PaneId::Targets => {
            if let Some(targets_data) = app.pane_data().targets.clone() {
                if targets_data.has_targets() {
                    panes::render_targets_panel(
                        frame,
                        app,
                        &targets_data,
                        &pane_render_styles(),
                        area,
                    );
                } else {
                    panes::render_empty_targets_panel(frame, app, area);
                }
            } else {
                panes::render_empty_targets_panel(frame, app, area);
            }
        },
        PaneId::Lints => panes::render_lints_panel(frame, app, area),
        PaneId::CiRuns => panes::render_ci_panel(frame, app, area),
        PaneId::Output => render_example_output(frame, app, area),
        PaneId::Toasts | PaneId::Settings | PaneId::Finder | PaneId::Keymap => {},
    }
}

fn sync_layout_pane_hitboxes(app: &mut App, layout: &pane::ResolvedPaneLayout<PaneId>) {
    for resolved in layout.panes() {
        register_hitbox_for_pane(app, resolved.pane);
    }
}

fn sync_hovered_pane_row(app: &mut App) {
    let hovered = app
        .mouse_pos()
        .and_then(|pos| super::interaction::hovered_pane_row_at(app, pos));
    app.set_hovered_pane_row(hovered);
    app.apply_hovered_pane_row();
}

/// Exhaustive match on `PaneId` — adding a variant forces you to decide
/// whether the pane gets hitboxes.
fn register_hitbox_for_pane(app: &mut App, id: PaneId) {
    if panes::has_row_hitboxes(id) {
        let pane = app.pane_manager().pane(id).clone();
        super::interaction::register_pane_row_hitboxes(app, id, &pane, Content);
    }
}

pub(super) fn render_project_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let (mut items, header, summary_line, row_width) = {
        let widths = app.cached_fit_widths();
        let items: Vec<ListItem> = render_tree_items(app, widths);
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
    let block = pane::default_pane_chrome().block(title, app.is_focused(PaneId::ProjectList));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        clear_project_list_surface(app);
        app.layout_cache_mut().project_list_body = Rect::ZERO;
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
        clear_project_list_surface(app);
        app.layout_cache_mut().project_list_body = Rect::ZERO;
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
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_len(total_project_rows);
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_content_area(list_area);
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_viewport_rows(usize::from(list_area.height));
    let project_list = List::new(items);
    let mut list_state = ListState::default()
        .with_selected(Some(app.pane_manager().pane(PaneId::ProjectList).pos()));
    *list_state.offset_mut() = app.pane_manager().pane(PaneId::ProjectList).scroll_offset();
    frame.render_stateful_widget(project_list, list_area, &mut list_state);
    app.layout_cache_mut().project_list_body = list_area;
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_scroll_offset(list_state.offset());
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(list_state.selected().unwrap_or(0));
    super::interaction::register_project_list_hitboxes(app, list_area, row_width);

    if pin_summary && let Some(line) = summary_line {
        render_project_list_footer(frame, content_area, line);
    }

    pane::render_overflow_affordance(frame, area, app.pane_manager().pane(PaneId::ProjectList));
}

fn clear_project_list_surface(app: &mut App) {
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .clear_surface();
}

fn render_project_list_footer(frame: &mut Frame, content_area: Rect, line: Line<'static>) {
    let footer_area = Rect::new(
        content_area.x,
        content_area.y + content_area.height.saturating_sub(1),
        content_area.width,
        1,
    );
    frame.render_widget(Paragraph::new(line), footer_area);
}

fn project_panel_title_with_counts(app: &App, max_width: usize) -> String {
    let focused = app.is_focused(PaneId::ProjectList);
    let cursor = app.pane_manager().pane(PaneId::ProjectList).pos();
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

    let groups = root_counts
        .iter()
        .map(|(name, count, start)| PaneTitleGroup {
            label:  name.clone().into(),
            len:    *count,
            cursor: focused
                .then_some(cursor)
                .filter(|cursor| *cursor >= *start && *cursor < *start + *count)
                .map(|cursor| cursor - *start),
        })
        .collect();

    let body = PaneTitleCount::Grouped(groups).body();
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

    let block = pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(LABEL_COLOR))
        .block(title, app.is_focused(PaneId::Output));

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
        app.selected_project_is_deleted(),
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
    let git_status = app.git_status_for_item(item);
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
            git_status,
            DiscoveryRowKind::Root,
        ),
        git_status,
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
    project: &project::Package,
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
    let lang = project::Package::lang_icon();
    let lint = if app.is_rust_at_path(path) {
        app.lint_icon(path)
    } else {
        " "
    };
    let ci = if app.is_ci_owner_path(path) {
        app.ci_for(path)
    } else {
        None
    };
    let hide_git_status = app.is_workspace_member_path(path);
    let origin_sync = if hide_git_status
        || matches!(
            app.git_status_for(path),
            Some(crate::project::GitStatus::Untracked | crate::project::GitStatus::Ignored)
        ) {
        String::new()
    } else {
        app.git_sync(path)
    };
    let main_sync = if hide_git_status
        || matches!(
            app.git_status_for(path),
            Some(crate::project::GitStatus::Untracked | crate::project::GitStatus::Ignored)
        ) {
        String::new()
    } else {
        app.git_main(path)
    };
    let deleted = inherited_deleted || app.is_deleted(project.path());
    let git_status = app.git_status_for(path);
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
            git_status,
            DiscoveryRowKind::PathOnly,
        ),
        git_status,
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
    let git_status = app.git_status_for(wt_abs);
    let wt_health = worktree_health_for_entry(item, wi);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix,
        name: &wt_name,
        name_segments: app.discovery_name_segments_for_path(
            wt_abs,
            &wt_name,
            git_status,
            DiscoveryRowKind::WorktreeEntry,
        ),
        git_status,
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
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let item = &app.projects()[node_index];
    let Some(submodule) = item.submodules().get(submodule_index) else {
        let row = super::columns::build_group_header_cells(PREFIX_SUBMODULE, "");
        return ListItem::new(super::columns::row_to_line(&row, widths));
    };
    let name = format!("{} (s)", submodule.name);
    let sorted = child_sorted.get(&node_index).map_or(&[][..], Vec::as_slice);
    render_path_only_entry(
        app,
        submodule,
        item.path(),
        PREFIX_SUBMODULE,
        &name,
        sorted,
        widths,
    )
}

fn render_path_only_entry(
    app: &App,
    entry: &impl crate::project::ProjectFields,
    inherited_deleted_path: &Path,
    prefix: &'static str,
    name: &str,
    sorted: &[u64],
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    let path = entry.path().as_path();
    let disk = app.formatted_disk(path);
    let ds = disk_color(disk_percentile(entry.info().disk_usage_bytes, sorted));
    let git_status = app.git_status_for(path);
    let deleted = app.is_deleted(inherited_deleted_path) || app.is_deleted(path);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, entry.info().worktree_health);
    let row = super::columns::build_row_cells(super::columns::ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
            path,
            name,
            git_status,
            DiscoveryRowKind::PathOnly,
        ),
        git_status,
        lint_icon: " ",
        lint_style: Style::default(),
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: "  ",
        git_origin_sync: "",
        git_main: "",
        ci: None,
        deleted,
        worktree_health: entry.info().worktree_health,
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
    let focus = app.pane_focus_state(PaneId::ProjectList);
    let pane = app.pane_manager().pane(PaneId::ProjectList);

    let rows = app.visible_rows();
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let item = render_tree_item(app, row, &root_labels, root_sorted, child_sorted, widths);
            item.style(pane.selection_state(row_index, focus).overlay_style())
        })
        .collect()
}

fn render_tree_item(
    app: &App,
    row: &VisibleRow,
    root_labels: &[String],
    root_sorted: &[u64],
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ResolvedWidths,
) -> ListItem<'static> {
    match row {
        VisibleRow::Root { node_index } => {
            render_root_item(app, *node_index, root_labels, root_sorted, widths)
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
        } => render_submodule_item(app, *node_index, *submodule_index, child_sorted, widths),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::should_pin_project_summary;
    use crate::tui::panes;
    use crate::tui::panes::BottomRow;
    use crate::tui::panes::PaneId;

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
