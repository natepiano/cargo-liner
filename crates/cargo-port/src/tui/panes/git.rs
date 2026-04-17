use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::PaneId;
use super::package;
use super::package::RenderStyles;
use crate::constants::IN_SYNC;
use crate::constants::NO_CI_UNPUBLISHED_BRANCH;
use crate::tui::app::App;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::detail;
use crate::tui::detail::DetailField;
use crate::tui::detail::GitData;
use crate::tui::pane;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneTitleCount;

struct GitRenderCtx<'a> {
    data:   &'a GitData,
    fields: &'a [DetailField],
    pane:   &'a Pane,
    focus:  PaneFocusState,
    styles: &'a RenderStyles,
}

pub(in super::super) fn git_label_width(data: &GitData, fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| match *field {
            DetailField::VsOrigin => "Remote".width(),
            DetailField::VsLocal => format!("vs local {}", data.main_branch_label).width(),
            _ => field.label().width(),
        })
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_git_column_inner(frame: &mut Frame, ctx: &GitRenderCtx<'_>, area: Rect) -> usize {
    let data = ctx.data;
    let fields = ctx.fields;
    let pane = ctx.pane;
    let focus = ctx.focus;
    let styles = ctx.styles;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = git_label_width(data, fields);

    for (i, field) in fields.iter().enumerate() {
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            focused_output_line = lines.len();
        }
        let dynamic_label;
        let label = match *field {
            DetailField::VsOrigin => {
                dynamic_label = "Remote".to_string();
                &dynamic_label
            },
            DetailField::VsLocal => {
                let branch = data.main_branch_label.as_str();
                dynamic_label = format!("vs local {branch}");
                &dynamic_label
            },
            _ => field.label(),
        };
        let value = field.git_value(data);
        let selection = pane.selection_state(i, focus);
        let base_value_style = if *field == DetailField::Origin && value.starts_with('⑂') {
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD)
        } else if matches!(
            *field,
            DetailField::Sync | DetailField::VsOrigin | DetailField::VsLocal
        ) && value == IN_SYNC
        {
            Style::default().fg(SUCCESS_COLOR)
        } else if (*field == DetailField::VsOrigin && value == NO_CI_UNPUBLISHED_BRANCH)
            || (*field == DetailField::Sync && value == crate::constants::NO_REMOTE_SYNC)
        {
            Style::default().fg(INACTIVE_BORDER_COLOR)
        } else if *field == DetailField::WorktreeError {
            Style::default().fg(Color::White).bg(ERROR_COLOR)
        } else {
            Style::default()
        };
        let ls = selection.patch(styles.readonly_label);
        let vs = selection.patch(base_value_style);
        if matches!(
            *field,
            DetailField::Repo
                | DetailField::Branch
                | DetailField::RepoDesc
                | DetailField::VsOrigin
                | DetailField::WorktreeError
        ) && !value.is_empty()
        {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
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
                    Span::styled(prefix, ls),
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

    append_worktree_lines(&mut lines, &ctx.data.worktree_names);

    let scroll_y = package::detail_column_scroll_offset(focus, focused_output_line, area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    usize::from(scroll_y)
}

fn append_worktree_lines(lines: &mut Vec<Line<'static>>, worktree_names: &[String]) {
    if worktree_names.is_empty() {
        return;
    }
    let count = worktree_names.len();
    let label_style = Style::default().fg(LABEL_COLOR);
    let value_style = Style::default().fg(TITLE_COLOR);
    lines.push(Line::from(vec![
        Span::styled("  Worktrees  ", label_style),
        Span::styled(count.to_string(), value_style),
    ]));
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

    let Some(git_data) = app.pane_data().git.clone() else {
        app.pane_manager_mut().pane_mut(PaneId::Git).clear_surface();
        let empty = pane::empty_pane_block(pane::pane_title("Git", &PaneTitleCount::None));
        frame.render_widget(empty, area);
        return;
    };

    let git = detail::git_fields_from_data(&git_data);
    if git.is_empty() {
        app.pane_manager_mut().pane_mut(PaneId::Git).clear_surface();
        let empty_git = pane::empty_pane_block(" Not a git repo ");
        frame.render_widget(empty_git, area);
        return;
    }

    app.pane_manager_mut()
        .pane_mut(PaneId::Git)
        .set_len(git.len());
    let focus = app.pane_focus_state(PaneId::Git);
    let git_block = styles.chrome.block(
        git_panel_title(&git_data),
        matches!(focus, PaneFocusState::Active),
    );
    let git_inner = git_block.inner(area);
    app.pane_manager_mut()
        .pane_mut(PaneId::Git)
        .set_content_area(git_inner);
    frame.render_widget(git_block, area);
    let git_ctx = GitRenderCtx {
        data: &git_data,
        fields: &git,
        pane: app.pane_manager().pane(PaneId::Git),
        focus,
        styles: &styles,
    };
    let scroll_offset = render_git_column_inner(frame, &git_ctx, git_inner);
    app.pane_manager_mut()
        .pane_mut(PaneId::Git)
        .set_scroll_offset(scroll_offset);
}
