use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::DetailField;
use super::GitData;
use super::PaneId;
use super::RemoteRow;
use super::WorktreeInfo;
use super::package;
use super::package::RenderStyles;
use crate::constants::GIT_LOCAL;
use crate::constants::IN_SYNC;
use crate::tui::app::App;
use crate::tui::app::AvailabilityStatus;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_TITLE_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface;
use crate::tui::pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneSelectionState;
use crate::tui::pane::PaneTitleCount;
use crate::tui::pane::Viewport;
use crate::tui::panes;

struct GitRenderCtx<'a> {
    data:   &'a GitData,
    fields: &'a [DetailField],
    pane:   &'a Viewport,
    focus:  PaneFocusState,
    styles: &'a RenderStyles,
}

/// Which section a flat `pos()` index lives in, plus the offset within
/// that section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Flat(usize),
    Remote(usize),
    Worktree(usize),
}

const fn section_for_pos(
    pos: usize,
    flat_len: usize,
    remotes_len: usize,
    worktrees_len: usize,
) -> Option<Section> {
    if pos < flat_len {
        Some(Section::Flat(pos))
    } else if pos < flat_len + remotes_len {
        Some(Section::Remote(pos - flat_len))
    } else if pos < flat_len + remotes_len + worktrees_len {
        Some(Section::Worktree(pos - flat_len - remotes_len))
    } else {
        None
    }
}

pub fn git_label_width(data: &GitData, fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| match *field {
            DetailField::VsLocal => format!("vs local {}", data.main_branch_label).width(),
            _ => field.label().width(),
        })
        .max()
        .unwrap_or(0)
        .max(8)
}

/// A section separator to render as an overlay rule after the paragraph
/// (so its `├`/`┤` endcaps can overlap the outer pane's vertical borders).
struct SectionRule {
    inner_y: usize,
    title:   String,
    focused: bool,
}

/// Result of building the Git pane paragraph, used to register row
/// hitboxes at the correct screen rows after rendering.
struct GitRenderLayout {
    scroll_offset: usize,
    /// Inner-y (paragraph-relative) of the first rendered line for each
    /// selectable row (flat fields first, then remote rows, then worktree
    /// rows). Same ordering as `pane.pos()`.
    row_line_ys:   Vec<usize>,
}

/// Mutable accumulators threaded through the per-section builders.
struct SectionAccum<'a> {
    lines:               &'a mut Vec<Line<'static>>,
    focused_output_line: &'a mut usize,
    section_rules:       &'a mut Vec<SectionRule>,
    row_line_ys:         &'a mut Vec<usize>,
}

fn render_git_column_inner(
    frame: &mut Frame,
    ctx: &GitRenderCtx<'_>,
    outer_area: Rect,
    inner_area: Rect,
) -> GitRenderLayout {
    let flat_len = ctx.fields.len();
    let remotes_len = ctx.data.remotes.len();
    let worktrees_len = ctx.data.worktrees.len();
    let current_section = if matches!(ctx.focus, PaneFocusState::Active) {
        section_for_pos(ctx.pane.pos(), flat_len, remotes_len, worktrees_len)
    } else {
        None
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let mut section_rules: Vec<SectionRule> = Vec::new();
    let mut row_line_ys: Vec<usize> = Vec::with_capacity(flat_len + remotes_len + worktrees_len);

    let mut accum = SectionAccum {
        lines:               &mut lines,
        focused_output_line: &mut focused_output_line,
        section_rules:       &mut section_rules,
        row_line_ys:         &mut row_line_ys,
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
            label_width: git_label_width(ctx.data, ctx.fields),
        },
    );
    append_remotes_section(&mut accum, ctx, flat_len, current_section);
    append_worktrees_section(&mut accum, ctx, flat_len, remotes_len, current_section);

    let scroll_y =
        package::detail_column_scroll_offset(ctx.focus, focused_output_line, inner_area.height);
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
        row_line_ys,
    }
}

/// Register one-line hitboxes for every visible selectable row. Called
/// after the paragraph renders because we need the scroll offset to map
/// inner-y to absolute screen-y.
fn register_git_row_hitboxes(app: &mut App, inner_area: Rect, layout: &GitRenderLayout) {
    let scroll = layout.scroll_offset;
    let visible_top = inner_area.y;
    let visible_bottom = inner_area.y.saturating_add(inner_area.height);
    for (row_index, &inner_y) in layout.row_line_ys.iter().enumerate() {
        if inner_y < scroll {
            continue;
        }
        let offset = inner_y - scroll;
        let screen_y = inner_area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        if screen_y < visible_top || screen_y >= visible_bottom {
            continue;
        }
        interaction::register_pane_row_hitbox(
            app,
            Rect::new(inner_area.x, screen_y, inner_area.width, 1),
            PaneId::Git,
            row_index,
            UiSurface::Content,
        );
    }
}

fn append_remotes_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    current_section: Option<Section>,
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
        focused,
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    let col_widths = remote_col_widths(&ctx.data.remotes);
    render_remote_header(accum.lines, &col_widths);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, remote) in ctx.data.remotes.iter().enumerate() {
        let row_index = flat_len + i;
        accum.row_line_ys.push(accum.lines.len());
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = ctx.pane.selection_state(row_index, ctx.focus);
        accum
            .lines
            .push(remote_row_line(remote, &col_widths, selection));
    }
}

fn append_worktrees_section(
    accum: &mut SectionAccum<'_>,
    ctx: &GitRenderCtx<'_>,
    flat_len: usize,
    remotes_len: usize,
    current_section: Option<Section>,
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
        focused,
    });
    accum.lines.push(Line::from(Span::raw(String::new())));
    let col_widths = worktree_col_widths(&ctx.data.worktrees);
    render_worktree_header(accum.lines, &col_widths);
    let active = matches!(ctx.focus, PaneFocusState::Active);
    for (i, wt) in ctx.data.worktrees.iter().enumerate() {
        let row_index = flat_len + remotes_len + i;
        accum.row_line_ys.push(accum.lines.len());
        if active && row_index == ctx.pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let selection = ctx.pane.selection_state(row_index, ctx.focus);
        accum
            .lines
            .push(worktree_row_line(wt, &col_widths, selection));
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
        let title_color = if rule.focused {
            TITLE_COLOR
        } else {
            INACTIVE_TITLE_COLOR
        };
        let title_style = Style::default()
            .fg(title_color)
            .add_modifier(Modifier::BOLD);
        pane::render_horizontal_rule(
            frame,
            Rect {
                x:      outer_area.x,
                y:      abs_y,
                width:  outer_area.width,
                height: 1,
            },
            rule_style,
            Some(pane::RuleTitle {
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
}

/// Compute the displayed value string for a flat git-pane row,
/// including row-specific decorations (local-only suffix on `Branch`,
/// `(github unreachable)` / `(github rate-limited)` suffix on
/// rate-limit rows).
fn build_field_value(data: &GitData, field: DetailField, is_rate_limit_row: bool) -> String {
    if field == DetailField::Branch {
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
    raw
}

const fn github_status_suffix(status: AvailabilityStatus) -> Option<&'static str> {
    match status {
        AvailabilityStatus::Reachable => None,
        AvailabilityStatus::Unreachable => Some("github unreachable"),
        AvailabilityStatus::RateLimited => Some("github rate-limited"),
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
    } = *args;
    for (i, field) in fields.iter().enumerate() {
        accum.row_line_ys.push(accum.lines.len());
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            *accum.focused_output_line = accum.lines.len();
        }
        let dynamic_label;
        let label = match *field {
            DetailField::VsLocal => {
                let branch = data.main_branch_label.as_str();
                dynamic_label = format!("vs local {branch}");
                &dynamic_label
            },
            _ => field.label(),
        };
        let is_rate_limit_row = matches!(
            *field,
            DetailField::RateLimitCore | DetailField::RateLimitGraphQl
        );
        let value = build_field_value(data, *field, is_rate_limit_row);
        let selection = pane.selection_state(i, focus);
        let base_value_style = if matches!(*field, DetailField::VsLocal) && value == IN_SYNC {
            Style::default().fg(SUCCESS_COLOR)
        } else if matches!(*field, DetailField::VsLocal)
            && value == crate::constants::NO_REMOTE_SYNC
        {
            Style::default().fg(INACTIVE_BORDER_COLOR)
        } else if *field == DetailField::WorktreeError {
            Style::default().fg(Color::White).bg(ERROR_COLOR)
        } else if is_rate_limit_row && !data.github_status.is_available() {
            Style::default().fg(ERROR_COLOR)
        } else {
            Style::default()
        };
        let ls = selection.patch(styles.readonly_label);
        let vs = selection.patch(base_value_style);
        if matches!(
            *field,
            DetailField::Branch | DetailField::RepoDesc | DetailField::WorktreeError
        ) && !value.is_empty()
        {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area_width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.width() > avail {
                let wrapped =
                    if matches!(*field, DetailField::RepoDesc | DetailField::WorktreeError) {
                        package::word_wrap(&value, avail)
                    } else {
                        package::hard_wrap(&value, avail)
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
                let reset_style = selection.patch(Style::default().fg(INACTIVE_BORDER_COLOR));
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

struct RemoteColWidths {
    name:    usize,
    url:     usize,
    tracked: usize,
    status:  usize,
}

/// The icon column pads to this display width. Emoji render as 2 cells on
/// most terminals; we append a trailing space for separation, giving 3.
const REMOTE_ICON_COL: usize = 3;
const REMOTES_NAME_HEADER: &str = "Remote";
const REMOTES_URL_HEADER: &str = "URL";
const REMOTES_TRACKED_HEADER: &str = "Tracked";
const REMOTES_STATUS_HEADER: &str = "Status";

fn remote_col_widths(remotes: &[RemoteRow]) -> RemoteColWidths {
    let name = remotes
        .iter()
        .map(|r| r.name.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_NAME_HEADER.width());
    let url = remotes
        .iter()
        .map(|r| r.display_url.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_URL_HEADER.width());
    let tracked = remotes
        .iter()
        .map(|r| r.tracked_ref.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_TRACKED_HEADER.width());
    let status = remotes
        .iter()
        .map(|r| r.status.width())
        .max()
        .unwrap_or(0)
        .max(REMOTES_STATUS_HEADER.width());
    RemoteColWidths {
        name,
        url,
        tracked,
        status,
    }
}

fn render_remote_header(lines: &mut Vec<Line<'static>>, widths: &RemoteColWidths) {
    let style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);
    // Leading: 1 space pad + REMOTE_ICON_COL blank for icon alignment.
    let text = format!(
        " {:<icon$}{:<name$}  {:<url$}  {:<tracked$}  {:<status$}",
        "",
        REMOTES_NAME_HEADER,
        REMOTES_URL_HEADER,
        REMOTES_TRACKED_HEADER,
        REMOTES_STATUS_HEADER,
        icon = REMOTE_ICON_COL,
        name = widths.name,
        url = widths.url,
        tracked = widths.tracked,
        status = widths.status,
    );
    lines.push(Line::from(Span::styled(text, style)));
}

fn remote_row_line(
    row: &RemoteRow,
    widths: &RemoteColWidths,
    selection: PaneSelectionState,
) -> Line<'static> {
    // Icon cell: emoji + trailing spaces to reach REMOTE_ICON_COL width.
    let icon_width = row.icon.width();
    let icon_pad = REMOTE_ICON_COL.saturating_sub(icon_width);
    let icon_cell = format!("{}{}", row.icon, " ".repeat(icon_pad));
    let text = format!(
        "{:<name$}  {:<url$}  {:<tracked$}  {:<status$}",
        row.name,
        row.display_url,
        row.tracked_ref,
        row.status,
        name = widths.name,
        url = widths.url,
        tracked = widths.tracked,
        status = widths.status,
    );
    let data_style = selection.patch(Style::default().fg(INACTIVE_TITLE_COLOR));
    let icon_style = selection.patch(Style::default());
    Line::from(vec![
        Span::raw(" ".to_string()),
        Span::styled(icon_cell, icon_style),
        Span::styled(text, data_style),
    ])
}

// ── Worktrees table ──────────────────────────────────────────────────

struct WorktreeColWidths {
    name:   usize,
    branch: usize,
    status: usize,
}

const WORKTREES_NAME_HEADER: &str = "Name";
const WORKTREES_BRANCH_HEADER: &str = "Branch";
const WORKTREES_STATUS_HEADER: &str = "Status";

fn worktree_col_widths(worktrees: &[WorktreeInfo]) -> WorktreeColWidths {
    let name = worktrees
        .iter()
        .map(|w| w.name.width())
        .max()
        .unwrap_or(0)
        .max(WORKTREES_NAME_HEADER.width());
    let branch = worktrees
        .iter()
        .map(|w| w.branch.as_deref().unwrap_or("").width())
        .max()
        .unwrap_or(0)
        .max(WORKTREES_BRANCH_HEADER.width());
    let status = worktrees
        .iter()
        .map(|w| worktree_status_text(w.ahead_behind).width())
        .max()
        .unwrap_or(0)
        .max(WORKTREES_STATUS_HEADER.width());
    WorktreeColWidths {
        name,
        branch,
        status,
    }
}

fn render_worktree_header(lines: &mut Vec<Line<'static>>, widths: &WorktreeColWidths) {
    let style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);
    let text = format!(
        " {:<name$}  {:<branch$}  {:<status$}",
        WORKTREES_NAME_HEADER,
        WORKTREES_BRANCH_HEADER,
        WORKTREES_STATUS_HEADER,
        name = widths.name,
        branch = widths.branch,
        status = widths.status,
    );
    lines.push(Line::from(Span::styled(text, style)));
}

fn worktree_row_line(
    row: &WorktreeInfo,
    widths: &WorktreeColWidths,
    selection: PaneSelectionState,
) -> Line<'static> {
    let branch = row.branch.clone().unwrap_or_else(|| "-".to_string());
    let status = worktree_status_text(row.ahead_behind);
    let text = format!(
        " {:<name$}  {:<branch$}  {:<status$}",
        row.name,
        branch,
        status,
        name = widths.name,
        branch = widths.branch,
        status = widths.status,
    );
    let style = selection.patch(Style::default().fg(INACTIVE_TITLE_COLOR));
    Line::from(Span::styled(text, style))
}

fn worktree_status_text(ahead_behind: Option<(usize, usize)>) -> String {
    match ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{}{a}", crate::constants::SYNC_UP),
        Some((0, b)) => format!("{}{b}", crate::constants::SYNC_DOWN),
        Some((a, b)) => format!(
            "{}{a} {}{b}",
            crate::constants::SYNC_UP,
            crate::constants::SYNC_DOWN
        ),
        None => crate::constants::NO_REMOTE_SYNC.to_string(),
    }
}

fn git_panel_title(data: &GitData) -> String {
    match data.branch.as_deref() {
        Some(branch) if !branch.is_empty() => format!(" Git - {branch} "),
        _ => pane::pane_title("Git", &PaneTitleCount::None),
    }
}

/// Render the Git info panel as a standalone pane.
pub fn render_git_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let styles = RenderStyles {
        readonly_label: Style::default().fg(LABEL_COLOR),
        chrome:         pane::default_pane_chrome(),
    };

    let Some(git_data) = app.pane_data().git().cloned() else {
        app.panes_mut().git_mut().viewport_mut().clear_surface();
        let empty = pane::empty_pane_block(pane::pane_title("Git", &PaneTitleCount::None));
        frame.render_widget(empty, area);
        return;
    };

    let flat_fields = panes::git_fields_from_data(&git_data);
    let total_rows = flat_fields.len() + git_data.remotes.len() + git_data.worktrees.len();
    if total_rows == 0 && git_data.description.as_deref().is_none_or(str::is_empty) {
        app.panes_mut().git_mut().viewport_mut().clear_surface();
        let empty_git = pane::empty_pane_block(" Not a git repo ");
        frame.render_widget(empty_git, area);
        return;
    }

    app.panes_mut().git_mut().viewport_mut().set_len(total_rows);
    let focus = app.pane_focus_state(PaneId::Git);
    let border_style = if matches!(focus, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let git_block = styles.chrome.block(
        git_panel_title(&git_data),
        matches!(focus, PaneFocusState::Active),
    );
    let git_inner = git_block.inner(area);
    frame.render_widget(git_block, area);

    let content_area = render_git_about_section(
        frame,
        git_data.description.as_deref(),
        &flat_fields,
        &git_data,
        area,
        git_inner,
        border_style,
    );

    {
        let viewport = app.panes_mut().git_mut().viewport_mut();
        viewport.set_content_area(content_area);
        viewport.set_viewport_rows(usize::from(content_area.height));
    }
    let git_ctx = GitRenderCtx {
        data: &git_data,
        fields: &flat_fields,
        pane: app.panes().git().viewport(),
        focus,
        styles: &styles,
    };
    let layout = render_git_column_inner(frame, &git_ctx, area, content_area);
    app.panes_mut()
        .git_mut()
        .viewport_mut()
        .set_scroll_offset(layout.scroll_offset);
    register_git_row_hitboxes(app, content_area, &layout);
    pane::render_overflow_affordance(frame, area, app.panes().git().viewport());
}

/// Render the About section (repo description) at the top of the Git panel,
/// separated from the rest of the pane by a horizontal rule with `├`/`┤`
/// endcaps. Returns the area below the separator for the scrolling content.
fn render_git_about_section(
    frame: &mut Frame,
    description: Option<&str>,
    flat_fields: &[DetailField],
    data: &GitData,
    outer_area: Rect,
    git_inner: Rect,
    border_style: Style,
) -> Rect {
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    let Some(description) = description else {
        return git_inner;
    };

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
    let lower_content_height = flat_fields.len() + remotes_block + worktrees_block;
    let reserved_lower_height = u16::try_from(lower_content_height).unwrap_or(u16::MAX);
    let reserved_separator_height = u16::from(git_inner.height > reserved_lower_height);
    let description_max_height = git_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));
    if description_max_height == 0 {
        return git_inner;
    }

    let description_padding = u16::from(git_inner.width > 2);
    let description_width = git_inner
        .width
        .saturating_sub(description_padding.saturating_mul(2));
    let lines =
        package::description_lines(Some(description), description_width, description_max_height);
    let description_height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    if description_height == 0 {
        return git_inner;
    }

    let description_area = Rect {
        x:      git_inner.x.saturating_add(description_padding),
        y:      git_inner.y,
        width:  description_width,
        height: description_height,
    };
    frame.render_widget(Paragraph::new(lines), description_area);

    let separator_y = git_inner.y.saturating_add(description_height);
    let has_room_for_separator = separator_y < git_inner.bottom();
    if !has_room_for_separator {
        return Rect {
            x:      git_inner.x,
            y:      separator_y,
            width:  git_inner.width,
            height: 0,
        };
    }

    pane::render_rules(
        frame,
        &[pane::PaneRule::Horizontal {
            area:        Rect {
                x:      outer_area.x,
                y:      separator_y,
                width:  outer_area.width,
                height: 1,
            },
            connector_x: None,
        }],
        border_style,
    );

    let content_y = separator_y.saturating_add(1);
    Rect {
        x:      git_inner.x,
        y:      content_y,
        width:  git_inner.width,
        height: git_inner.bottom().saturating_sub(content_y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_for_pos_maps_flat_indices() {
        assert_eq!(section_for_pos(0, 3, 2, 1), Some(Section::Flat(0)));
        assert_eq!(section_for_pos(2, 3, 2, 1), Some(Section::Flat(2)));
    }

    #[test]
    fn section_for_pos_maps_remote_indices() {
        assert_eq!(section_for_pos(3, 3, 2, 1), Some(Section::Remote(0)));
        assert_eq!(section_for_pos(4, 3, 2, 1), Some(Section::Remote(1)));
    }

    #[test]
    fn section_for_pos_maps_worktree_indices() {
        assert_eq!(section_for_pos(5, 3, 2, 1), Some(Section::Worktree(0)));
    }

    #[test]
    fn section_for_pos_out_of_range_is_none() {
        assert_eq!(section_for_pos(6, 3, 2, 1), None);
        assert_eq!(section_for_pos(0, 0, 0, 0), None);
    }
}
