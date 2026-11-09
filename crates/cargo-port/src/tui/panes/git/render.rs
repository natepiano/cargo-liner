use std::borrow::Cow;
use std::sync::LazyLock;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::ACTIVITY_SPINNER;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneSelectionState;
use tui_pane::PaneTitleCount;
use tui_pane::RuleTitle;
use tui_pane::Viewport;
use tui_pane::VisualRowSpan;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::inactive_border_color;
use tui_pane::inactive_title_color;
use tui_pane::label_color;
use tui_pane::on_error_color;
use tui_pane::success_color;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthStr;

use super::GitPane;
use crate::constants::GIT_LOCAL;
use crate::constants::NO_REMOTE_SYNC;
use crate::project::AheadBehind;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::GitStatus;
use crate::project::PinnedCommit;
use crate::project::PullRequestCompleteness;
use crate::project::TrackedRef;
use crate::tui::panes;
use crate::tui::panes::DescriptionBlock;
use crate::tui::panes::DescriptionState;
use crate::tui::panes::DetailField;
use crate::tui::panes::EmptyDescriptionBehavior;
use crate::tui::panes::GitData;
#[cfg(test)]
use crate::tui::panes::PullRequestPolling;
use crate::tui::panes::PullRequestRow;
use crate::tui::panes::PullRequestSection;
use crate::tui::panes::PullRequestSectionState;
use crate::tui::panes::QuotaObservation;
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
use crate::tui::panes::pane_data::GitCheckout;
use crate::tui::panes::pane_data::GitProbe;
use crate::tui::panes::support;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::theme_roles;

static MIN_FITTED_TEXT_WIDTH: LazyLock<usize> = LazyLock::new(|| FIT_TEXT_ELLIPSIS.width() + 1);
pub(super) static SPLIT_ROW_MIN_INNER_WIDTH: LazyLock<usize> =
    LazyLock::new(|| 2 * *MIN_FITTED_TEXT_WIDTH + 3);

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

pub(super) fn git_label_width(fields: &[DetailField]) -> usize {
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
    const fn is_focused(self) -> bool { matches!(self, Self::Focused) }
}

impl From<bool> for SectionFocus {
    fn from(focused: bool) -> Self {
        if focused {
            Self::Focused
        } else {
            Self::Unfocused
        }
    }
}

struct SectionRule {
    inner_y:       usize,
    title:         String,
    section_focus: SectionFocus,
}

/// Result of building the Git pane paragraph, used to register row
/// hitboxes at the correct screen rows after rendering.
struct GitRenderLayout {
    content_height: usize,
    scroll_offset:  usize,
    row_spans:      Vec<VisualRowSpan>,
}

/// Mutable accumulators threaded through the per-section builders.
struct SectionAccum<'a> {
    lines:               &'a mut Vec<Line<'static>>,
    focused_output_line: &'a mut usize,
    section_rules:       &'a mut Vec<SectionRule>,
    row_spans:           &'a mut Vec<VisualRowSpan>,
}

fn render_git_column_inner(
    frame: &mut Frame,
    ctx: &GitRenderCtx<'_>,
    outer_area: Rect,
    inner_area: Rect,
    chrome: &mut PaneFrameChrome,
) -> GitRenderLayout {
    let flat_len = ctx.fields.len();
    let pull_requests_len = ctx.data.pull_requests.rows.len();
    let remotes_len = ctx.data.remotes.listed().len();
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
    let mut row_spans: Vec<VisualRowSpan> =
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
            area_height: inner_area.height,
            label_width: git_label_width(ctx.fields),
            row_offset:  ctx.row_offset,
        },
    );
    append_pull_requests_section(&mut accum, ctx, flat_len, current_section, inner_area.width);
    // `sync_col_layout` computes one `SyncColLayout` for `append_remotes_section` and
    // `append_worktrees_section`, aligning their Branch, Tracked and Sync columns vertically.
    let sync_layout = sync_col_layout(
        ctx.data.remotes.listed(),
        &ctx.data.worktrees,
        usize::from(inner_area.width),
    );
    let remote_layout = remote_col_layout(ctx.data, usize::from(inner_area.width));
    append_remotes_section(
        &mut accum,
        ctx,
        flat_len,
        pull_requests_len,
        current_section,
        &remote_layout,
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

    let scroll_y = paginate_git_column(
        &mut lines,
        &mut row_spans,
        &mut section_rules,
        focused_output_line,
        inner_area.height,
        ctx.focus,
    );
    let content_height = lines.len();
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), inner_area);
    paint_git_row_selections(frame, ctx, inner_area, scroll_y, &row_spans);
    push_section_overlays(&section_rules, scroll_y, outer_area, inner_area, chrome);
    GitRenderLayout {
        content_height,
        scroll_offset: usize::from(scroll_y),
        row_spans,
    }
}

fn paginate_git_column(
    lines: &mut Vec<Line<'static>>,
    row_spans: &mut [VisualRowSpan],
    section_rules: &mut [SectionRule],
    focused_line: usize,
    height: u16,
    focus: PaneFocusState,
) -> u16 {
    let headers =
        support::detail_header_groups(row_spans, section_rules.iter().map(|rule| rule.inner_y));
    let pages =
        support::paginate_detail_lines(lines, row_spans, &headers, focused_line, height, focus);
    for span in row_spans {
        pages.place_span(span);
    }
    for rule in section_rules {
        rule.inner_y = pages.line_y(rule.inner_y);
    }
    pages.scroll_offset
}

fn paint_git_row_selections(
    frame: &mut Frame,
    ctx: &GitRenderCtx<'_>,
    inner_area: Rect,
    scroll_y: u16,
    row_spans: &[VisualRowSpan],
) {
    for (index, span) in row_spans.iter().enumerate() {
        let start = span.start_y.max(usize::from(scroll_y));
        let end = (span.start_y + span.height)
            .min(usize::from(scroll_y) + usize::from(inner_area.height));
        if start < end {
            let area = Rect::new(
                inner_area.x,
                inner_area.y + u16::try_from(start - usize::from(scroll_y)).unwrap_or(u16::MAX),
                inner_area.width,
                u16::try_from(end - start).unwrap_or(u16::MAX),
            );
            theme_roles::paint_selection(
                frame,
                area,
                tui_pane::selection_state(ctx.pane, ctx.row_offset + index, ctx.focus),
                ctx.styles.chrome.color_mode,
            );
        }
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
    let mode = ctx.styles.chrome.color_mode;
    if ctx.data.remotes.listed().is_empty() {
        return;
    }
    let focused = matches!(current_section, Some(Section::Remote(_)));
    let cursor = match current_section {
        Some(Section::Remote(i)) => Some(i),
        _ => None,
    };
    let title = section_title_text("Remotes", ctx.data.remotes.listed().len(), cursor);
    // Push a blank spacer into `accum.lines`, then record `SectionRule::inner_y` and reserve its
    // placeholder line for the overlay.
    accum.lines.push(Line::from(Span::raw(String::new())));
    accum.section_rules.push(SectionRule {
        inner_y: accum.lines.len(),
        title,
        section_focus: SectionFocus::from(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    render_remote_header(accum.lines, layout, focused);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, remote) in ctx.data.remotes.listed().iter().enumerate() {
        let row_index = ctx.row_offset + flat_len + pull_requests_len + i;
        accum.row_spans.push(VisualRowSpan {
            start_y: accum.lines.len(),
            height:  1,
        });
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = tui_pane::selection_state(ctx.pane, row_index, ctx.focus);
        accum.lines.push(
            remote_row_line(remote, layout, selection, mode)
                .style(theme_roles::selection_style(selection, mode)),
        );
    }
}

fn append_pull_requests_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    current_section: Option<Section>,
    area_width: u16,
) {
    let mode = ctx.styles.chrome.color_mode;
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
    let mut title = match section.state {
        PullRequestSectionState::Loading | PullRequestSectionState::Unavailable => {
            "Pull Requests".to_string()
        },
        PullRequestSectionState::Loaded
        | PullRequestSectionState::Stale
        | PullRequestSectionState::HiddenConfirmedEmpty => {
            section_title_text("Pull Requests", section.rows.len(), cursor)
        },
    };
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
        section_focus: SectionFocus::from(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));

    if section.rows.is_empty() {
        accum.lines.push(Line::from(Span::styled(
            format!(
                " {}",
                fit_text(
                    &pull_request_status_text(section),
                    usize::from(area_width).saturating_sub(2)
                )
            ),
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
        accum.lines.push(
            pull_request_row_line(row, &col_widths, selection, ctx.animation_elapsed, mode)
                .style(theme_roles::selection_style(selection, mode)),
        );
        accum.row_spans.push(VisualRowSpan { start_y, height: 1 });
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
    mode: ColorMode,
) -> Line<'static> {
    let style = selection.patch(Style::default().fg(inactive_title_color()), mode);
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
            selection.patch(Style::default().fg(accent_color()), mode),
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

fn fit_text(text: &str, max_width: usize) -> Cow<'_, str> {
    if text.width() <= max_width {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(render::truncate_with_ellipsis(
            text,
            max_width,
            FIT_TEXT_ELLIPSIS,
        ))
    }
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
    let mode = ctx.styles.chrome.color_mode;
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
        section_focus: SectionFocus::from(focused),
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    render_worktree_header(accum.lines, layout, focused);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, wt) in ctx.data.worktrees.iter().enumerate() {
        let row_index = ctx.row_offset + flat_len + pull_requests_len + remotes_len + i;
        accum.row_spans.push(VisualRowSpan {
            start_y: accum.lines.len(),
            height:  1,
        });
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = tui_pane::selection_state(ctx.pane, row_index, ctx.focus);
        accum.lines.push(
            worktree_row_line(wt, layout, selection, mode)
                .style(theme_roles::selection_style(selection, mode)),
        );
    }
}

fn section_title_text(label: &str, len: usize, cursor: Option<usize>) -> String {
    format!("{label} {}", PaneTitleCount::Single { len, cursor }.body())
}

/// Report each section rule, full pane width so the grid pass tees it into
/// the side borders — and in the same pass gives it the pane's own border
/// shade, so a focused pane stays yellow end to end.
fn push_section_overlays(
    section_rules: &[SectionRule],
    scroll_y: u16,
    outer_area: Rect,
    inner_area: Rect,
    chrome: &mut PaneFrameChrome,
) {
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
        let rule_area = Rect {
            x:      outer_area.x,
            y:      abs_y,
            width:  outer_area.width,
            height: 1,
        };
        chrome.rules.push(rule_area);
        let title =
            section_title_with_count(&rule.title, usize::from(rule_area.width.saturating_sub(3)));
        chrome.labels.extend(tui_pane::rule_title_label(
            rule_area,
            RuleTitle {
                text:  &title,
                style: title_style,
            },
            None,
        ));
    }
}

fn section_title_with_count(title: &str, width: usize) -> String {
    let Some((word, count)) = title.rsplit_once(" (") else {
        return render::truncate_with_ellipsis(title, width, "…");
    };
    if title.width() <= width {
        return title.to_string();
    }
    let total = count
        .trim_end_matches(')')
        .rsplit(" of ")
        .next()
        .unwrap_or(count);
    let compact = format!("{word} ({total})");
    if compact.width() <= width {
        return compact;
    }
    if word.width() <= width {
        return word.to_string();
    }
    render::truncate_with_ellipsis(word, width, "…")
}

struct RenderFlatArgs<'a> {
    data:        &'a GitData,
    fields:      &'a [DetailField],
    pane:        &'a Viewport,
    focus:       PaneFocusState,
    styles:      &'a RenderStyles,
    area_width:  u16,
    area_height: u16,
    label_width: usize,
    row_offset:  usize,
}

/// Add the local branch marker to the field value.
fn build_field_value(data: &GitData, field: DetailField) -> String {
    let raw = field.git_value(data);
    match field {
        DetailField::Head
            if data.is_local()
                && !raw.is_empty()
                && !matches!(data.checkout, GitCheckout::UnreadableHead) =>
        {
            format!("{raw} ({GIT_LOCAL} local)")
        },
        DetailField::RateLimitCore | DetailField::RateLimitGraphQl
            if data.github_status.is_unauthenticated() =>
        {
            let hint = DetailField::GithubStatus.git_value(data);
            match raw.as_str() {
                "" => format!("({hint})"),
                _ => format!("{raw} ({hint})"),
            }
        },
        _ => raw,
    }
}

/// A value's semantic style; rendering never infers an error from text.
enum GitValueStyle {
    Plain,
    Error,
    Warning,
    Success,
    Inactive,
    Broken,
}

impl GitValueStyle {
    fn style(self) -> Style {
        match self {
            Self::Plain => Style::default(),
            Self::Error => Style::default().fg(error_color()),
            Self::Warning => Style::default().fg(warning_color()),
            Self::Success => Style::default().fg(success_color()),
            Self::Inactive => Style::default().fg(inactive_border_color()),
            Self::Broken => Style::default().fg(on_error_color()).bg(error_color()),
        }
    }
}

fn field_value_style(data: &GitData, field: DetailField) -> Style {
    let role = match field {
        field if data.failed_reads.field_failed(field) => GitValueStyle::Error,
        DetailField::WorktreeError => GitValueStyle::Broken,
        DetailField::Head if data.checkout.head_value_is_unreadable() => GitValueStyle::Error,
        DetailField::GitStatus if matches!(data.status, Some(GitStatus::Unreadable)) => {
            GitValueStyle::Error
        },
        DetailField::VsLocal => match data.vs_local {
            GitProbe::Probed(Comparison::Unreadable) => GitValueStyle::Error,
            GitProbe::Probed(Comparison::Counted(AheadBehind {
                ahead: 0,
                behind: 0,
            })) => GitValueStyle::Success,
            GitProbe::Awaiting | GitProbe::Probed(Comparison::NotCompared) => {
                GitValueStyle::Inactive
            },
            GitProbe::Probed(Comparison::Counted(_)) => GitValueStyle::Plain,
        },
        DetailField::Inception
            if matches!(data.inception, FirstCommit::Probed(CommitDate::Unreadable)) =>
        {
            GitValueStyle::Error
        },
        DetailField::LastCommit
            if matches!(data.last_commit, GitProbe::Probed(CommitDate::Unreadable)) =>
        {
            GitValueStyle::Error
        },
        DetailField::Bisect
            if matches!(
                data.bisect,
                Some(crate::project::BisectProgress::Unreadable)
            ) =>
        {
            GitValueStyle::Error
        },
        DetailField::Pinned
            if data.submodule_ctx.as_ref().is_some_and(|context| {
                matches!(context.pinned_commit, PinnedCommit::Unreadable)
            }) =>
        {
            GitValueStyle::Error
        },
        DetailField::RemotesUnreadable => GitValueStyle::Error,
        DetailField::GithubStatus | DetailField::RateLimitCore | DetailField::RateLimitGraphQl => {
            if data.github_status.is_unauthenticated() {
                GitValueStyle::Warning
            } else if !data.github_status.is_available() {
                GitValueStyle::Error
            } else {
                GitValueStyle::Plain
            }
        },
        DetailField::Stars if panes::github_stars_is_unreachable_placeholder(data) => {
            GitValueStyle::Warning
        },
        DetailField::Worktrees
        | DetailField::DeletedWorktrees
        | DetailField::Path
        | DetailField::Targets
        | DetailField::Disk
        | DetailField::Tracks
        | DetailField::Pinned
        | DetailField::DiskTarget
        | DetailField::DiskNonTarget
        | DetailField::DiskOutOfTreeTarget
        | DetailField::Lint
        | DetailField::Ci
        | DetailField::Head
        | DetailField::Bisect
        | DetailField::GitStatus
        | DetailField::Stars
        | DetailField::Inception
        | DetailField::LastCommit
        | DetailField::LastFetched
        | DetailField::Version
        | DetailField::Edition
        | DetailField::License
        | DetailField::Homepage
        | DetailField::Repository => GitValueStyle::Plain,
    };
    role.style()
}

fn flat_field_label_width(area_width: u16, label_width: usize, label_only: bool) -> usize {
    if label_only {
        usize::from(area_width).saturating_sub(1)
    } else if wraps_field_value(area_width, label_width) {
        label_width
    } else {
        label_width.min(usize::from(area_width).saturating_sub(3) / 2)
    }
}

/// Keep compact rows below the width needed for a whole field label.
fn wraps_field_value(area_width: u16, label_width: usize) -> bool {
    let width = usize::from(area_width);
    width <= support::NARROW_FIELD_INNER_WIDTH && width >= label_width + 3
}

fn render_flat_fields(accum: &mut SectionAccum<'_>, args: &RenderFlatArgs<'_>) {
    for (i, field) in args.fields.iter().enumerate() {
        let label_width = label_width_for_field(args, *field);
        let row_index = args.row_offset + i;
        let lines = flat_field_lines(args, *field, row_index, label_width);
        accum.row_spans.push(VisualRowSpan {
            start_y: accum.lines.len(),
            height:  lines.len(),
        });
        if matches!(args.focus, PaneFocusState::Active) && row_index == args.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        accum.lines.extend(lines);
    }
}

fn github_status_line(args: &RenderFlatArgs<'_>, row_index: usize) -> Line<'static> {
    let selection = tui_pane::selection_state(args.pane, row_index, args.focus);
    let overlay = theme_roles::selection_style(selection, args.styles.chrome.color_mode);
    let value = DetailField::GithubStatus.git_value(args.data);
    let width = usize::from(args.area_width).saturating_sub(2);
    Line::from(Span::styled(
        format!(" {}", fit_text(&value, width)),
        field_value_style(args.data, DetailField::GithubStatus).patch(overlay),
    ))
    .style(overlay)
}

fn label_width_for_field(args: &RenderFlatArgs<'_>, field: DetailField) -> usize {
    let width = usize::from(args.area_width);
    let desired = if width <= support::NARROW_FIELD_INNER_WIDTH
        && matches!(
            field,
            DetailField::RateLimitCore | DetailField::RateLimitGraphQl
        ) {
        git_label_width(&[DetailField::RateLimitCore, DetailField::RateLimitGraphQl])
    } else if args.area_height < 2 {
        field.label().width()
    } else {
        args.label_width
    };
    if field == DetailField::Head && width <= support::NARROW_FIELD_INNER_WIDTH {
        let value_width = build_field_value(args.data, field).width();
        if field.label().width() + value_width + 3 <= width {
            return desired
                .min(width - value_width - 3)
                .max(field.label().width());
        }
    }
    if width >= field.label().width() + 2 && width > *SPLIT_ROW_MIN_INNER_WIDTH {
        desired
            .min(width.saturating_sub(2))
            .max(field.label().width())
    } else if width == field.label().width() + 1 && width > *SPLIT_ROW_MIN_INNER_WIDTH {
        field.label().width()
    } else {
        flat_field_label_width(
            args.area_width,
            args.label_width,
            width < *SPLIT_ROW_MIN_INNER_WIDTH,
        )
    }
}

/// Keep a whole label when its value needs a separate page.
fn field_uses_value_row(args: &RenderFlatArgs<'_>, label_width: usize) -> bool {
    let width = usize::from(args.area_width);
    width > *SPLIT_ROW_MIN_INNER_WIDTH
        && width <= support::NARROW_FIELD_INNER_WIDTH
        && label_width < width
        && (args.area_height > 1 || label_width + 1 >= width)
}

fn flat_field_lines(
    args: &RenderFlatArgs<'_>,
    field: DetailField,
    row_index: usize,
    label_width: usize,
) -> Vec<Line<'static>> {
    let mode = args.styles.chrome.color_mode;
    if field == DetailField::GithubStatus {
        return vec![github_status_line(args, row_index)];
    }
    let one_row = args.area_height < 2
        && usize::from(args.area_width) <= support::NARROW_FIELD_INNER_WIDTH
        && usize::from(args.area_width) > *SPLIT_ROW_MIN_INNER_WIDTH;
    let value = build_field_value(args.data, field);
    let compact_prefix = usize::from(args.area_width) > *SPLIT_ROW_MIN_INNER_WIDTH
        && field.label().width() + 1 == usize::from(args.area_width);
    let padding = if compact_prefix {
        1
    } else if one_row {
        2
    } else {
        3
    };
    let content_width = usize::from(args.area_width).saturating_sub(padding);
    let available = content_width.saturating_sub(label_width);
    let label = fit_text(field.label(), label_width);
    let selection = tui_pane::selection_state(args.pane, row_index, args.focus);
    let overlay = theme_roles::selection_style(selection, mode);
    let label_style = args.styles.readonly_label.patch(overlay);
    if usize::from(args.area_width) < *SPLIT_ROW_MIN_INNER_WIDTH {
        return vec![Line::from(Span::styled(format!(" {label}"), label_style)).style(overlay)];
    }
    let is_rate_limit_row = matches!(
        field,
        DetailField::RateLimitCore | DetailField::RateLimitGraphQl
    );
    let prefix = if compact_prefix {
        format!("{label} ")
    } else {
        format!(" {label:<label_width$} ")
    };
    let wraps = matches!(field, DetailField::WorktreeError | DetailField::Bisect);
    let value = if is_rate_limit_row {
        fit_rate_limit_value(args.data, field, value, available)
    } else {
        value
    };
    let value_style = field_value_style(args.data, field).patch(overlay);
    let mut lines = if !is_rate_limit_row && field_uses_value_row(args, label_width) {
        support::field_value_lines(
            prefix,
            &value,
            usize::from(args.area_width),
            label_style,
            value_style,
            true,
        )
    } else if wraps && !value.is_empty() && available > 0 && value.width() > available {
        let prefix_len = prefix.width();
        support::word_wrap(&value, available)
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| {
                let prefix_span = if index == 0 {
                    Span::styled(prefix.clone(), label_style)
                } else {
                    Span::raw(" ".repeat(prefix_len))
                };
                Line::from(vec![prefix_span, Span::styled(chunk, value_style)])
            })
            .collect()
    } else {
        let value = if wraps || value.width() <= available {
            value
        } else {
            fit_text(&value, available).into_owned()
        };
        let mut spans = vec![Span::styled(prefix, label_style)];
        if is_rate_limit_row
            && args.data.github_status.is_available()
            && let Some(index) = value.find(" resets ")
        {
            let (base, reset) = value.split_at(index);
            let reset_style =
                selection.patch(Style::default().fg(tui_pane::secondary_text_color()), mode);
            spans.push(Span::styled(base.to_string(), value_style));
            spans.push(Span::styled(reset.to_string(), reset_style));
        } else {
            spans.push(Span::styled(value, value_style));
        }
        vec![Line::from(spans)]
    };
    for line in &mut lines {
        line.style = line.style.patch(overlay);
    }
    lines
}

fn fit_rate_limit_value(
    data: &GitData,
    field: DetailField,
    value: String,
    available: usize,
) -> String {
    if value.width() <= available {
        return value;
    }
    let observation = match field {
        DetailField::RateLimitCore => data.rate_limit_core,
        DetailField::RateLimitGraphQl => data.rate_limit_graphql,
        _ => return value,
    };
    let QuotaObservation::Observed(quota) = observation else {
        return value;
    };
    let base = format!("{}/{}", quota.remaining, quota.limit);
    if base.width() <= available {
        base
    } else {
        quota.remaining.to_string()
    }
}

// `SyncColLayout` shares trailing column widths between Remotes and Worktrees.

// `SyncColLayout` supplies shared Branch/Tracked/Sync widths to `render_remote_header` and
// `render_worktree_header`, aligning both sections vertically.

/// Select the leading columns that fit before Branch, Tracked and Sync.
#[derive(Clone, Copy)]
enum SyncTableMode {
    Compact,
    Full,
}

const PREFERRED_REMOTE_NAME_FLOOR: usize = 4;

/// Share column widths between the Remotes and Worktrees tables.
struct SyncColLayout {
    available:     usize,
    mode:          SyncTableMode,
    remote_name:   usize,
    remote_url:    usize,
    worktree_name: usize,
    /// Display column where the Branch column starts in both tables.
    lead:          usize,
    branch:        usize,
    tracked:       usize,
    sync:          usize,
}

fn remote_col_layout(data: &GitData, available: usize) -> SyncColLayout {
    sync_col_layout(data.remotes.listed(), &data.worktrees, available)
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
    let mut remote_name = remotes
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

    let values = sync_value_widths(remotes, worktrees);
    let branch = values.0.max(BRANCH_HEADER.width());
    let tracked = values.1.max(TRACKED_HEADER.width());
    let sync = values.2.max(SYNC_HEADER.width());
    let full_lead = available.saturating_sub(1 + branch + tracked + sync + 4);
    let remote_name_floor = full_lead
        .saturating_sub(1 + REMOTE_ICON_COL + 4 + *MIN_FITTED_TEXT_WIDTH)
        .clamp(*MIN_FITTED_TEXT_WIDTH, PREFERRED_REMOTE_NAME_FLOOR);
    let remote_fixed_floor = 1 + REMOTE_ICON_COL + *MIN_FITTED_TEXT_WIDTH + 4;
    let min_url = if available < remote_fixed_floor + *MIN_FITTED_TEXT_WIDTH {
        MIN_FLEX_COL
    } else {
        *MIN_FITTED_TEXT_WIDTH
    };
    let min_lead = remote_lead_or(remotes, remote_fixed_floor + min_url)
        .max(worktree_lead_or(worktrees, 3 + *MIN_FITTED_TEXT_WIDTH));
    let mode = if available >= 4 * *MIN_FITTED_TEXT_WIDTH + 4
        && min_lead + branch + tracked + sync + 4 > available.saturating_sub(1)
    {
        SyncTableMode::Compact
    } else {
        SyncTableMode::Full
    };
    let budget = available.saturating_sub(1);
    let natural_lead = if matches!(mode, SyncTableMode::Compact) {
        remote_lead_or(remotes, remote_name + 2).max(worktree_lead_or(worktrees, worktree_name + 2))
    } else {
        remote_lead_or(remotes, 1 + REMOTE_ICON_COL + remote_name + 4 + remote_url)
            .max(worktree_lead_or(worktrees, 3 + worktree_name))
    };
    let min_lead = if matches!(mode, SyncTableMode::Compact) {
        *MIN_FITTED_TEXT_WIDTH + 2
    } else {
        min_lead
    };
    if matches!(mode, SyncTableMode::Full) && natural_lead + branch + tracked + sync + 4 > budget {
        remote_name = remote_name.min(remote_name_floor);
    }
    let mut layout = SyncColLayout {
        available: budget,
        mode,
        remote_name,
        remote_url,
        worktree_name,
        lead: min_lead.min(natural_lead),
        branch,
        tracked,
        sync,
    };
    fit_sync_trio(&mut layout, values);
    let gaps = 2 * layout.column_gap();
    layout.lead = budget
        .saturating_sub(layout.branch + layout.tracked + layout.sync + gaps)
        .clamp(layout.lead, natural_lead.max(layout.lead));
    if matches!(mode, SyncTableMode::Compact) {
        layout.remote_name = remote_name.min(layout.lead.saturating_sub(2));
        layout.remote_url = 0;
        layout.worktree_name = worktree_name.min(layout.lead.saturating_sub(2));
    } else {
        layout.remote_url = remote_url.min(
            layout
                .lead
                .saturating_sub(1 + REMOTE_ICON_COL + remote_name + 4),
        );
        layout.worktree_name = worktree_name.min(layout.lead.saturating_sub(3));
    }
    layout
}

impl SyncColLayout {
    const fn column_gap(&self) -> usize {
        match self.mode {
            SyncTableMode::Compact => 1,
            SyncTableMode::Full => 2,
        }
    }
}

fn sync_value_widths(remotes: &[RemoteRow], worktrees: &[WorktreeInfo]) -> (usize, usize, usize) {
    let branch = remotes
        .iter()
        .map(|r| r.branch.width())
        .chain(worktrees.iter().map(|w| worktree_branch_text(w).width()))
        .max()
        .unwrap_or(0);
    let tracked = remotes
        .iter()
        .map(|r| r.tracked_text().width())
        .chain(worktrees.iter().map(|w| worktree_tracked_text(w).width()))
        .max()
        .unwrap_or(0);
    let sync = remotes
        .iter()
        .map(|r| r.status_text().width())
        .chain(
            worktrees
                .iter()
                .map(|w| panes::format_ahead_behind(w.ahead_behind).width()),
        )
        .max()
        .unwrap_or(0);
    (branch, tracked, sync)
}

fn fit_sync_trio(layout: &mut SyncColLayout, values: (usize, usize, usize)) {
    let mut excess =
        (layout.lead + layout.branch + layout.tracked + layout.sync + 2 * layout.column_gap())
            .saturating_sub(layout.available);
    // Take spare header cells before cutting the Branch or Tracked labels.
    while excess > 0 {
        let spare = [
            (layout.branch, "Branch".width()),
            (layout.tracked, "Tracked".width()),
            (layout.sync, "Sync".width()),
        ];
        let Some((index, _)) = spare
            .iter()
            .enumerate()
            .filter(|(_, (width, header))| width > header)
            .max_by_key(|(_, (width, _))| *width)
        else {
            break;
        };
        match index {
            0 => layout.branch -= 1,
            1 => layout.tracked -= 1,
            _ => layout.sync -= 1,
        }
        excess -= 1;
    }
    let sync_padding = excess.min(
        layout
            .sync
            .saturating_sub(values.2.max(*MIN_FITTED_TEXT_WIDTH)),
    );
    layout.sync -= sync_padding;
    excess -= sync_padding;
    if excess > 0 {
        for (width, value) in [
            (&mut layout.tracked, values.1),
            (&mut layout.branch, values.0),
            (&mut layout.sync, values.2),
        ] {
            let cut = excess.min(width.saturating_sub(value.max(*MIN_FITTED_TEXT_WIDTH)));
            *width -= cut;
            excess -= cut;
        }
    }
    while excess > 0
        && (layout.branch > *MIN_FITTED_TEXT_WIDTH || layout.tracked > *MIN_FITTED_TEXT_WIDTH)
    {
        let width = if layout.branch >= layout.tracked {
            &mut layout.branch
        } else {
            &mut layout.tracked
        };
        *width -= 1;
        excess -= 1;
    }
    let cut = excess.min(
        layout
            .sync
            .saturating_sub(values.2.max(*MIN_FITTED_TEXT_WIDTH)),
    );
    layout.sync -= cut;
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
        "{}{gap}{}{gap}{}",
        left_column(branch, layout.branch),
        left_column(tracked, layout.tracked),
        right_column(sync, layout.sync),
        gap = " ".repeat(layout.column_gap()),
    )
}

fn left_column(text: &str, width: usize) -> String {
    let text = fit_text(text, width);
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

fn right_column(text: &str, width: usize) -> String {
    let text = fit_text(text, width);
    format!("{}{text}", " ".repeat(width.saturating_sub(text.width())))
}

fn fit_table_line(mut line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() <= width {
        return line;
    }
    let mut remaining = width.saturating_sub(1);
    let mut spans = Vec::new();
    for span in line.spans {
        let text = render::truncate_with_ellipsis(&span.content, remaining, "");
        remaining = remaining.saturating_sub(text.width());
        spans.push(Span::styled(text, span.style));
        if remaining == 0 {
            spans.push(Span::styled("…", span.style));
            break;
        }
    }
    line.spans = spans;
    line
}

fn render_remote_header(lines: &mut Vec<Line<'static>>, layout: &SyncColLayout, focused: bool) {
    let style = column_header_style(focused);
    let name = left_column(REMOTES_NAME_HEADER, layout.remote_name);
    let leading = match layout.mode {
        SyncTableMode::Compact => format!(" {name}"),
        SyncTableMode::Full => format!(
            " {}{name}  {}",
            " ".repeat(REMOTE_ICON_COL),
            left_column(REMOTES_URL_HEADER, layout.remote_url)
        ),
    };
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let trio = sync_trio(layout, BRANCH_HEADER, TRACKED_HEADER, SYNC_HEADER);
    lines.push(fit_table_line(
        Line::from(Span::styled(format!("{leading}{pad}{trio}"), style)),
        layout.available,
    ));
}

fn remote_row_line(
    row: &RemoteRow,
    layout: &SyncColLayout,
    selection: PaneSelectionState,
    mode: ColorMode,
) -> Line<'static> {
    let icon_cell = match layout.mode {
        SyncTableMode::Compact => String::new(),
        SyncTableMode::Full => format!(
            "{}{}",
            row.icon,
            " ".repeat(REMOTE_ICON_COL.saturating_sub(row.icon.width()))
        ),
    };
    let push_suffix = row
        .push_annotation
        .as_deref()
        .map(|annotation| format!("  {annotation}"))
        .unwrap_or_default();
    let name = left_column(&row.name, layout.remote_name);
    let leading = match layout.mode {
        SyncTableMode::Compact => name,
        SyncTableMode::Full => format!(
            "{name}  {}",
            left_column(&row.display_url, layout.remote_url)
        ),
    };
    let consumed = 1 + icon_cell.width() + leading.width();
    let pad = " ".repeat(layout.lead.saturating_sub(consumed));
    let leading = format!(
        "{leading}{pad}{}{gap}",
        left_column(&row.branch, layout.branch),
        gap = " ".repeat(layout.column_gap())
    );
    let data_style = selection.patch(Style::default().fg(inactive_title_color()), mode);
    let tracked_style = selection.patch(
        if matches!(row.tracked_ref, TrackedRef::Unreadable) {
            GitValueStyle::Error.style()
        } else {
            Style::default().fg(inactive_title_color())
        },
        mode,
    );
    let sync_style = selection.patch(
        if matches!(row.status, Comparison::Unreadable) {
            GitValueStyle::Error.style()
        } else {
            Style::default().fg(inactive_title_color())
        },
        mode,
    );
    fit_table_line(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(icon_cell, selection.patch(Style::default(), mode)),
            Span::styled(leading, data_style),
            Span::styled(
                left_column(&row.tracked_text(), layout.tracked),
                tracked_style,
            ),
            Span::styled(
                format!(
                    "{gap}{}",
                    right_column(&row.status_text(), layout.sync),
                    gap = " ".repeat(layout.column_gap())
                ),
                sync_style,
            ),
            Span::styled(
                fit_text(
                    &push_suffix,
                    layout.available.saturating_sub(
                        layout.lead + layout.branch + layout.tracked + layout.sync + 4,
                    ),
                )
                .into_owned(),
                data_style,
            ),
        ]),
        layout.available,
    )
}

// `render_worktree_header` and `worktree_row_line` render Worktrees with the shared
// `SyncColLayout`.

fn render_worktree_header(lines: &mut Vec<Line<'static>>, layout: &SyncColLayout, focused: bool) {
    let style = column_header_style(focused);
    let leading = format!(
        " {}",
        left_column(WORKTREES_NAME_HEADER, layout.worktree_name)
    );
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let trio = sync_trio(layout, BRANCH_HEADER, TRACKED_HEADER, SYNC_HEADER);
    lines.push(fit_table_line(
        Line::from(Span::styled(format!("{leading}{pad}{trio}"), style)),
        layout.available,
    ));
}

fn worktree_row_line(
    row: &WorktreeInfo,
    layout: &SyncColLayout,
    selection: PaneSelectionState,
    mode: ColorMode,
) -> Line<'static> {
    let sync_text = panes::format_ahead_behind(row.ahead_behind);
    let sync = right_column(&sync_text, layout.sync);
    let leading = format!(" {}", left_column(&row.name, layout.worktree_name));
    let pad = " ".repeat(layout.lead.saturating_sub(leading.width()));
    let prefix = format!(
        "{leading}{pad}{}{gap}{}{gap}",
        left_column(worktree_branch_text(row), layout.branch),
        left_column(worktree_tracked_text(row), layout.tracked),
        gap = " ".repeat(layout.column_gap()),
    );
    let style = selection.patch(Style::default().fg(inactive_title_color()), mode);
    let sync_style = selection.patch(
        if matches!(row.ahead_behind, Comparison::Unreadable) {
            GitValueStyle::Error.style()
        } else {
            Style::default().fg(inactive_title_color())
        },
        mode,
    );
    fit_table_line(
        Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(sync, sync_style),
        ]),
        layout.available,
    )
}

fn git_panel_title(data: &GitData) -> String {
    match &data.checkout {
        GitCheckout::Branch { name, .. } if !name.is_empty() => format!(" Git - {name} "),
        GitCheckout::Detached { short_sha } => format!(" Git - detached @ {short_sha} "),
        GitCheckout::Branch { .. }
        | GitCheckout::Unborn
        | GitCheckout::UnreadableHead
        | GitCheckout::Broken
        | GitCheckout::AwaitingProbe => tui_pane::pane_title("Git", &PaneTitleCount::None),
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
) -> PaneFrameChrome {
    let mode = ctx.color_mode;
    let Some(git_data) = pane.content().cloned() else {
        pane.viewport.clear_surface();
        pane.clear_row_layout();
        return empty_git_chrome(tui_pane::pane_title("Git", &PaneTitleCount::None));
    };

    let flat_fields = panes::git_fields_from_data(&git_data);
    let description_rows = usize::from(panes::git_has_description_row(&git_data));
    let total_rows = description_rows
        + flat_fields.len()
        + git_data.pull_requests.rows.len()
        + git_data.remotes.listed().len()
        + git_data.worktrees.len();
    if total_rows == 0 && git_data.description.as_deref().is_none_or(str::is_empty) {
        pane.viewport.clear_surface();
        pane.clear_row_layout();
        return empty_git_chrome(tui_pane::pane_title("Git", &PaneTitleCount::None));
    }

    pane.viewport.set_len(total_rows);
    // `pane.viewport.set_content_height(git_content_height(...))` includes headers and spacing from
    // `git_lower_content_height` plus the synced multiline description: these exceed the
    // addressable `total_rows`, so the pager must use rendered height.
    pane.viewport.set_content_height(git_content_height(
        usize::from(ctx.synced_description_height.rows()),
        description_rows > 0,
        git_lower_content_height(&git_data, flat_fields.len()),
    ));
    let pane_focus_state = pane.focus.pane_focus_state;
    let mut chrome = PaneFrameChrome {
        title: git_panel_title(&git_data),
        focused: matches!(pane_focus_state, PaneFocusState::Active),
        ..PaneFrameChrome::default()
    };
    let inner_area = tui_pane::frame_inner(area);

    let about_layout = render_git_about_section(
        frame,
        &GitAboutCtx {
            description: &git_data.description,
            flat_fields: &flat_fields,
            data: &git_data,
            outer_area: area,
            inner_area,
            synced_description_height: ctx.synced_description_height,
            pane: &pane.viewport,
            focus: pane_focus_state,
        },
        mode,
    );

    chrome.rules.extend(about_layout.separator);

    {
        let viewport = &mut pane.viewport;
        viewport.set_content_area(about_layout.content_area);
        viewport.set_viewport_rows(usize::from(about_layout.content_area.height));
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
    let layout = render_git_column_inner(
        frame,
        &git_ctx,
        area,
        about_layout.content_area,
        &mut chrome,
    );
    pane.viewport.set_scroll_offset(layout.scroll_offset);
    pane.viewport.set_content_height(layout.content_height);
    pane.set_row_layout(
        about_layout.description_rect,
        about_layout.content_area,
        description_rows,
        layout.row_spans,
    );
    chrome.labels.extend(tui_pane::overflow_affordance_label(
        area,
        tui_pane::ViewportOverflow::for_paged_content(
            pane.viewport.content_height(),
            pane.viewport.scroll_offset(),
            pane.viewport.visible_rows(),
        ),
        Style::default().fg(label_color()),
    ));
    let _ = ctx;
    chrome
}

/// The chrome an empty Git pane reports: the border always takes the
/// inactive shade, matching the dim block this path used to draw itself.
fn empty_git_chrome(title: impl Into<String>) -> PaneFrameChrome {
    PaneFrameChrome {
        title: title.into(),
        focused: false,
        ..PaneFrameChrome::default()
    }
}

/// Inputs for [`render_git_about_section`]. Grouped into a struct
/// so the function stays under the argument-count threshold.
struct GitAboutCtx<'a> {
    description:               &'a DescriptionState,
    flat_fields:               &'a [DetailField],
    data:                      &'a GitData,
    outer_area:                Rect,
    inner_area:                Rect,
    synced_description_height: SyncedDescriptionHeight,
    pane:                      &'a Viewport,
    focus:                     PaneFocusState,
}

#[derive(Clone, Copy)]
struct GitAboutLayout {
    content_area:     Rect,
    description_rect: Option<Rect>,
    /// The About separator, reported rather than drawn: the grid pass draws
    /// it with the pane borders it runs into, so the junctions come out
    /// right without this section knowing about them.
    separator:        Option<Rect>,
}

/// Render the About section (repo description) at the top of the Git panel,
/// separated from the rest of the pane by a horizontal rule the grid pass
/// draws. Returns the area below the separator for the scrolling content.
fn render_git_about_section(
    frame: &mut Frame,
    ctx: &GitAboutCtx<'_>,
    mode: ColorMode,
) -> GitAboutLayout {
    let inner_area = ctx.inner_area;

    let lower_content_height = git_lower_content_height(ctx.data, ctx.flat_fields.len());
    let reserved_lower_height = u16::from(lower_content_height > 0);
    let reserved_separator_height =
        u16::from(inner_area.height > reserved_lower_height.saturating_add(1));
    let baseline_max = inner_area
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));

    // `DescriptionBlock::for_pane` rebuilds the wrapped rows used for the frame's synced
    // description height; `block.render_with_selection` renders those owned rows so the content
    // cannot drift from the height used for inter-pane synchronization.
    let block = DescriptionBlock::for_pane(
        ctx.description,
        ctx.outer_area,
        EmptyDescriptionBehavior::RenderNothing,
    );
    let description_height = block.render_with_selection(
        frame,
        inner_area,
        ctx.synced_description_height,
        baseline_max,
        tui_pane::selection_state(ctx.pane, 0, ctx.focus),
        mode,
    );
    if description_height == 0 {
        return GitAboutLayout {
            content_area:     inner_area,
            description_rect: None,
            separator:        None,
        };
    }

    let separator_y = inner_area.y.saturating_add(description_height);
    let has_room_for_separator = separator_y < inner_area.bottom();
    if !has_room_for_separator {
        return GitAboutLayout {
            content_area:     Rect {
                x:      inner_area.x,
                y:      separator_y,
                width:  inner_area.width,
                height: 0,
            },
            description_rect: Some(Rect {
                x:      inner_area.x,
                y:      inner_area.y,
                width:  inner_area.width,
                height: description_height,
            }),
            separator:        None,
        };
    }

    let separator = Rect {
        x:      ctx.outer_area.x,
        y:      separator_y,
        width:  ctx.outer_area.width,
        height: 1,
    };

    let content_y = separator_y.saturating_add(1);
    GitAboutLayout {
        content_area:     Rect {
            x:      inner_area.x,
            y:      content_y,
            width:  inner_area.width,
            height: inner_area.bottom().saturating_sub(content_y),
        },
        description_rect: Some(Rect {
            x:      inner_area.x,
            y:      inner_area.y,
            width:  inner_area.width,
            height: description_height,
        }),
        separator:        Some(separator),
    }
}

/// Rows the Git pane's lower (scrolling) content occupies, below the About
/// section: the flat field rows plus the pull-request, remotes, and worktrees
/// blocks. Each of the remotes / worktrees blocks adds three header/spacing
/// rows when non-empty. Shared by the render path and the cross-project
/// top-row height measurement so the predicted height matches what renders.
pub(in crate::tui::panes) fn git_lower_content_height(
    data: &GitData,
    flat_fields_len: usize,
) -> usize {
    let remotes_block = if data.remotes.listed().is_empty() {
        0
    } else {
        3 + data.remotes.listed().len()
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
pub(in crate::tui::panes) const fn git_content_height(
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
    use tui_pane::ColorMode;

    use super::*;
    use crate::project::AheadBehind;
    use crate::project::Comparison;

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
        // `render_remote_header` places the right-aligned Sync column at the end of `text`.
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
        let mode = ColorMode::Colored;
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
            mode,
        );
        let text = line_text(&line);

        assert!(text.starts_with(" #1  ready"));
        assert!(text.contains(FIT_TEXT_ELLIPSIS));
    }

    #[test]
    fn pull_request_row_marks_polling_checks() {
        let mode = ColorMode::Colored;
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
            mode,
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
            tracked_ref:     TrackedRef::Named("origin/main".to_string()),
            status:          Comparison::Counted(AheadBehind {
                ahead:  0,
                behind: 0,
            }),
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
            ahead_behind: Comparison::Counted(AheadBehind {
                ahead:  0,
                behind: 0,
            }),
        }
    }

    #[test]
    fn wide_remotes_keep_the_icon_url_and_column_spacing() {
        let remote = sample_remote();
        let worktree = sample_worktree();
        let layout = sync_col_layout(
            std::slice::from_ref(&remote),
            std::slice::from_ref(&worktree),
            WIDE_PANE,
        );
        let line = remote_row_line(
            &remote,
            &layout,
            PaneSelectionState::Unselected,
            ColorMode::Colored,
        );

        assert_eq!(
            line_text(&line),
            " 📁 origin  natepiano/bevy_window_manager  main                origin/main    ☑️"
        );
    }

    #[test]
    fn remotes_keep_the_icon_and_url_only_after_the_trailing_columns_fit() {
        let remote = sample_remote();
        let worktree = sample_worktree();
        for width in [15, 22, 49, 50, 80, 95] {
            let layout = sync_col_layout(
                std::slice::from_ref(&remote),
                std::slice::from_ref(&worktree),
                width,
            );
            let text = line_text(&remote_row_line(
                &remote,
                &layout,
                PaneSelectionState::Unselected,
                ColorMode::Colored,
            ));
            if width < 50 {
                assert!(!text.contains("📁"), "{width}: {text}");
                assert!(
                    !text.split_whitespace().any(|token| token == "n…"),
                    "{width}: {text}"
                );
                assert_eq!(text.split_whitespace().count(), 4, "{width}: {text}");
            } else {
                assert!(text.starts_with(" 📁 "), "{width}: {text}");
                let tokens = text.split_whitespace().collect::<Vec<_>>();
                let url = tokens[2];
                assert!(url.width() >= 2, "{width}: {text}");
                let prefix = url.strip_suffix('…').unwrap_or(url);
                assert!(remote.display_url.starts_with(prefix), "{width}: {text}");
                assert!(
                    text.contains("main                origin/main"),
                    "{width}: {text}"
                );
                assert!(text.width() <= width, "{width}: {text}");
            }
        }
    }

    #[test]
    fn remotes_sync_values_align_right() {
        let mode = ColorMode::Colored;
        let row = sample_remote();
        let layout = sync_col_layout(std::slice::from_ref(&row), &[], WIDE_PANE);

        let line = remote_row_line(&row, &layout, PaneSelectionState::Unselected, mode);

        // `remote_row_line` right-aligns the Sync column at the end of `line`, so no trailing
        // padding follows.
        assert!(line_text(&line).ends_with("☑️"));
    }

    #[test]
    fn worktree_sync_values_align_right() {
        let mode = ColorMode::Colored;
        let row = sample_worktree();
        let layout = sync_col_layout(&[], std::slice::from_ref(&row), WIDE_PANE);

        let line = worktree_row_line(&row, &layout, PaneSelectionState::Unselected, mode);

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
        // `remote_text` and `worktree_text` place Branch, Tracked and Sync at matching columns;
        // their leading ASCII text makes byte indices equal display columns.
        assert_eq!(remote_text.find("Branch"), worktree_text.find("Branch"));
        assert_eq!(remote_text.find("Tracked"), worktree_text.find("Tracked"));
        assert!(remote_text.ends_with("Sync"));
        assert!(worktree_text.ends_with("Sync"));
    }

    #[test]
    fn flex_columns_truncate_before_trio() {
        let mode = ColorMode::Colored;
        let remote = sample_remote();
        let wt = sample_worktree();
        // `sync_col_layout(..., 67)` shrinks the URL and Name columns while preserving full Branch,
        // Tracked and Sync widths.
        let layout = sync_col_layout(std::slice::from_ref(&remote), std::slice::from_ref(&wt), 67);

        let remote_line = line_text(&remote_row_line(
            &remote,
            &layout,
            PaneSelectionState::Unselected,
            mode,
        ));
        let worktree_line = line_text(&worktree_row_line(
            &wt,
            &layout,
            PaneSelectionState::Unselected,
            mode,
        ));

        // `remote_line` and `worktree_line` ellipsis-truncate their flexible columns...
        assert!(remote_line.contains(FIT_TEXT_ELLIPSIS));
        assert!(worktree_line.contains(FIT_TEXT_ELLIPSIS));
        // ...while preserving the Sync delta at the right edge of both rows.
        assert!(remote_line.ends_with("☑️"));
        assert!(worktree_line.ends_with("☑️"));
    }

    #[test]
    fn flex_columns_stop_shrinking_at_floor() {
        let remote = sample_remote();
        // `sync_col_layout(..., 4)` keeps `layout.remote_url` at `MIN_FLEX_COL`; the line clips
        // instead of shrinking the URL column to nothing.
        let layout = sync_col_layout(std::slice::from_ref(&remote), &[], 4);

        assert_eq!(layout.remote_url, MIN_FLEX_COL);
    }
}

#[cfg(test)]
mod moved_facade_tests {
    use tui_pane::error_color;

    use super::field_value_style;
    use super::git_label_width;
    use crate::tui::panes::GitCheckout;
    use crate::tui::panes::GitData;
    use crate::tui::panes::HeadRelation;
    use crate::tui::panes::pane_data::DetailField;

    #[test]
    fn git_label_width_uses_ahead_behind_label() {
        let fields = vec![DetailField::VsLocal];

        assert_eq!(git_label_width(&fields), "Ahead/Behind".len());
    }
    #[test]
    fn unreadable_branch_relation_uses_error_color() {
        let data = GitData {
            checkout: GitCheckout::Branch {
                name:     "main".into(),
                relation: HeadRelation::Unreadable,
            },
            ..GitData::default()
        };
        assert_eq!(
            field_value_style(&data, DetailField::Head).fg,
            Some(error_color())
        );
    }
}
