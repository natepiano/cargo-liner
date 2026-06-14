use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::ACTIVITY_SPINNER;
use tui_pane::PaneFocusState;
use tui_pane::PaneRule;
use tui_pane::PaneSelectionState;
use tui_pane::PaneTitleCount;
use tui_pane::RuleTitle;
use tui_pane::Viewport;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::inactive_border_color;
use tui_pane::inactive_title_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::GitPane;
use crate::constants::GIT_LOCAL;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::project::HeadState;
use crate::project::PullRequestCompleteness;
use crate::tui::app::AvailabilityStatus;
use crate::tui::panes;
use crate::tui::panes::DescriptionBlock;
use crate::tui::panes::DetailField;
use crate::tui::panes::EmptyDescriptionBehavior;
use crate::tui::panes::GitData;
#[cfg(test)]
use crate::tui::panes::PullRequestPolling;
use crate::tui::panes::PullRequestRow;
use crate::tui::panes::PullRequestSection;
use crate::tui::panes::PullRequestSectionState;
use crate::tui::panes::RemoteRow;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::panes::WorktreeInfo;
use crate::tui::panes::constants::BRANCH_HEADER;
use crate::tui::panes::constants::FIT_TEXT_ELLIPSIS;
use crate::tui::panes::constants::MIN_FLEX_COL;
use crate::tui::panes::constants::PULL_REQUEST_BRANCH_HEADER;
use crate::tui::panes::constants::PULL_REQUEST_MIN_TITLE_WIDTH;
use crate::tui::panes::constants::PULL_REQUEST_NUMBER_HEADER;
use crate::tui::panes::constants::PULL_REQUEST_STATUS_HEADER;
use crate::tui::panes::constants::PULL_REQUEST_TITLE_HEADER;
use crate::tui::panes::constants::REMOTE_ICON_COL;
use crate::tui::panes::constants::REMOTES_NAME_HEADER;
use crate::tui::panes::constants::REMOTES_URL_HEADER;
use crate::tui::panes::constants::SYNC_HEADER;
use crate::tui::panes::constants::TRACKED_HEADER;
use crate::tui::panes::constants::WORKTREES_NAME_HEADER;
use crate::tui::panes::package;
use crate::tui::panes::support;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::theme_roles;

struct GitRenderCtx<'a> {
    data:              &'a GitData,
    fields:            &'a [DetailField],
    pane:              &'a Viewport,
    focus:             PaneFocusState,
    styles:            &'a RenderStyles,
    row_offset:        usize,
    animation_elapsed: Duration,
}

/// Which section a flat `pos()` index lives in, plus the offset within
/// that section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Flat(usize),
    PullRequest(usize),
    Remote(usize),
    Worktree(usize),
}

const fn section_for_pos(
    pos: usize,
    flat_len: usize,
    pull_requests_len: usize,
    remotes_len: usize,
    worktrees_len: usize,
) -> Option<Section> {
    if pos < flat_len {
        Some(Section::Flat(pos))
    } else if pos < flat_len + pull_requests_len {
        Some(Section::PullRequest(pos - flat_len))
    } else if pos < flat_len + pull_requests_len + remotes_len {
        Some(Section::Remote(pos - flat_len - pull_requests_len))
    } else if pos < flat_len + pull_requests_len + remotes_len + worktrees_len {
        Some(Section::Worktree(
            pos - flat_len - pull_requests_len - remotes_len,
        ))
    } else {
        None
    }
}

pub fn git_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

/// A section separator to render as an overlay rule after the paragraph
/// (so its `├`/`┤` endcaps can overlap the outer pane's vertical borders).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SectionFocus {
    Focused,
    Unfocused,
}

impl SectionFocus {
    const fn from_focused(focused: bool) -> Self {
        if focused {
            Self::Focused
        } else {
            Self::Unfocused
        }
    }

    const fn is_focused(self) -> bool { matches!(self, Self::Focused) }
}

struct SectionRule {
    inner_y:       usize,
    title:         String,
    section_focus: SectionFocus,
}

/// Result of building the Git pane paragraph, used to register row
/// hitboxes at the correct screen rows after rendering.
struct GitRenderLayout {
    scroll_offset: usize,
    row_spans:     Vec<GitVisualRowSpan>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct GitVisualRowSpan {
    pub start_y: usize,
    pub height:  usize,
}

/// Mutable accumulators threaded through the per-section builders.
struct SectionAccum<'a> {
    lines:               &'a mut Vec<Line<'static>>,
    focused_output_line: &'a mut usize,
    section_rules:       &'a mut Vec<SectionRule>,
    row_spans:           &'a mut Vec<GitVisualRowSpan>,
}

fn render_git_column_inner(
    frame: &mut Frame,
    ctx: &GitRenderCtx<'_>,
    outer_area: Rect,
    inner_area: Rect,
) -> GitRenderLayout {
    let flat_len = ctx.fields.len();
    let pull_requests_len = ctx.data.pull_requests.rows.len();
    let remotes_len = ctx.data.remotes.len();
    let worktrees_len = ctx.data.worktrees.len();
    let current_section = if matches!(ctx.focus, PaneFocusState::Active) {
        ctx.pane.pos().checked_sub(ctx.row_offset).and_then(|pos| {
            section_for_pos(pos, flat_len, pull_requests_len, remotes_len, worktrees_len)
        })
    } else {
        None
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let mut section_rules: Vec<SectionRule> = Vec::new();
    let mut row_spans: Vec<GitVisualRowSpan> =
        Vec::with_capacity(flat_len + pull_requests_len + remotes_len + worktrees_len);

    let mut accum = SectionAccum {
        lines:               &mut lines,
        focused_output_line: &mut focused_output_line,
        section_rules:       &mut section_rules,
        row_spans:           &mut row_spans,
    };

    render_flat_fields(
        &mut accum,
        &RenderFlatArgs {
            data:        ctx.data,
            fields:      ctx.fields,
            pane:        ctx.pane,
            focus:       ctx.focus,
            styles:      ctx.styles,
            area_width:  inner_area.width,
            label_width: git_label_width(ctx.fields),
            row_offset:  ctx.row_offset,
        },
    );
    append_pull_requests_section(&mut accum, ctx, flat_len, current_section, inner_area.width);
    // Remotes and Worktrees share one column layout so their Branch /
    // Tracked / Sync columns line up vertically across both sections.
    let sync_layout = sync_col_layout(
        &ctx.data.remotes,
        &ctx.data.worktrees,
        usize::from(inner_area.width),
    );
    append_remotes_section(
        &mut accum,
        ctx,
        flat_len,
        pull_requests_len,
        current_section,
        &sync_layout,
    );
    append_worktrees_section(
        &mut accum,
        ctx,
        flat_len,
        pull_requests_len,
        remotes_len,
        current_section,
        &sync_layout,
    );

    let scroll_y = package::detail_column_scroll_offset(
        ctx.focus,
        focused_output_line,
        inner_area.height,
        lines.len(),
    );
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), inner_area);
    render_section_overlays(
        frame,
        &section_rules,
        scroll_y,
        outer_area,
        inner_area,
        ctx.focus,
        ctx.styles,
    );
    GitRenderLayout {
        scroll_offset: usize::from(scroll_y),
        row_spans,
    }
}

fn append_remotes_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    pull_requests_len: usize,
    current_section: Option<Section>,
    layout: &SyncColLayout,
) {
    if ctx.data.remotes.is_empty() {
        return;
    }
    let focused = matches!(current_section, Some(Section::Remote(_)));
    let cursor = match current_section {
        Some(Section::Remote(i)) => Some(i),
        _ => None,
    };
    let title = section_title_text("Remotes", ctx.data.remotes.len(), cursor);
    // Blank spacer row + placeholder row for the rule overlay.
    accum.lines.push(Line::from(Span::raw(String::new())));
    accum.section_rules.push(SectionRule {
        inner_y: accum.lines.len(),
        title,
        section_focus: SectionFocus::from_focused(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    render_remote_header(accum.lines, layout, focused);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, remote) in ctx.data.remotes.iter().enumerate() {
        let row_index = ctx.row_offset + flat_len + pull_requests_len + i;
        accum.row_spans.push(GitVisualRowSpan {
            start_y: accum.lines.len(),
            height:  1,
        });
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = tui_pane::selection_state(ctx.pane, row_index, ctx.focus);
        accum.lines.push(remote_row_line(remote, layout, selection));
    }
}

fn append_pull_requests_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    current_section: Option<Section>,
    area_width: u16,
) {
    let section = &ctx.data.pull_requests;
    if section.rows.is_empty()
        && matches!(section.state, PullRequestSectionState::HiddenConfirmedEmpty)
    {
        return;
    }
    let focused = matches!(current_section, Some(Section::PullRequest(_)));
    let cursor = match current_section {
        Some(Section::PullRequest(i)) => Some(i),
        _ => None,
    };
    let mut title = section_title_text("Pull Requests", section.rows.len(), cursor);
    if matches!(
        section.completeness,
        Some(PullRequestCompleteness::Truncated { .. })
    ) {
        title.push_str(" +");
    }
    accum.lines.push(Line::from(Span::raw(String::new())));
    accum.section_rules.push(SectionRule {
        inner_y: accum.lines.len(),
        title,
        section_focus: SectionFocus::from_focused(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));

    if section.rows.is_empty() {
        accum.lines.push(Line::from(Span::styled(
            format!(" {}", pull_request_status_text(section)),
            Style::default().fg(inactive_title_color()),
        )));
        return;
    }

    let col_widths = pull_request_col_widths(&section.rows, area_width, ctx.animation_elapsed);
    render_pull_request_header(accum.lines, &col_widths, focused);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, row) in section.rows.iter().enumerate() {
        let row_index = ctx.row_offset + flat_len + i;
        let start_y = accum.lines.len();
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = start_y;
        }
        let selection = tui_pane::selection_state(ctx.pane, row_index, ctx.focus);
        accum.lines.push(pull_request_row_line(
            row,
            &col_widths,
            selection,
            ctx.animation_elapsed,
        ));
        accum
            .row_spans
            .push(GitVisualRowSpan { start_y, height: 1 });
    }
    if matches!(
        section.completeness,
        Some(PullRequestCompleteness::Truncated { .. })
    ) {
        accum.lines.push(Line::from(Span::styled(
            " more pull requests not shown".to_string(),
            Style::default().fg(warning_color()),
        )));
    }
}

fn pull_request_status_text(section: &PullRequestSection) -> String {
    match section.state {
        PullRequestSectionState::Loading => "loading pull requests".to_string(),
        PullRequestSectionState::Unavailable => section.unavailable_reason.map_or_else(
            || "pull requests unavailable".to_string(),
            |reason| format!("pull requests unavailable: {}", reason.label()),
        ),
        PullRequestSectionState::Stale => section.unavailable_reason.map_or_else(
            || "stale pull requests".to_string(),
            |reason| {
                let fetched = section
                    .fetched_at
                    .as_deref()
                    .map(|ts| format!("; fetched {ts}"))
                    .unwrap_or_default();
                format!("stale pull requests: {}{fetched}", reason.label())
            },
        ),
        PullRequestSectionState::Loaded | PullRequestSectionState::HiddenConfirmedEmpty => {
            String::new()
        },
    }
}

struct PullRequestColWidths {
    number: usize,
    status: usize,
    branch: usize,
    title:  usize,
}

fn pull_request_col_widths(
    rows: &[PullRequestRow],
    area_width: u16,
    animation_elapsed: Duration,
) -> PullRequestColWidths {
    let number = rows
        .iter()
        .map(|row| format!("#{}", row.number).width())
        .max()
        .unwrap_or(0)
        .max(PULL_REQUEST_NUMBER_HEADER.width());
    let status = rows
        .iter()
        .map(|row| pull_request_state_text(row, animation_elapsed).width())
        .max()
        .unwrap_or(0)
        .max(PULL_REQUEST_STATUS_HEADER.width());
    let branch_preferred = rows
        .iter()
        .map(|row| row.branch.width())
        .max()
        .unwrap_or(0)
        .max(PULL_REQUEST_BRANCH_HEADER.width());
    let title_preferred = rows
        .iter()
        .map(|row| row.title.width())
        .max()
        .unwrap_or(0)
        .max(PULL_REQUEST_TITLE_HEADER.width());
    let fixed_width = 1 + number + 2 + status + 2 + 2;
    let branch_title_width = usize::from(area_width).saturating_sub(fixed_width);
    let branch = if branch_title_width >= branch_preferred + title_preferred {
        branch_preferred
    } else if branch_title_width > PULL_REQUEST_MIN_TITLE_WIDTH {
        branch_preferred.min(branch_title_width - PULL_REQUEST_MIN_TITLE_WIDTH)
    } else {
        branch_preferred.min(branch_title_width)
    };
    let title = if branch_title_width >= branch_preferred + title_preferred {
        title_preferred
    } else {
        branch_title_width.saturating_sub(branch)
    };
    PullRequestColWidths {
        number,
        status,
        branch,
        title,
    }
}

fn column_header_style(focused: bool) -> Style {
    let style = Style::default().fg(theme_roles::column_header_color());
    if focused {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn render_pull_request_header(
    lines: &mut Vec<Line<'static>>,
    widths: &PullRequestColWidths,
    focused: bool,
) {
    let style = column_header_style(focused);
    let text = format!(
        " {:<number$}  {:<status$}  {:<branch$}  {}",
        fit_text(PULL_REQUEST_NUMBER_HEADER, widths.number),
        fit_text(PULL_REQUEST_STATUS_HEADER, widths.status),
        fit_text(PULL_REQUEST_BRANCH_HEADER, widths.branch),
        fit_text(PULL_REQUEST_TITLE_HEADER, widths.title),
        number = widths.number,
        status = widths.status,
        branch = widths.branch,
    );
    lines.push(Line::from(Span::styled(text, style)));
}

fn pull_request_row_line(
    row: &PullRequestRow,
    widths: &PullRequestColWidths,
    selection: PaneSelectionState,
    animation_elapsed: Duration,
) -> Line<'static> {
    let style = selection.patch(Style::default().fg(inactive_title_color()));
    if !row.polling.is_polling() {
        let state = pull_request_state_text(row, animation_elapsed);
        let text = format!(
            " {:<number$}  {:<status$}  {:<branch$}  {}",
            fit_text(&format!("#{}", row.number), widths.number),
            fit_text(&state, widths.status),
            fit_text(&row.branch, widths.branch),
            fit_text(&row.title, widths.title),
            number = widths.number,
            status = widths.status,
            branch = widths.branch,
        );
        return Line::from(Span::styled(text, style));
    }

    let spinner = ACTIVITY_SPINNER.frame_at(animation_elapsed);
    let label_width = widths.status.saturating_sub(spinner.width() + 1);
    let state_label = fit_text(row.state_label, label_width);
    let state_text = format!("{state_label} {spinner}");
    let state_padding = " ".repeat(widths.status.saturating_sub(state_text.width()));
    Line::from(vec![
        Span::styled(
            format!(
                " {:<number$}  {} ",
                fit_text(&format!("#{}", row.number), widths.number),
                state_label,
                number = widths.number,
            ),
            style,
        ),
        Span::styled(
            spinner.to_string(),
            selection.patch(Style::default().fg(accent_color())),
        ),
        Span::styled(
            format!(
                "{state_padding}  {:<branch$}  {}",
                fit_text(&row.branch, widths.branch),
                fit_text(&row.title, widths.title),
                branch = widths.branch,
            ),
            style,
        ),
    ])
}

fn pull_request_state_text(row: &PullRequestRow, animation_elapsed: Duration) -> String {
    if row.polling.is_polling() {
        format!(
            "{} {}",
            row.state_label,
            ACTIVITY_SPINNER.frame_at(animation_elapsed)
        )
    } else {
        row.state_label.to_string()
    }
}

fn fit_text(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width <= FIT_TEXT_ELLIPSIS.len() {
        return ".".repeat(max_width);
    }
    let mut out = String::new();
    let target = max_width - FIT_TEXT_ELLIPSIS.len();
    for ch in text.chars() {
        if out.width() + ch.width().unwrap_or(0) > target {
            break;
        }
        out.push(ch);
    }
    out.push_str(FIT_TEXT_ELLIPSIS);
    out
}

fn append_worktrees_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    pull_requests_len: usize,
    remotes_len: usize,
    current_section: Option<Section>,
    layout: &SyncColLayout,
) {
    if ctx.data.worktrees.is_empty() {
        return;
    }
    let focused = matches!(current_section, Some(Section::Worktree(_)));
    let cursor = match current_section {
        Some(Section::Worktree(i)) => Some(i),
        _ => None,
    };
    let title = section_title_text("Worktrees", ctx.data.worktrees.len(), cursor);
    accum.lines.push(Line::from(Span::raw(String::new())));
    accum.section_rules.push(SectionRule {
        inner_y: accum.lines.len(),
        title,
        section_focus: SectionFocus::from_focused(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    render_worktree_header(accum.lines, layout, focused);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, wt) in ctx.data.worktrees.iter().enumerate() {
        let row_index = ctx.row_offset + flat_len + pull_requests_len + remotes_len + i;
        accum.row_spans.push(GitVisualRowSpan {
            start_y: accum.lines.len(),
            height:  1,
        });
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = tui_pane::selection_state(ctx.pane, row_index, ctx.focus);
        accum.lines.push(worktree_row_line(wt, layout, selection));
    }
}

fn section_title_text(label: &str, len: usize, cursor: Option<usize>) -> String {
    format!("{label} {}", PaneTitleCount::Single { len, cursor }.body())
}

fn render_section_overlays(
    frame: &mut Frame,
    section_rules: &[SectionRule],
    scroll_y: u16,
    outer_area: Rect,
    inner_area: Rect,
    focus: PaneFocusState,
    styles: &RenderStyles,
) {
    // Match the outer pane chrome — rule segments use the same border style
    // as the pane it sits inside, so focused panes stay yellow end-to-end and
    // unfocused panes stay at the default theme weight.
    let rule_style = if matches!(focus, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    for rule in section_rules {
        let relative_y = u16::try_from(rule.inner_y).unwrap_or(u16::MAX);
        if relative_y < scroll_y {
            continue;
        }
        let abs_y = inner_area.y.saturating_add(relative_y - scroll_y);
        if abs_y < inner_area.y || abs_y >= inner_area.y.saturating_add(inner_area.height) {
            continue;
        }
        let title_color = if rule.section_focus.is_focused() {
            title_color()
        } else {
            inactive_title_color()
        };
        let mut title_style = Style::default().fg(title_color);
        if rule.section_focus.is_focused() {
            title_style = title_style.add_modifier(Modifier::BOLD);
        }
        tui_pane::render_horizontal_rule(
            frame,
            Rect {
                x:      outer_area.x,
                y:      abs_y,
                width:  outer_area.width,
                height: 1,
            },
            rule_style,
            Some(RuleTitle {
                text:  &rule.title,
                style: title_style,
            }),
            None,
        );
    }
}

struct RenderFlatArgs<'a> {
    data:        &'a GitData,
    fields:      &'a [DetailField],
    pane:        &'a Viewport,
    focus:       PaneFocusState,
    styles:      &'a RenderStyles,
    area_width:  u16,
    label_width: usize,
    row_offset:  usize,
}

/// Compute the displayed value string for a flat git-pane row,
/// including row-specific decorations (local-only suffix on `Branch`,
/// `(github unreachable)` / `(github rate-limited)` /
/// `(unauthenticated — gh auth login)` suffix on rate-limit rows).
/// Also returns the "github unreachable" placeholder text for an empty
/// Stars row when GitHub is down — the parallel of the crates.io
/// unreachable placeholder on the Package pane.
fn build_field_value(data: &GitData, field: DetailField, is_rate_limit_row: bool) -> String {
    if field == DetailField::Head {
        let raw = field.git_value(data);
        return if data.is_local() && !raw.is_empty() {
            format!("{raw} ({GIT_LOCAL} local)")
        } else {
            raw
        };
    }
    let raw = field.git_value(data);
    if is_rate_limit_row && let Some(suffix) = github_status_suffix(data.github_status) {
        return if raw.is_empty() {
            format!("({suffix})")
        } else {
            format!("{raw} ({suffix})")
        };
    }
    if field == DetailField::Stars
        && raw.is_empty()
        && panes::github_stars_is_unreachable_placeholder(data)
    {
        return "github unreachable".to_string();
    }
    raw
}

const fn github_status_suffix(status: AvailabilityStatus) -> Option<&'static str> {
    match status {
        AvailabilityStatus::Reachable => None,
        AvailabilityStatus::Unreachable => Some("github unreachable"),
        AvailabilityStatus::RateLimited => Some("github rate-limited"),
        AvailabilityStatus::Unauthenticated => Some("unauthenticated — gh auth login"),
        AvailabilityStatus::NotInstalled => Some("gh not installed"),
    }
}

fn render_flat_fields(accum: &mut SectionAccum<'_>, args: &RenderFlatArgs<'_>) {
    let RenderFlatArgs {
        data,
        fields,
        pane,
        focus,
        styles,
        area_width,
        label_width,
        row_offset,
    } = *args;
    for (i, field) in fields.iter().enumerate() {
        let row_index = row_offset + i;
        accum.row_spans.push(GitVisualRowSpan {
            start_y: accum.lines.len(),
            height:  1,
        });
        if matches!(focus, PaneFocusState::Active) && row_index == pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let label = field.label();
        let is_rate_limit_row = matches!(
            *field,
            DetailField::RateLimitCore | DetailField::RateLimitGraphQl
        );
        let value = build_field_value(data, *field, is_rate_limit_row);
        let selection = tui_pane::selection_state(pane, row_index, focus);
        let base_value_style =
            if matches!(*field, DetailField::VsLocal) && value.starts_with(IN_SYNC) {
                Style::default().fg(success_color())
            } else if matches!(*field, DetailField::VsLocal) && value == NO_REMOTE_SYNC {
                Style::default().fg(inactive_border_color())
            } else if *field == DetailField::WorktreeError {
                Style::default().fg(text_default()).bg(error_color())
            } else if is_rate_limit_row && data.github_status.is_unauthenticated() {
                // Unauthenticated is actionable, not an outage — warn (yellow)
                // rather than error (red) like the unreachable / rate-limited rows.
                Style::default().fg(warning_color())
            } else if is_rate_limit_row && !data.github_status.is_available() {
                Style::default().fg(error_color())
            } else if *field == DetailField::Stars
                && panes::github_stars_is_unreachable_placeholder(data)
            {
                Style::default().fg(warning_color())
            } else {
                Style::default()
            };
        let ls = selection.patch(styles.readonly_label);
        let vs = selection.patch(base_value_style);
        if matches!(
            *field,
            DetailField::Head | DetailField::WorktreeError | DetailField::Bisect
        ) && !value.is_empty()
        {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area_width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.width() > avail {
                let wrapped = if matches!(*field, DetailField::WorktreeError | DetailField::Bisect)
                {
                    support::word_wrap(&value, avail)
                } else {
                    support::hard_wrap(&value, avail)
                };
                for (wi, chunk) in wrapped.iter().enumerate() {
                    if wi == 0 {
                        accum.lines.push(Line::from(vec![
                            Span::styled(prefix.clone(), ls),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    } else {
                        accum.lines.push(Line::from(vec![
                            Span::raw(" ".repeat(prefix_len)),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    }
                }
            } else {
                accum.lines.push(Line::from(vec![
                    Span::styled(prefix, ls),
                    Span::styled(value, vs),
                ]));
            }
        } else {
            let mut spans = vec![Span::styled(format!(" {label:<label_width$} "), ls)];
            if is_rate_limit_row
                && data.github_status.is_available()
                && let Some(idx) = value.find(" resets ")
            {
                let (base, reset) = value.split_at(idx);
                let reset_style = selection.patch(Style::default().fg(inactive_border_color()));
                spans.push(Span::styled(base.to_string(), vs));
                spans.push(Span::styled(reset.to_string(), reset_style));
            } else {
                spans.push(Span::styled(value, vs));
            }
            accum.lines.push(Line::from(spans));
        }
    }
}

// ── Remotes table ────────────────────────────────────────────────────

// The trailing Branch/Tracked/Sync trio is shared by both tables so the
// columns line up vertically across the Remotes and Worktrees sections.

/// Column layout shared between the Remotes and Worktrees tables. The
/// leading columns differ (Remote/URL vs Name), but the trailing
/// Branch/Tracked/Sync trio uses identical widths and starts at the same
/// `lead` display column in both tables, so the two sections line up.
struct SyncColLayout {
    remote_name:   usize,
    remote_url:    usize,
    worktree_name: usize,
    /// Display column where the Branch column starts in both tables.
    lead:          usize,
    branch:        usize,
    tracked:       usize,
    sync:          usize,
}

/// The worktree's own checked-out branch — the source side of its delta.
fn worktree_branch_text(wt: &WorktreeInfo) -> &str { wt.branch.as_deref().unwrap_or("-") }

/// What the worktree is measured against (the primary's branch), or the
/// no-comparison rune for the primary entry itself.
fn worktree_tracked_text(wt: &WorktreeInfo) -> &str {
    wt.tracked.as_deref().unwrap_or(NO_REMOTE_SYNC)
}

fn sync_col_layout(
    remotes: &[RemoteRow],
    worktrees: &[WorktreeInfo],
    available: usize,
) -> SyncColLayout {
    let remote_name = remotes
        .iter()
        .map(|r| r.name.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_NAME_HEADER.width());
    let remote_url = remotes
        .iter()
        .map(|r| r.display_url.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_URL_HEADER.width());
    let worktree_name = worktrees
        .iter()
        .map(|w| w.name.width())
        .max()
        .unwrap_or(0)
        .max(WORKTREES_NAME_HEADER.width());

    let branch = remotes
        .iter()
        .map(|r| r.branch.width())
        .chain(worktrees.iter().map(|w| worktree_branch_text(w).width()))
        .max()
        .unwrap_or(0)
        .max(BRANCH_HEADER.width());
    let tracked = remotes
        .iter()
        .map(|r| r.tracked_ref.width())
        .chain(worktrees.iter().map(|w| worktree_tracked_text(w).width()))
        .max()
        .unwrap_or(0)
        .max(TRACKED_HEADER.width());
    let sync = remotes
        .iter()
        .map(|r| r.status.width())
        .chain(
            worktrees
                .iter()
                .map(|w| panes::format_ahead_behind(w.ahead_behind).width()),
        )
        .max()
        .unwrap_or(0)
        .max(SYNC_HEADER.width());

    // Fixed leading widths (everything before the flexible column), plus the
    // 2-space gap that precedes the Branch column.
    let remote_fixed = 1 + REMOTE_ICON_COL + remote_name + 2 + 2;
    let worktree_fixed = 1 + 2;

    // Branch starts at `lead`. We keep the trio at full width and let the
    // flexible column (URL / Name) absorb any shortfall, down to MIN_FLEX_COL.
    let natural_lead = remote_lead_or(remotes, remote_fixed + remote_url)
        .max(worktree_lead_or(worktrees, worktree_fixed + worktree_name));
    let min_lead = remote_lead_or(remotes, remote_fixed + MIN_FLEX_COL)
        .max(worktree_lead_or(worktrees, worktree_fixed + MIN_FLEX_COL))
        .min(natural_lead);
    let trio_total = branch + 2 + tracked + 2 + sync;
    let budget_lead = available.saturating_sub(trio_total);
    let lead = budget_lead.clamp(min_lead, natural_lead);

    SyncColLayout {
        remote_name,
        // Clamp each flexible column to what `lead` leaves for it; the render
        // path ellipsis-truncates the content to this width.
        remote_url: remote_url.min(lead.saturating_sub(remote_fixed)),
        worktree_name: worktree_name.min(lead.saturating_sub(worktree_fixed)),
        lead,
        branch,
        tracked,
        sync,
    }
}

/// `lead` for the Remotes table, or 0 when there are no remotes (so an empty
/// table doesn't inflate the shared layout).
const fn remote_lead_or(remotes: &[RemoteRow], lead: usize) -> usize {
    if remotes.is_empty() { 0 } else { lead }
}

const fn worktree_lead_or(worktrees: &[WorktreeInfo], lead: usize) -> usize {
    if worktrees.is_empty() { 0 } else { lead }
}

/// Format the shared Branch/Tracked/Sync trio. `branch` and `tracked` are
/// left-aligned text; `sync` is right-aligned (numeric ahead/behind).
fn sync_trio(layout: &SyncColLayout, branch: &str, tracked: &str, sync: &str) -> String {
    format!(
        "{branch:<bw$}  {tracked:<tw$}  {sync:>sw$}",
        bw = layout.branch,
        tw = layout.tracked,
        sw = layout.sync,
    )
}

fn render_remote_header(lines: &mut Vec<Line<'static>>, layout: &SyncColLayout, focused: bool) {
    let style = column_header_style(focused);
    // Leading: 1 space pad + REMOTE_ICON_COL blank for icon alignment.
    let leading = format!(
        " {:<icon$}{:<name$}  {:<url$}",
        "",
        REMOTES_NAME_HEADER,
        REMOTES_URL_HEADER,
        icon = REMOTE_ICON_COL,
        name = layout.remote_name,
        url = layout.remote_url,
    );
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let trio = sync_trio(layout, BRANCH_HEADER, TRACKED_HEADER, SYNC_HEADER);
    lines.push(Line::from(Span::styled(
        format!("{leading}{pad}{trio}"),
        style,
    )));
}

fn remote_row_line(
    row: &RemoteRow,
    layout: &SyncColLayout,
    selection: PaneSelectionState,
) -> Line<'static> {
    // Icon cell: emoji + trailing spaces to reach REMOTE_ICON_COL width.
    let icon_width = row.icon.width();
    let icon_pad = REMOTE_ICON_COL.saturating_sub(icon_width);
    let icon_cell = format!("{}{}", row.icon, " ".repeat(icon_pad));
    let push_suffix = row
        .push_annotation
        .as_deref()
        .map(|annotation| format!("  {annotation}"))
        .unwrap_or_default();
    // Width already spent by the leading space + icon spans, then the
    // Remote/URL columns, before the Branch column begins.
    let consumed = 1 + REMOTE_ICON_COL + layout.remote_name + 2 + layout.remote_url;
    let pad = " ".repeat(layout.lead.saturating_sub(consumed));
    let trio = sync_trio(layout, &row.branch, &row.tracked_ref, &row.status);
    let url = fit_text(&row.display_url, layout.remote_url);
    let text = format!(
        "{:<name$}  {:<url$}{pad}{trio}{push_suffix}",
        row.name,
        url,
        name = layout.remote_name,
        url = layout.remote_url,
    );
    let data_style = selection.patch(Style::default().fg(inactive_title_color()));
    let icon_style = selection.patch(Style::default());
    Line::from(vec![
        Span::raw(" ".to_string()),
        Span::styled(icon_cell, icon_style),
        Span::styled(text, data_style),
    ])
}

// ── Worktrees table ──────────────────────────────────────────────────

fn render_worktree_header(lines: &mut Vec<Line<'static>>, layout: &SyncColLayout, focused: bool) {
    let style = column_header_style(focused);
    let leading = format!(
        " {:<name$}",
        WORKTREES_NAME_HEADER,
        name = layout.worktree_name
    );
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let trio = sync_trio(layout, BRANCH_HEADER, TRACKED_HEADER, SYNC_HEADER);
    lines.push(Line::from(Span::styled(
        format!("{leading}{pad}{trio}"),
        style,
    )));
}

fn worktree_row_line(
    row: &WorktreeInfo,
    layout: &SyncColLayout,
    selection: PaneSelectionState,
) -> Line<'static> {
    let sync = panes::format_ahead_behind(row.ahead_behind);
    let name = fit_text(&row.name, layout.worktree_name);
    let leading = format!(" {:<name$}", name, name = layout.worktree_name);
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let trio = sync_trio(
        layout,
        worktree_branch_text(row),
        worktree_tracked_text(row),
        &sync,
    );
    let style = selection.patch(Style::default().fg(inactive_title_color()));
    Line::from(Span::styled(format!("{leading}{pad}{trio}"), style))
}

fn git_panel_title(data: &GitData) -> String {
    match data.head.as_ref() {
        Some(HeadState::Branch(name)) if !name.is_empty() => format!(" Git - {name} "),
        Some(HeadState::Detached { short_sha }) => format!(" Git - detached @ {short_sha} "),
        Some(HeadState::Branch(_) | HeadState::Unborn) | None => {
            tui_pane::pane_title("Git", &PaneTitleCount::None)
        },
    }
}

/// Body of `GitPane::render`. Same pattern as
/// `cpu::render_cpu_pane_body`: typed parameters via `ctx`.
pub(super) fn render_git_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut GitPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let Some(git_data) = pane.content().cloned() else {
        pane.viewport.clear_surface();
        pane.clear_row_layout();
        let empty = tui_pane::empty_pane_block(tui_pane::pane_title("Git", &PaneTitleCount::None));
        frame.render_widget(empty, area);
        return;
    };

    let flat_fields = panes::git_fields_from_data(&git_data);
    let description_rows = usize::from(panes::git_has_description_row(&git_data));
    let total_rows = description_rows
        + flat_fields.len()
        + git_data.pull_requests.rows.len()
        + git_data.remotes.len()
        + git_data.worktrees.len();
    if total_rows == 0 && git_data.description.as_deref().is_none_or(str::is_empty) {
        pane.viewport.clear_surface();
        pane.clear_row_layout();
        let empty_git = tui_pane::empty_pane_block(" Not a git repo ");
        frame.render_widget(empty_git, area);
        return;
    }

    pane.viewport.set_len(total_rows);
    // The lower blocks add header/spacing rows the addressable-row count omits,
    // and a multi-line description renders taller than its single cursor row,
    // so the rendered height is not the flat row count. Tell the viewport the
    // rendered height so the scroll pager pages on what actually renders.
    pane.viewport.set_content_height(git_content_height(
        usize::from(ctx.synced_description_height.rows()),
        description_rows > 0,
        git_lower_content_height(&git_data, flat_fields.len()),
    ));
    let pane_focus_state = pane.focus.pane_focus_state;
    let border_style = if matches!(pane_focus_state, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let git_block = styles.chrome.block(
        git_panel_title(&git_data),
        matches!(pane_focus_state, PaneFocusState::Active),
    );
    let git_inner = git_block.inner(area);
    frame.render_widget(git_block, area);

    let about_layout = render_git_about_section(
        frame,
        &GitAboutCtx {
            description: git_data.description.as_deref(),
            flat_fields: &flat_fields,
            data: &git_data,
            outer_area: area,
            git_inner,
            border_style,
            synced_description_height: ctx.synced_description_height,
            pane: &pane.viewport,
            focus: pane_focus_state,
        },
    );

    {
        let viewport = &mut pane.viewport;
        viewport.set_content_area(about_layout.content_area);
        viewport.set_viewport_rows(
            usize::from(about_layout.content_area.height).saturating_add(description_rows),
        );
    }
    let git_ctx = GitRenderCtx {
        data: &git_data,
        fields: &flat_fields,
        pane: &pane.viewport,
        focus: pane_focus_state,
        styles,
        row_offset: description_rows,
        animation_elapsed: ctx.animation_elapsed,
    };
    let layout = render_git_column_inner(frame, &git_ctx, area, about_layout.content_area);
    pane.viewport.set_scroll_offset(layout.scroll_offset);
    pane.set_row_layout(
        about_layout.description_rect,
        about_layout.content_area,
        description_rows,
        layout.row_spans,
    );
    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
    let _ = ctx;
}

/// Inputs for [`render_git_about_section`]. Grouped into a struct
/// so the function stays under the argument-count threshold.
struct GitAboutCtx<'a> {
    description:               Option<&'a str>,
    flat_fields:               &'a [DetailField],
    data:                      &'a GitData,
    outer_area:                Rect,
    git_inner:                 Rect,
    border_style:              Style,
    synced_description_height: SyncedDescriptionHeight,
    pane:                      &'a Viewport,
    focus:                     PaneFocusState,
}

#[derive(Clone, Copy)]
struct GitAboutLayout {
    content_area:     Rect,
    description_rect: Option<Rect>,
}

/// Render the About section (repo description) at the top of the Git panel,
/// separated from the rest of the pane by a horizontal rule with `├`/`┤`
/// endcaps. Returns the area below the separator for the scrolling content.
fn render_git_about_section(frame: &mut Frame, ctx: &GitAboutCtx<'_>) -> GitAboutLayout {
    let git_inner = ctx.git_inner;

    let lower_content_height = git_lower_content_height(ctx.data, ctx.flat_fields.len());
    let reserved_lower_height = u16::from(lower_content_height > 0);
    let reserved_separator_height =
        u16::from(git_inner.height > reserved_lower_height.saturating_add(1));
    let baseline_max = git_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));

    // Build the same DescriptionBlock that `sync_floor` consumed at the
    // top of the frame and let it render — the block owns the wrapped
    // rows so the rendered content can't drift from the height that
    // fed the inter-pane sync.
    let block = DescriptionBlock::for_pane(
        ctx.description,
        ctx.outer_area,
        EmptyDescriptionBehavior::RenderEmpty,
    );
    let description_height = block.render_with_selection(
        frame,
        git_inner,
        ctx.synced_description_height,
        baseline_max,
        tui_pane::selection_state(ctx.pane, 0, ctx.focus),
    );
    if description_height == 0 {
        return GitAboutLayout {
            content_area:     git_inner,
            description_rect: None,
        };
    }

    let separator_y = git_inner.y.saturating_add(description_height);
    let has_room_for_separator = separator_y < git_inner.bottom();
    if !has_room_for_separator {
        return GitAboutLayout {
            content_area:     Rect {
                x:      git_inner.x,
                y:      separator_y,
                width:  git_inner.width,
                height: 0,
            },
            description_rect: Some(Rect {
                x:      git_inner.x,
                y:      git_inner.y,
                width:  git_inner.width,
                height: description_height,
            }),
        };
    }

    tui_pane::render_rules(
        frame,
        &[PaneRule::Horizontal {
            area:        Rect {
                x:      ctx.outer_area.x,
                y:      separator_y,
                width:  ctx.outer_area.width,
                height: 1,
            },
            connector_x: None,
        }],
        ctx.border_style,
    );

    let content_y = separator_y.saturating_add(1);
    GitAboutLayout {
        content_area:     Rect {
            x:      git_inner.x,
            y:      content_y,
            width:  git_inner.width,
            height: git_inner.bottom().saturating_sub(content_y),
        },
        description_rect: Some(Rect {
            x:      git_inner.x,
            y:      git_inner.y,
            width:  git_inner.width,
            height: description_height,
        }),
    }
}

/// Rows the Git pane's lower (scrolling) content occupies, below the About
/// section: the flat field rows plus the pull-request, remotes, and worktrees
/// blocks. Each of the remotes / worktrees blocks adds three header/spacing
/// rows when non-empty. Shared by the render path and the cross-project
/// top-row height measurement so the predicted height matches what renders.
pub fn git_lower_content_height(data: &GitData, flat_fields_len: usize) -> usize {
    let remotes_block = if data.remotes.is_empty() {
        0
    } else {
        3 + data.remotes.len()
    };
    let worktrees_block = if data.worktrees.is_empty() {
        0
    } else {
        3 + data.worktrees.len()
    };
    let pr_block = pull_request_block_height(&data.pull_requests);
    flat_fields_len + pr_block + remotes_block + worktrees_block
}

/// Total vertical rows the Git pane's content occupies. When the repo has a
/// description the About section renders, contributing the synced description
/// height plus the separator above the lower content; otherwise the section is
/// absent and only the lower content occupies the pane. Shared by the render
/// path (the viewport's scroll-overflow extent) and the cross-project top-row
/// measurement so the pager and the row height agree on what renders.
pub const fn git_content_height(
    synced_description_height: usize,
    has_description: bool,
    lower_content_height: usize,
) -> usize {
    if has_description {
        synced_description_height + 1 + lower_content_height
    } else {
        lower_content_height
    }
}

fn pull_request_block_height(section: &PullRequestSection) -> usize {
    if section.rows.is_empty()
        && matches!(section.state, PullRequestSectionState::HiddenConfirmedEmpty)
    {
        return 0;
    }
    let row_height = if section.rows.is_empty() {
        1
    } else {
        1 + section.rows.len()
    };
    let truncated = usize::from(matches!(
        section.completeness,
        Some(PullRequestCompleteness::Truncated { .. })
    ));
    2 + row_height + truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn section_for_pos_maps_flat_indices() {
        assert_eq!(section_for_pos(0, 3, 1, 2, 1), Some(Section::Flat(0)));
        assert_eq!(section_for_pos(2, 3, 1, 2, 1), Some(Section::Flat(2)));
    }

    #[test]
    fn section_for_pos_maps_pull_request_indices() {
        assert_eq!(
            section_for_pos(3, 3, 1, 2, 1),
            Some(Section::PullRequest(0))
        );
    }

    #[test]
    fn section_for_pos_maps_remote_indices() {
        assert_eq!(section_for_pos(4, 3, 1, 2, 1), Some(Section::Remote(0)));
        assert_eq!(section_for_pos(5, 3, 1, 2, 1), Some(Section::Remote(1)));
    }

    #[test]
    fn section_for_pos_maps_worktree_indices() {
        assert_eq!(section_for_pos(6, 3, 1, 2, 1), Some(Section::Worktree(0)));
    }

    #[test]
    fn section_for_pos_out_of_range_is_none() {
        assert_eq!(section_for_pos(7, 3, 1, 2, 1), None);
        assert_eq!(section_for_pos(0, 0, 0, 0, 0), None);
    }

    #[test]
    fn remotes_header_labels_sync_column() {
        let layout = sync_col_layout(&[], &[], WIDE_PANE);
        let mut lines = Vec::new();

        render_remote_header(&mut lines, &layout, true);

        let text = line_text(&lines[0]);
        assert!(text.contains("Branch"));
        assert!(text.contains("Tracked"));
        // Sync is the last, right-aligned column.
        assert!(text.ends_with("Sync"));
    }

    #[test]
    fn pull_request_header_labels_title_column() {
        let row = PullRequestRow {
            number:      1,
            title:       "feat: show open pull requests".to_string(),
            url:         String::new(),
            state_label: "ready",
            polling:     PullRequestPolling::Idle,
            branch:      "natepiano:feat/open-prs".to_string(),
            base:        "main".to_string(),
        };
        let widths = pull_request_col_widths(&[row], 80, Duration::ZERO);
        let mut lines = Vec::new();

        render_pull_request_header(&mut lines, &widths, true);

        assert!(line_text(&lines[0]).contains("Status"));
        assert!(line_text(&lines[0]).contains("Branch"));
        assert!(line_text(&lines[0]).contains("Title"));
    }

    #[test]
    fn pull_request_row_is_single_truncated_line() {
        let row = PullRequestRow {
            number:      1,
            title:       "feat: show open pull requests".to_string(),
            url:         String::new(),
            state_label: "ready",
            polling:     PullRequestPolling::Idle,
            branch:      "natepiano:feat/open-prs".to_string(),
            base:        "main".to_string(),
        };
        let widths = pull_request_col_widths(std::slice::from_ref(&row), 46, Duration::ZERO);

        let line = pull_request_row_line(
            &row,
            &widths,
            PaneSelectionState::Unselected,
            Duration::ZERO,
        );
        let text = line_text(&line);

        assert!(text.starts_with(" #1  ready"));
        assert!(text.contains("..."));
    }

    #[test]
    fn pull_request_row_marks_polling_checks() {
        let row = PullRequestRow {
            number:      1,
            title:       "feat: show open pull requests".to_string(),
            url:         String::new(),
            state_label: "checks",
            polling:     PullRequestPolling::Active,
            branch:      "natepiano:feat/open-prs".to_string(),
            base:        "main".to_string(),
        };
        let widths = pull_request_col_widths(std::slice::from_ref(&row), 80, Duration::ZERO);

        let line = pull_request_row_line(
            &row,
            &widths,
            PaneSelectionState::Unselected,
            Duration::ZERO,
        );

        assert!(line_text(&line).contains(ACTIVITY_SPINNER.frame_at(Duration::ZERO)));
        assert!(line.spans.iter().any(|span| {
            span.content.as_ref() == ACTIVITY_SPINNER.frame_at(Duration::ZERO)
                && span.style.fg == Some(accent_color())
        }));
    }

    /// A pane width wider than any test layout, so the flexible columns
    /// never truncate.
    const WIDE_PANE: usize = 200;

    fn sample_remote() -> RemoteRow {
        RemoteRow {
            name:            "origin".to_string(),
            icon:            GIT_LOCAL,
            display_url:     "natepiano/bevy_window_manager".to_string(),
            branch:          "main".to_string(),
            tracked_ref:     "origin/main".to_string(),
            status:          IN_SYNC.to_string(),
            full_url:        None,
            push_annotation: None,
        }
    }

    fn sample_worktree() -> WorktreeInfo {
        WorktreeInfo {
            name:         "bevy_window_manager_bevy_update".to_string(),
            path:         String::new(),
            branch:       Some("update/bevy_0.19.0".to_string()),
            tracked:      Some("main".to_string()),
            ahead_behind: Some((0, 0)),
        }
    }

    #[test]
    fn remotes_sync_values_align_right() {
        let row = sample_remote();
        let layout = sync_col_layout(std::slice::from_ref(&row), &[], WIDE_PANE);

        let line = remote_row_line(&row, &layout, PaneSelectionState::Unselected);

        // Right-aligned Sync is the last column, so no trailing pad follows.
        assert!(line_text(&line).ends_with("☑️"));
    }

    #[test]
    fn worktree_sync_values_align_right() {
        let row = sample_worktree();
        let layout = sync_col_layout(&[], std::slice::from_ref(&row), WIDE_PANE);

        let line = worktree_row_line(&row, &layout, PaneSelectionState::Unselected);

        assert!(line_text(&line).ends_with("☑️"));
    }

    #[test]
    fn remotes_and_worktrees_columns_line_up() {
        let remote = sample_remote();
        let wt = sample_worktree();
        let layout = sync_col_layout(
            std::slice::from_ref(&remote),
            std::slice::from_ref(&wt),
            WIDE_PANE,
        );

        let mut remote_header = Vec::new();
        render_remote_header(&mut remote_header, &layout, true);
        let mut worktree_header = Vec::new();
        render_worktree_header(&mut worktree_header, &layout, true);

        let remote_text = line_text(&remote_header[0]);
        let worktree_text = line_text(&worktree_header[0]);
        // The shared trio starts at the same column in both tables. Leading
        // text is ASCII, so byte index equals display column here.
        assert_eq!(remote_text.find("Branch"), worktree_text.find("Branch"));
        assert_eq!(remote_text.find("Tracked"), worktree_text.find("Tracked"));
        assert!(remote_text.ends_with("Sync"));
        assert!(worktree_text.ends_with("Sync"));
    }

    #[test]
    fn flex_columns_truncate_before_trio() {
        let remote = sample_remote();
        let wt = sample_worktree();
        // Narrow enough to shrink the URL / Name columns while the
        // Branch/Tracked/Sync trio stays at full width.
        let layout = sync_col_layout(std::slice::from_ref(&remote), std::slice::from_ref(&wt), 67);

        let remote_line = line_text(&remote_row_line(
            &remote,
            &layout,
            PaneSelectionState::Unselected,
        ));
        let worktree_line = line_text(&worktree_row_line(
            &wt,
            &layout,
            PaneSelectionState::Unselected,
        ));

        // The flexible columns ellipsize...
        assert!(remote_line.contains("..."));
        assert!(worktree_line.contains("..."));
        // ...but the Sync delta survives at the right edge of both rows.
        assert!(remote_line.ends_with("☑️"));
        assert!(worktree_line.ends_with("☑️"));
    }

    #[test]
    fn flex_columns_stop_shrinking_at_floor() {
        let remote = sample_remote();
        // Absurdly narrow: the URL can't squeeze past the floor, so the line
        // is left to clip rather than shrinking the column to nothing.
        let layout = sync_col_layout(std::slice::from_ref(&remote), &[], 4);

        assert_eq!(layout.remote_url, MIN_FLEX_COL);
    }
}
