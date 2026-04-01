use std::collections::HashMap;
use std::time::Duration;

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

use super::LINT_SPINNER;
use super::animation::OFFLINE_PULSE;
use super::app::App;
use super::app::CiState;
use super::app::ConfirmAction;
use super::app::ExpandKey;
use super::app::FitWidths;
use super::app::NetworkStatus;
use super::app::VisibleRow;
use super::constants::BLOCK_BORDER_WIDTH;
use super::constants::BYTES_PER_GIB;
use super::constants::BYTES_PER_MIB;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::DETAIL_PANEL_HEIGHT;
use super::constants::OFFLINE_PULSE_AMPLITUDE;
use super::constants::OFFLINE_PULSE_BLUE;
use super::constants::OFFLINE_PULSE_GREEN;
use super::constants::OFFLINE_PULSE_OFFSET;
use super::constants::OFFLINE_PULSE_RED;
use super::constants::SEARCH_BAR_HEIGHT;
use super::shortcuts::Shortcut;
use super::types::FocusTarget;
use super::types::LayoutCache;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::constants::GIT_CLONE;
use crate::constants::GIT_FORK;
use crate::constants::GIT_LOCAL;
use crate::constants::IN_SYNC;
use crate::constants::WORKTREE;
use crate::project;
use crate::project::GitOrigin;
use crate::project::ProjectLanguage::Rust;
use crate::project::RustProject;

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
    } else {
        format!("{:.1} MiB", bytes as f64 / BYTES_PER_MIB as f64)
    }
}

/// Terminal display width of a string, accounting for multi-byte and wide characters.
/// Use this for ALL layout calculations — never `.len()` for column sizing.
pub(super) fn display_width(s: &str) -> usize { UnicodeWidthStr::width(s) }

// ── Row prefix strings ───────────────────────────────────────────────
// Single source of truth: width calc and render both reference these.

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼ ";
pub(super) const PREFIX_ROOT_COLLAPSED: &str = "▶ ";
pub(super) const PREFIX_ROOT_LEAF: &str = "  ";
pub(super) const PREFIX_MEMBER_INLINE: &str = "    ";
pub(super) const PREFIX_MEMBER_NAMED: &str = "        ";
pub(super) const PREFIX_GROUP_EXPANDED: &str = "    ▼ ";
pub(super) const PREFIX_GROUP_COLLAPSED: &str = "    ▶ ";
pub(super) const PREFIX_WT_EXPANDED: &str = "    ▼ ";
pub(super) const PREFIX_WT_COLLAPSED: &str = "    ▶ ";
pub(super) const PREFIX_WT_FLAT: &str = "    ";
pub(super) const PREFIX_WT_GROUP_EXPANDED: &str = "        ▼ ";
pub(super) const PREFIX_WT_GROUP_COLLAPSED: &str = "        ▶ ";
pub(super) const PREFIX_WT_MEMBER_INLINE: &str = "        ";
pub(super) const PREFIX_WT_MEMBER_NAMED: &str = "            ";

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

pub(super) struct RowData<'a> {
    pub prefix:     &'a str,
    pub name:       &'a str,
    pub lint_icon:  &'a str,
    pub lang_icon:  &'a str,
    pub disk:       &'a str,
    pub disk_style: Style,
    pub ci:         Option<Conclusion>,
    pub git_icon:   &'a str,
    pub git_sync:   &'a str,
    pub name_width: usize,
    pub disk_width: usize,
    pub sync_width: usize,
    pub deleted:    bool,
}

/// Pad a string to a target display width using trailing spaces (left-aligned).
fn pad_right(s: &str, target: usize) -> String {
    let w = display_width(s);
    let pad = target.saturating_sub(w);
    format!("{s}{}", " ".repeat(pad))
}

/// Pad a string to a target display width using leading spaces (right-aligned).
fn pad_left(s: &str, target: usize) -> String {
    let w = display_width(s);
    let pad = target.saturating_sub(w);
    format!("{}{s}", " ".repeat(pad))
}

/// Pad a string to a target display width, centered.
fn pad_center(s: &str, target: usize) -> String {
    let w = display_width(s);
    let total_pad = target.saturating_sub(w);
    let left = total_pad / 2;
    let right = total_pad - left;
    format!("{}{s}{}", " ".repeat(left), " ".repeat(right))
}

/// Max display width across all CI conclusion icons.
fn ci_icon_width() -> usize {
    [
        Conclusion::Success,
        Conclusion::Failure,
        Conclusion::Cancelled,
    ]
    .iter()
    .map(|c| display_width(c.icon()))
    .max()
    .unwrap_or(1)
}

/// Display width of the lint icon column (widest lint icon).
fn lint_icon_width() -> usize {
    display_width(crate::constants::LINT_PASSED)
        .max(display_width(LINT_SPINNER.frame_at(Duration::ZERO)))
}

pub(super) fn project_row_spans(row: &RowData<'_>) -> Line<'static> {
    let prefix_width = display_width(row.prefix);
    let lint_w = lint_icon_width();
    let available = row.name_width.saturating_sub(prefix_width + lint_w);
    let padded_name = format!("{}{}", row.prefix, pad_right(row.name, available));
    let lint_cell = pad_right(row.lint_icon, lint_w);
    let disk_width = row.disk_width;
    let sync_width = row.sync_width;
    let sync_cell = if row.git_sync == IN_SYNC {
        if sync_width <= 2 {
            format!(" {}", pad_left(row.git_sync, sync_width))
        } else {
            format!(" {}", pad_center(row.git_sync, sync_width))
        }
    } else {
        format!(" {}", pad_right(row.git_sync, sync_width))
    };
    let ci_icon = pad_right(row.ci.map_or("", Conclusion::icon), ci_icon_width());
    let lang_cell = pad_right(row.lang_icon, display_width("🦀"));
    let git_cell = pad_right(row.git_icon, display_width(GIT_CLONE));

    if row.deleted {
        let strike = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::CROSSED_OUT);
        return Line::from(vec![
            Span::styled(padded_name, strike),
            Span::styled(lint_cell, strike),
            Span::styled(format!(" {:>disk_width$}", row.disk), strike),
            Span::styled(format!(" {lang_cell}"), strike),
            Span::styled(sync_cell, strike),
            Span::styled(format!(" {git_cell}"), strike),
            Span::styled(format!(" {ci_icon}"), strike),
        ]);
    }

    let ci_style = conclusion_style(row.ci);
    let origin_style = match row.git_icon {
        GIT_FORK => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        GIT_CLONE => Style::default().fg(Color::White),
        GIT_LOCAL => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    let sync_style = if row.git_sync == IN_SYNC {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };

    Line::from(vec![
        Span::raw(padded_name),
        Span::raw(lint_cell),
        Span::styled(format!(" {:>disk_width$}", row.disk), row.disk_style),
        Span::raw(format!(" {lang_cell}")),
        Span::styled(sync_cell, sync_style),
        Span::styled(format!(" {git_cell}"), origin_style),
        Span::styled(format!(" {ci_icon}"), ci_style),
    ])
}

/// Build a probe row using max-width placeholders and measure its display width.
/// This is the single source of truth for panel width — no hand-counted arithmetic.
pub(super) fn probe_row_width(name_width: usize, disk_width: usize, sync_width: usize) -> usize {
    let name_pad = "X".repeat(name_width);
    let disk_pad = "X".repeat(disk_width);
    let sync_pad = "X".repeat(sync_width);
    let row = project_row_spans(&RowData {
        prefix: "",
        name: &name_pad,
        lint_icon: crate::constants::LINT_PASSED,
        lang_icon: "🦀",
        disk: &disk_pad,
        disk_style: Style::default(),
        ci: Some(Conclusion::Success),
        git_icon: GIT_CLONE,
        git_sync: &sync_pad,
        name_width,
        disk_width,
        sync_width,
        deleted: false,
    });
    row.width()
}

pub(super) fn group_header_spans(prefix: &str, name: &str, name_width: usize) -> Line<'static> {
    let prefix_width = display_width(prefix);
    let available = name_width.saturating_sub(prefix_width);
    let padded = format!("{prefix}{}", pad_right(name, available));
    Line::from(vec![Span::styled(
        padded,
        Style::default().fg(Color::Yellow),
    )])
}

pub(super) fn ui(frame: &mut Frame, app: &mut App) {
    app.layout_cache = LayoutCache::default();

    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let left_width = u16::try_from(
        probe_row_width(
            app.cached_fit_widths.name,
            app.cached_fit_widths.disk,
            app.cached_fit_widths.sync,
        ) + BLOCK_BORDER_WIDTH,
    )
    .unwrap_or(u16::MAX);

    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(outer_layout[0]);

    render_left_panel(frame, app, main_layout[0]);
    render_right_panel(frame, app, main_layout[1]);

    render_status_bar(frame, app, outer_layout[1]);

    if app.show_settings {
        super::settings::render_settings_popup(frame, app);
    }
    if app.show_finder {
        super::finder::render_finder_popup(frame, app);
    }
    if let Some(ref action) = app.confirm {
        render_confirm_popup(frame, action);
    }
}

fn render_confirm_popup(frame: &mut Frame, action: &ConfirmAction) {
    let prompt = match action {
        ConfirmAction::Clean(_) => "Run cargo clean?",
    };

    let text = format!(" {prompt}  (y/n) ");
    let width = u16::try_from(text.len() + 4).unwrap_or(u16::MAX);
    let area = centered_rect(width, CONFIRM_DIALOG_HEIGHT, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
    let search_height = if app.searching { SEARCH_BAR_HEIGHT } else { 0 };
    let left_constraints = if app.scan_complete {
        vec![Constraint::Length(search_height), Constraint::Min(1)]
    } else {
        let project_rows = u16::try_from(app.visible_rows().len()).unwrap_or(u16::MAX);
        let project_height = (project_rows + 2).max(3);
        vec![
            Constraint::Length(search_height),
            Constraint::Length(project_height),
            Constraint::Min(3),
        ]
    };
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(left_constraints)
        .split(area);

    app.layout_cache.project_list = left_layout[1];
    app.layout_cache.scan_log = if app.scan_complete {
        None
    } else {
        Some(left_layout[2])
    };

    if app.searching {
        render_search_bar(frame, app, left_layout[0]);
    }

    render_project_list(frame, app, left_layout[1]);

    if !app.scan_complete {
        render_scan_log(frame, app, left_layout[2]);
    }
}

fn render_right_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.render_widget(Clear, area);

    let detail_info = app.cached_detail.as_ref().map(|c| c.info.clone());
    let has_ci = app
        .selected_project()
        .and_then(|p| app.ci_state_for(p))
        .is_some();
    let detail_ci_runs: Vec<CiRun> = app
        .selected_project()
        .and_then(|p| app.ci_state_for(p))
        .map(|s: &CiState| s.runs().to_vec())
        .unwrap_or_default();
    let has_example_output = !app.example_output.is_empty();

    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(DETAIL_PANEL_HEIGHT),
            Constraint::Min(3),
        ])
        .split(area);

    // CI content_area and len are set inside render_ci_panel.

    super::detail::render_detail_panel(frame, app, detail_info.as_ref(), right_layout[0]);

    // Running output replaces the CI panel; Esc restores it.
    if has_example_output {
        render_example_output(frame, app, right_layout[1]);
    } else if has_ci {
        super::detail::render_ci_panel(frame, app, &detail_ci_runs, right_layout[1]);
        if app.network_status == NetworkStatus::Offline {
            render_offline_overlay(frame, app, right_layout[1]);
        }
    } else {
        let selected_project_ref = app.selected_project();
        render_empty_ci_panel(frame, app, selected_project_ref, right_layout[1]);
        if app.network_status == NetworkStatus::Offline {
            render_offline_overlay(frame, app, right_layout[1]);
        }
    }
}

fn render_empty_ci_panel(frame: &mut Frame, app: &App, project: Option<&RustProject>, area: Rect) {
    let ci_focused = app.focus == FocusTarget::CiRuns;
    let border_style = if ci_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" CI ")
        .title_style(Style::default().fg(Color::DarkGray))
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Determine why there's no CI
    let has_git = project.is_some_and(|p| app.git_info.contains_key(&p.path));
    let has_url = project
        .and_then(|p| app.git_info.get(&p.path))
        .is_some_and(|g| g.url.is_some());
    let is_local = project
        .and_then(|p| app.git_info.get(&p.path))
        .is_some_and(|g| g.origin == GitOrigin::Local);

    let msg = if !has_git {
        "Not a git repository"
    } else if is_local || !has_url {
        "CI requires a GitHub origin remote"
    } else if !app.scan_complete {
        "Loading..."
    } else {
        "No CI runs found"
    };

    if inner.height > 0 {
        let text = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::DarkGray),
        )))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(text, inner);
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0.0..=255.0 before cast"
)]
fn render_offline_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let msg = "  No internet connection  ";

    let progress = OFFLINE_PULSE.progress_at(app.animation_elapsed());
    let pulse = (progress * std::f64::consts::TAU)
        .sin()
        .mul_add(OFFLINE_PULSE_AMPLITUDE, OFFLINE_PULSE_OFFSET);

    let r = (OFFLINE_PULSE_RED * pulse).clamp(0.0, 255.0) as u8;
    let g = (OFFLINE_PULSE_GREEN * pulse).clamp(0.0, 255.0) as u8;
    let fg = Color::Rgb(r, g, OFFLINE_PULSE_BLUE);

    let msg_width = u16::try_from(msg.len()).unwrap_or(u16::MAX);
    let x = area.x + area.width.saturating_sub(msg_width) / 2;
    let y = area.y + area.height / 2;

    if y >= area.y && y < area.y + area.height {
        let overlay_area = Rect {
            x,
            y,
            width: msg_width.min(area.width),
            height: 1,
        };
        let widget = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(widget, overlay_area);
    }
}

pub(super) fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let search_style = if app.searching {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let search_text = if app.searching {
        if app.search_query.is_empty() {
            "…".to_string()
        } else {
            app.search_query.clone()
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
            .border_style(if app.searching {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            }),
    );

    frame.render_widget(search_bar, area);
}

pub(super) fn render_project_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let widths = &app.cached_fit_widths;

    let mut items: Vec<ListItem> = if app.searching && !app.search_query.is_empty() {
        render_filtered_items(app, widths)
    } else {
        render_tree_items(app, widths)
    };

    // Append disk total as the last row
    let total_bytes: u64 = app.disk_usage.values().sum();
    if total_bytes > 0 {
        let total_str = format_bytes(total_bytes);
        let total_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let mut row_line = project_row_spans(&RowData {
            prefix:     "",
            name:       &format!("{:>width$}", "Σ", width = widths.name),
            lint_icon:  crate::constants::LINT_NO_LOG,
            lang_icon:  "  ",
            disk:       &total_str,
            disk_style: total_style,
            ci:         None,
            git_icon:   " ",
            git_sync:   "",
            name_width: widths.name,
            disk_width: widths.disk,
            sync_width: widths.sync,
            deleted:    false,
        });
        if let Some(span) = row_line.spans.first_mut() {
            span.style = total_style;
        }
        items.push(ListItem::new(row_line));
    }

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let node_count = app.live_node_count();
    let scan_root = project::home_relative_path(&app.scan_root);
    // "Lint" right-aligns with the lint icon column: "Li" bleeds into
    // name padding, "nt" sits over the icon.  The header word "Lint" is
    // 4 chars; the icon column is `lint_icon_width()` (2).  So the name
    // span shrinks by 2 ("Li") and "Lint" is appended.
    let lint_w = lint_icon_width();
    let lint_header = format!("{:>width$}", "Lint", width = lint_w + 2);
    let name_header_pad = widths
        .name
        .saturating_sub(scan_root.len() + 3 + node_count.to_string().len() + 2);
    let header_line = Line::from(vec![
        Span::styled(
            format!(
                "{scan_root} ({node_count}){:<pad$}",
                "",
                pad = name_header_pad
            ),
            header_style,
        ),
        Span::styled(lint_header, header_style),
        Span::styled(
            format!(" {:>width$}", "Disk", width = widths.disk),
            header_style,
        ),
        Span::styled("  R Git CI", header_style),
    ]);

    let project_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(header_line)
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(if app.focus == FocusTarget::ProjectList {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        });

    frame.render_stateful_widget(project_list, area, &mut app.list_state);
}

pub(super) fn render_scan_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let log_items: Vec<ListItem> = app
        .scan_log
        .iter()
        .map(|p| {
            ListItem::new(Span::styled(
                format!("  {p}"),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let scan_focused = app.focus == FocusTarget::ScanLog;
    let scan_title = if scan_focused {
        " Scanning (focused) "
    } else {
        " Scanning "
    };
    let scan_log = List::new(log_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(scan_title)
                .title_style(if scan_focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                })
                .border_style(if scan_focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(scan_log, area, &mut app.scan_log_state);
}

fn render_example_output(frame: &mut Frame, app: &App, area: Rect) {
    let title = app.example_running.as_ref().map_or_else(
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
        .border_style(if app.example_running.is_some() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let lines: Vec<Line> = app
        .example_output
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
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    for shortcut in shortcuts {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!(" {}", shortcut.key), key_style));
        spans.push(Span::raw(format!(" {}", shortcut.description)));
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
    // Flash message takes over the entire status bar with a contrasting background.
    let flash_active = app.status_flash.as_ref().is_some_and(|(_, created)| {
        created.elapsed().as_millis() < u128::from(app.status_flash_millis)
    });

    if flash_active {
        if let Some((ref msg, _)) = app.status_flash {
            let flash_bar_style = Style::default().bg(Color::Yellow).fg(Color::Black);
            frame.render_widget(Paragraph::new("").style(flash_bar_style), area);

            let flash_text_style = flash_bar_style.add_modifier(Modifier::BOLD);
            let total_width = area.width as usize;
            let flash_width = msg.width();
            let flash_start = total_width.saturating_sub(flash_width) / 2;
            let flash_area = Rect {
                x:      area.x + u16::try_from(flash_start).unwrap_or(u16::MAX),
                y:      area.y,
                width:  u16::try_from((total_width - flash_start).min(flash_width + 1))
                    .unwrap_or(u16::MAX),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(msg.clone(), flash_text_style)))
                    .style(flash_bar_style),
                flash_area,
            );
        }
        return;
    }

    let bar_style = Style::default().bg(Color::DarkGray).fg(Color::White);

    // Fill the entire bar with the background color
    frame.render_widget(Paragraph::new("").style(bar_style), area);

    let context = app.input_context();
    let enter_action = app.enter_action();
    let is_rust = app.selected_project().is_some_and(|p| p.is_rust == Rust);
    let groups = super::shortcuts::for_status_bar(context, enter_action, is_rust);

    let mut left_spans = Vec::new();
    if !app.scan_complete {
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
    root_sorted: &[u64],
    name_width: usize,
    disk_width: usize,
    sync_width: usize,
) -> ListItem<'static> {
    let node = &app.nodes[node_index];
    let project = &node.project;
    let mut name = project.display_name();
    let live_wt = app.live_worktree_count(node);
    if live_wt > 0 {
        name = format!("{name} {WORKTREE}:{live_wt}");
    }
    let disk = app.formatted_disk_for_node(node);
    let disk_bytes = app.disk_bytes_for_node(node);
    let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
    let ci = app.ci_for_node(node);
    let lang = project.lang_icon();
    let lint = app.lint_icon(project);
    let git = app.git_icon(project);
    let sync = app.git_sync(project);
    let prefix = if node.has_children() {
        if app.expanded.contains(&ExpandKey::Node(node_index)) {
            PREFIX_ROOT_EXPANDED
        } else {
            PREFIX_ROOT_COLLAPSED
        }
    } else {
        PREFIX_ROOT_LEAF
    };
    ListItem::new(project_row_spans(&RowData {
        prefix,
        name: &name,
        lint_icon: lint,
        lang_icon: lang,
        disk: &disk,
        disk_style: ds,
        ci,
        git_icon: git,
        git_sync: &sync,
        name_width,
        disk_width,
        sync_width,
        deleted: app.is_deleted(&project.path),
    }))
}

/// Build a `ListItem` for a child project (workspace member or worktree).
fn render_child_item(
    app: &App,
    project: &RustProject,
    name: &str,
    child_sorted: &[u64],
    prefix: &'static str,
    widths: &FitWidths,
) -> ListItem<'static> {
    let disk = app.formatted_disk(project);
    let disk_bytes = app.disk_usage.get(&project.path).copied();
    let ds = disk_color(disk_percentile(disk_bytes, child_sorted));
    let lang = project.lang_icon();
    let lint = app.lint_icon(project);
    let ci = app.ci_for(project);
    let git = app.git_icon(project);
    let sync = app.git_sync(project);
    ListItem::new(project_row_spans(&RowData {
        prefix,
        name,
        lint_icon: lint,
        lang_icon: lang,
        disk: &disk,
        disk_style: ds,
        ci,
        git_icon: git,
        git_sync: &sync,
        name_width: widths.name,
        disk_width: widths.disk,
        sync_width: widths.sync,
        deleted: app.is_deleted(&project.path),
    }))
}

fn render_worktree_entry<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &FitWidths,
) -> ListItem<'a> {
    let wt = &app.nodes[ni].worktrees[wi];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);
    let name = wt
        .project
        .worktree_name
        .as_deref()
        .unwrap_or(&wt.project.path)
        .to_string();
    let prefix = if wt.has_members() {
        if app.expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            PREFIX_WT_EXPANDED
        } else {
            PREFIX_WT_COLLAPSED
        }
    } else {
        PREFIX_WT_FLAT
    };
    render_child_item(app, &wt.project, &name, sorted, prefix, widths)
}

fn render_wt_group_header<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    name_width: usize,
) -> ListItem<'a> {
    let group = &app.nodes[ni].worktrees[wi].groups[gi];
    let prefix = if app.expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
        PREFIX_WT_GROUP_EXPANDED
    } else {
        PREFIX_WT_GROUP_COLLAPSED
    };
    let label = format!("{} ({})", group.name, group.members.len());
    ListItem::new(group_header_spans(prefix, &label, name_width))
}

fn render_wt_member<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    mi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &FitWidths,
) -> ListItem<'a> {
    let wt = &app.nodes[ni].worktrees[wi];
    let group = &wt.groups[gi];
    let member = &group.members[mi];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);
    let indent = if group.name.is_empty() {
        PREFIX_WT_MEMBER_INLINE
    } else {
        PREFIX_WT_MEMBER_NAMED
    };
    let name = member.display_name();
    render_child_item(app, member, &name, sorted, indent, widths)
}

pub(super) fn render_tree_items(app: &App, widths: &FitWidths) -> Vec<ListItem<'static>> {
    let root_sorted = &app.cached_root_sorted;
    let child_sorted = &app.cached_child_sorted;

    let rows = app.visible_rows();
    rows.iter()
        .map(|row| match row {
            VisibleRow::Root { node_index } => render_root_item(
                app,
                *node_index,
                root_sorted,
                widths.name,
                widths.disk,
                widths.sync,
            ),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let prefix = if app
                    .expanded
                    .contains(&ExpandKey::Group(*node_index, *group_index))
                {
                    PREFIX_GROUP_EXPANDED
                } else {
                    PREFIX_GROUP_COLLAPSED
                };
                let label = format!("{} ({})", group.name, group.members.len());
                ListItem::new(group_header_spans(prefix, &label, widths.name))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let member = &group.members[*member_index];
                let empty = Vec::new();
                let sorted = child_sorted.get(node_index).unwrap_or(&empty);
                let indent = if group.name.is_empty() {
                    PREFIX_MEMBER_INLINE
                } else {
                    PREFIX_MEMBER_NAMED
                };
                let name = member.display_name();
                render_child_item(app, member, &name, sorted, indent, widths)
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => render_worktree_entry(app, *node_index, *worktree_index, child_sorted, widths),
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => {
                render_wt_group_header(app, *node_index, *worktree_index, *group_index, widths.name)
            },
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
        })
        .collect()
}

pub(super) fn render_filtered_items(app: &App, widths: &FitWidths) -> Vec<ListItem<'static>> {
    let root_sorted = &app.cached_root_sorted;
    app.filtered
        .iter()
        .filter_map(|&flat_idx| {
            let entry = app.flat_entries.get(flat_idx)?;
            let node = app.nodes.get(entry.node_index)?;
            let project = node
                .groups
                .get(entry.group_index)
                .and_then(|g| g.members.get(entry.member_index))
                .unwrap_or(&node.project);
            let disk = app.formatted_disk(project);
            let disk_bytes = app.disk_usage.get(&project.path).copied();
            let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
            let lang = project.lang_icon();
            let lint = app.lint_icon(project);
            let ci = app.ci_for(project);
            let git = app.git_icon(project);
            let sync = app.git_sync(project);
            Some(ListItem::new(project_row_spans(&RowData {
                prefix: "  ",
                name: &entry.name,
                lint_icon: lint,
                lang_icon: lang,
                disk: &disk,
                disk_style: ds,
                ci,
                git_icon: git,
                git_sync: &sync,
                name_width: widths.name,
                disk_width: widths.disk,
                sync_width: widths.sync,
                deleted: app.is_deleted(&project.path),
            })))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_display_widths() {
        assert_eq!(display_width("🌲"), 2);
        assert_eq!(display_width("🦀"), 2);
        assert_eq!(display_width("bevy_brp"), 8);
        assert_eq!(display_width("bevy_brp 🌲:2"), 13);

        // pad_right should add correct spaces
        let padded = pad_right("bevy_brp 🌲:2", 27);
        assert_eq!(display_width(&padded), 27, "padded display width");

        let padded_ascii = pad_right("bevy_brp", 27);
        assert_eq!(
            display_width(&padded_ascii),
            27,
            "ascii padded display width"
        );
    }

    #[test]
    fn row_spans_same_width_with_and_without_emoji() {
        let name_width = 32;
        let disk_width = 8;
        let sync_width = 2;

        let row_emoji = project_row_spans(&RowData {
            prefix: "▶ ",
            name: "bevy_brp 🌲:2",
            lint_icon: crate::constants::LINT_PASSED,
            lang_icon: "🦀",
            disk: "36.3 GiB",
            disk_style: Style::default(),
            ci: Some(Conclusion::Success),
            git_icon: GIT_CLONE,
            git_sync: "↑2",
            name_width,
            disk_width,
            sync_width,
            deleted: false,
        });

        let row_ascii = project_row_spans(&RowData {
            prefix: "▶ ",
            name: "bevy_mesh_outline_benchmark",
            lint_icon: crate::constants::LINT_PASSED,
            lang_icon: "🦀",
            disk: "36.3 GiB",
            disk_style: Style::default(),
            ci: Some(Conclusion::Success),
            git_icon: GIT_CLONE,
            git_sync: "↑2",
            name_width,
            disk_width,
            sync_width,
            deleted: false,
        });

        // Debug: check each span
        let emoji_spans: Vec<usize> = row_emoji
            .spans
            .iter()
            .map(|s| display_width(s.content.as_ref()))
            .collect();
        let ascii_spans: Vec<usize> = row_ascii
            .spans
            .iter()
            .map(|s| display_width(s.content.as_ref()))
            .collect();
        assert_eq!(
            emoji_spans, ascii_spans,
            "per-span widths should match\nemoji spans: {emoji_spans:?}\nascii spans: {ascii_spans:?}"
        );
    }
}
