use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use tui_pane::ACCENT_COLOR;
use tui_pane::ERROR_COLOR;
use tui_pane::INACTIVE_BORDER_COLOR;
use tui_pane::LABEL_COLOR;
use tui_pane::SUCCESS_COLOR;
use tui_pane::TITLE_COLOR;
use tui_pane::Viewport;
use tui_pane::render_overflow_affordance;
use unicode_width::UnicodeWidthStr;

use super::DetailField;
use super::PackageData;
use super::pane_impls::PackagePane;
use crate::tui::integration;
use crate::tui::pane;
use crate::tui::pane::PaneChrome;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::pane::PaneRule;
use crate::tui::pane::RuleTitle;
use crate::tui::panes;
use crate::tui::render;

/// Shared style constants for pane rendering.
pub struct RenderStyles {
    pub readonly_label: Style,
    pub chrome:         PaneChrome,
}

struct PackageRenderCtx<'a> {
    data:              &'a PackageData,
    fields:            &'a [DetailField],
    pane:              &'a Viewport,
    focus:             PaneFocusState,
    styles:            &'a RenderStyles,
    /// Threaded through so the Lint row can frame its icon at
    /// render time (the typed `LintDisplay` carries an unframed
    /// `LintStatus`).
    animation_elapsed: Duration,
    lint_enabled:      bool,
}

/// Compute the fixed stats column width from the stat rows and language stats.
/// Returns `(total_width, digit_width)`.
pub fn stats_column_width(data: &PackageData) -> (u16, u16) {
    let max_count = data
        .stats_rows
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let digit_width: u16 = match max_count {
        0..1000 => 3,
        1000..10_000 => 4,
        10_000..100_000 => 5,
        _ => 6,
    };
    let label_width: u16 = 10;
    let total = 1 + 1 + digit_width + 1 + label_width + 1;
    (total, digit_width)
}

pub fn detail_column_scroll_offset(
    focus: PaneFocusState,
    focused_output_line: usize,
    visible_height: u16,
) -> u16 {
    if !matches!(focus, PaneFocusState::Active) || visible_height == 0 {
        return 0;
    }

    let visible_height = usize::from(visible_height);
    let offset = focused_output_line
        .saturating_add(1)
        .saturating_sub(visible_height);
    u16::try_from(offset).unwrap_or(u16::MAX)
}

pub fn package_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_column_inner(frame: &mut Frame, ctx: &PackageRenderCtx<'_>, area: Rect) -> usize {
    let data = ctx.data;
    let fields = ctx.fields;
    let pane = ctx.pane;
    let focus = ctx.focus;
    let styles = ctx.styles;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = package_label_width(fields);
    for (i, field) in fields.iter().enumerate() {
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            focused_output_line = lines.len();
        }
        let label = field.label();
        let selection = pane::selection_state(pane, i, focus);
        let value = if *field == DetailField::Lint {
            lint_display_to_string(&data.lint_display, ctx.animation_elapsed, ctx.lint_enabled)
        } else if *field == DetailField::Ci {
            ci_display_to_string(&data.ci_display)
        } else {
            field.package_value(data)
        };
        let base_label_style = styles.readonly_label;
        let base_value_style = if *field == DetailField::Ci {
            ci_display_style(&data.ci_display)
        } else if *field == DetailField::Lint {
            lint_display_style(&data.lint_display)
        } else {
            Style::default()
        };
        let ls = selection.patch(base_label_style);
        let vs = selection.patch(base_value_style);

        if *field == DetailField::RepoDesc && !value.is_empty() {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 {
                let wrapped = word_wrap(&value, avail);
                for (wi, chunk) in wrapped.iter().enumerate() {
                    if wi == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(prefix.clone(), ls),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(prefix_len)),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    }
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else if matches!(*field, DetailField::Branch) && !value.is_empty() {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 {
                let wrapped = hard_wrap(&value, avail);
                for (wi, chunk) in wrapped.iter().enumerate() {
                    if wi == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(prefix.clone(), ls),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(prefix_len)),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    }
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!(" {label:<label_width$} "), ls),
                Span::styled(value, vs),
            ]));
        }
    }

    let scroll_y = detail_column_scroll_offset(focus, focused_output_line, area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    usize::from(scroll_y)
}

const NO_DESCRIPTION_AVAILABLE: &str = "No description available";

const STATS_TITLE: &str = "Structure";

struct ProjectPanelRender<'a> {
    pkg_data:          &'a PackageData,
    fields:            &'a [DetailField],
    focus:             PaneFocusState,
    styles:            &'a RenderStyles,
    border_style:      Style,
    animation_elapsed: Duration,
    lint_enabled:      bool,
}

#[derive(Clone, Copy)]
struct ProjectPanelAreas {
    lower: Rect,
}

/// Body of `PackagePane::render`. Reads pane state through
/// `pane: &mut PackagePane` and the typed `PaneRenderCtx` instead
/// of the whole `App`.
pub(super) fn render_package_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut PackagePane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let focus_state = pane.focus.state;
    let PaneRenderCtx {
        animation_elapsed,
        config,
        project_list: _,
        selected_project_path: _,
        inflight: _,
        scan: _,
        ci_status_lookup: _,
        keymap_render_inputs: _,
        settings_render_inputs: _,
        inline_error: _,
    } = ctx;
    let lint_enabled = config.current().lint.enabled;

    let Some(pkg_data) = pane.content().cloned() else {
        let title_style = Style::default()
            .fg(TITLE_COLOR)
            .add_modifier(Modifier::BOLD);
        pane.viewport.clear_surface();
        let empty_block = Block::default()
            .borders(Borders::ALL)
            .title(" Details ")
            .title_style(title_style);
        let content = vec![Line::from("  No project selected")];
        let detail = Paragraph::new(content).block(empty_block);
        frame.render_widget(detail, area);
        return;
    };

    let fields = panes::package_fields_from_data(&pkg_data);
    pane.viewport.set_len(fields.len());
    let border_style = if matches!(focus_state, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let title = format!(" {} - {} ", pkg_data.package_title, pkg_data.title_name);
    let project_block = styles
        .chrome
        .with_inactive_border(border_style)
        .block(title, matches!(focus_state, PaneFocusState::Active));
    let project_inner = project_block.inner(area);
    frame.render_widget(project_block, area);

    let context = ProjectPanelRender {
        pkg_data: &pkg_data,
        fields: &fields,
        focus: focus_state,
        styles,
        border_style,
        animation_elapsed: *animation_elapsed,
        lint_enabled,
    };
    let areas = render_project_description_section(frame, &context, area, project_inner);
    {
        let viewport = &mut pane.viewport;
        viewport.set_content_area(areas.lower);
        viewport.set_viewport_rows(usize::from(areas.lower.height));
    }

    let scroll_offset = render_project_metadata(frame, &pane.viewport, &context, areas.lower);
    pane.viewport.set_scroll_offset(scroll_offset);
    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(LABEL_COLOR),
    );
}

fn render_project_description_section(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    project_inner: Rect,
) -> ProjectPanelAreas {
    let lower_metadata_height = context.fields.len().max(context.pkg_data.stats_rows.len());
    let reserved_lower_height = u16::try_from(lower_metadata_height).unwrap_or(u16::MAX);
    let reserved_separator_height = u16::from(project_inner.height > reserved_lower_height);
    let description_max_height = project_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));
    let description_padding = u16::from(project_inner.width > 2);
    let description_width = project_inner
        .width
        .saturating_sub(description_padding.saturating_mul(2));
    let description_lines = description_lines(
        context.pkg_data.description.as_deref(),
        description_width,
        description_max_height,
    );
    let description_height = u16::try_from(description_lines.len()).unwrap_or(u16::MAX);
    let description_area = Rect {
        x: project_inner.x.saturating_add(description_padding),
        width: description_width,
        height: description_height,
        ..project_inner
    };
    frame.render_widget(Paragraph::new(description_lines), description_area);

    let separator_height = u16::from(
        description_height > 0
            && description_area.y.saturating_add(description_height) < project_inner.bottom(),
    );
    let lower_y = description_area
        .y
        .saturating_add(description_height)
        .saturating_add(separator_height);
    let lower_area = Rect {
        x:      project_inner.x,
        y:      lower_y,
        width:  project_inner.width,
        height: project_inner.bottom().saturating_sub(lower_y),
    };
    let stats_connector_x = project_stats_connector_x(context.pkg_data, lower_area);
    if separator_height > 0 {
        let rule_area = Rect {
            x:      area.x,
            y:      description_area.y.saturating_add(description_height),
            width:  area.width,
            height: 1,
        };
        let title = stats_connector_x.map(|_| RuleTitle {
            text:  STATS_TITLE,
            style: context
                .styles
                .chrome
                .title_style(matches!(context.focus, PaneFocusState::Active)),
        });
        pane::render_horizontal_rule(
            frame,
            rule_area,
            context.border_style,
            title,
            stats_connector_x,
        );
    }
    if let Some(connector_x) = stats_connector_x {
        let first_inner_x = area.x.saturating_add(1);
        let last_inner_x = area.right().saturating_sub(2);
        if connector_x >= first_inner_x
            && connector_x <= last_inner_x
            && area.width >= 3
            && area.height > 0
        {
            pane::render_rules(
                frame,
                &[PaneRule::Symbol {
                    area:  Rect {
                        x:      connector_x,
                        y:      area.bottom().saturating_sub(1),
                        width:  1,
                        height: 1,
                    },
                    glyph: '┴',
                }],
                context.border_style,
            );
        }
    }

    ProjectPanelAreas { lower: lower_area }
}

fn render_project_metadata(
    frame: &mut Frame,
    pane: &Viewport,
    context: &ProjectPanelRender<'_>,
    lower_area: Rect,
) -> usize {
    let col_ctx = PackageRenderCtx {
        data: context.pkg_data,
        fields: context.fields,
        pane,
        focus: context.focus,
        styles: context.styles,
        animation_elapsed: context.animation_elapsed,
        lint_enabled: context.lint_enabled,
    };
    let has_stats = !context.pkg_data.stats_rows.is_empty();
    if has_stats {
        let (stats_width, digit_width) = stats_column_width(context.pkg_data);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(lower_area);

        let scroll_offset = render_column_inner(frame, &col_ctx, sub_cols[0]);
        render_stats_column(
            frame,
            context.pkg_data,
            sub_cols[1],
            digit_width,
            context.border_style,
        );
        scroll_offset
    } else {
        render_column_inner(frame, &col_ctx, lower_area)
    }
}

fn project_stats_connector_x(data: &PackageData, lower_area: Rect) -> Option<u16> {
    if data.stats_rows.is_empty() {
        return None;
    }

    let (stats_width, _) = stats_column_width(data);
    let sub_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
        .split(lower_area);
    sub_cols.get(1).map(|area| area.x)
}

fn render_stats_column(
    frame: &mut Frame,
    data: &PackageData,
    area: Rect,
    digit_width: u16,
    border_style: Style,
) {
    pane::render_rules(
        frame,
        &[PaneRule::Vertical {
            area: Rect {
                x:      area.x,
                y:      area.y,
                width:  1,
                height: area.height,
            },
        }],
        border_style,
    );
    let stats_inner = Rect {
        x:      area.x.saturating_add(1),
        y:      area.y,
        width:  area.width.saturating_sub(1),
        height: area.height,
    };

    let stat_label_style = Style::default().fg(LABEL_COLOR);
    let stat_num_style = Style::default().fg(TITLE_COLOR);
    let dw = digit_width as usize;
    let stat_lines: Vec<Line<'_>> = data
        .stats_rows
        .iter()
        .map(|(label, count)| {
            Line::from(vec![
                Span::styled(format!(" {count:>dw$} "), stat_num_style),
                Span::styled(*label, stat_label_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(stat_lines), stats_inner);
}

pub fn description_lines(
    description: Option<&str>,
    width: u16,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_width = usize::from(width);
    let max_height = usize::from(max_height);
    if max_width == 0 || max_height == 0 {
        return Vec::new();
    }

    let (description, style) = description
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map_or_else(
            || (NO_DESCRIPTION_AVAILABLE, Style::default().fg(LABEL_COLOR)),
            |description| (description, Style::default()),
        );

    let wrapped = word_wrap(description, max_width);
    let overflowed = wrapped.len() > max_height;
    let mut visible = wrapped.into_iter().take(max_height).collect::<Vec<_>>();
    if overflowed && let Some(last) = visible.last_mut() {
        let with_ellipsis = format!("{last}\u{2026}");
        *last = render::truncate_with_ellipsis(&with_ellipsis, max_width, "\u{2026}");
    }

    visible
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

/// Style for the Lint row in the Package detail pane, derived
/// from the typed [`LintDisplay`].
fn lint_display_style(display: &super::LintDisplay) -> Style {
    use super::LintDisplay;
    use crate::lint::LintStatus;

    match display {
        LintDisplay::NotRust | LintDisplay::NoRuns => Style::default().fg(INACTIVE_BORDER_COLOR),
        LintDisplay::Runs { status, .. } => match status {
            LintStatus::Passed(_) => Style::default().fg(SUCCESS_COLOR),
            LintStatus::Failed(_) => Style::default().fg(ERROR_COLOR),
            LintStatus::Running(_) | LintStatus::Stale => Style::default().fg(ACCENT_COLOR),
            LintStatus::NoLog => Style::default(),
        },
    }
}

/// Render a typed [`LintDisplay`] to the string shown in the
/// Package detail row. The icon is framed at render time using
/// the current animation tick (the typed `LintDisplay` carries
/// an unframed `LintStatus` so the icon stays in sync with the
/// spinner animation).
fn lint_display_to_string(
    display: &super::LintDisplay,
    animation_elapsed: Duration,
    lint_enabled: bool,
) -> String {
    use super::LintDisplay;

    match display {
        LintDisplay::NotRust => "No lint runs — not a Rust project".to_string(),
        LintDisplay::NoRuns => "No lint runs".to_string(),
        LintDisplay::Runs { count, status } => {
            let icon = if lint_enabled {
                integration::lint_icon_for(status.kind()).frame_at(animation_elapsed)
            } else {
                crate::constants::LINT_NO_LOG
            };
            format!("{icon} {count}")
        },
    }
}

/// Style for the Ci row in the Package detail pane, derived
/// from the typed [`CiDisplay`].
fn ci_display_style(display: &super::CiDisplay) -> Style {
    use super::CiDisplay;

    match display {
        CiDisplay::NoWorkflow | CiDisplay::UnpublishedBranch | CiDisplay::NoRuns => {
            Style::default().fg(INACTIVE_BORDER_COLOR)
        },
        CiDisplay::Runs {
            ci_status: conclusion,
            ..
        } => render::conclusion_style(*conclusion),
    }
}

/// Render a typed [`CiDisplay`] to the string shown in the
/// Package detail row. The conclusion icon is read from
/// `CiStatus::icon()` at render time, in parallel with
/// `lint_display_to_string`.
fn ci_display_to_string(display: &super::CiDisplay) -> String {
    use super::CiDisplay;

    match display {
        CiDisplay::NoWorkflow => "No CI workflow configured".to_string(),
        CiDisplay::UnpublishedBranch => "unpublished branch".to_string(),
        CiDisplay::NoRuns => "No CI runs".to_string(),
        CiDisplay::Runs {
            ci_status: conclusion,
            local,
            github_total,
        } => {
            let icon = conclusion.map_or_else(String::new, |c| c.icon().to_string());
            let count_label = if *github_total > 0 {
                format!("local {local} / github {github_total}")
            } else if *local > 0 {
                format!("{local}")
            } else {
                String::new()
            };
            match (icon.is_empty(), count_label.is_empty()) {
                (true, true) => "No CI runs".to_string(),
                (true, false) => count_label,
                (false, true) => icon,
                (false, false) => format!("{icon} {count_label}"),
            }
        },
    }
}

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            if word.len() > max_width {
                result.push(word.to_string());
            } else {
                current_line.push_str(word);
            }
        } else if current_line.len() + 1 + word.len() > max_width {
            result.push(current_line);
            current_line = word.to_string();
        } else {
            current_line.push(' ');
            current_line.push_str(word);
        }
    }
    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

pub(super) fn hard_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_width {
        result.push(remaining[..max_width].to_string());
        remaining = &remaining[max_width..];
    }
    result.push(remaining.to_string());
    result
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use std::time::Duration;

    use chrono::DateTime;
    use tui_pane::ACTIVITY_SPINNER;

    use super::lint_display_to_string;
    use crate::lint::LintStatus;
    use crate::tui::panes::LintDisplay;

    #[test]
    fn package_lint_row_uses_framework_activity_spinner() {
        let timestamp =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let elapsed = Duration::from_millis(100);
        let display = LintDisplay::Runs {
            count:  3,
            status: LintStatus::Running(timestamp),
        };

        assert_eq!(
            lint_display_to_string(&display, elapsed, true),
            format!("{} 3", ACTIVITY_SPINNER.frame_at(elapsed))
        );
    }
}
