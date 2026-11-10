use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use tui_pane::PaneGridLayout;
use tui_pane::PanePlacement;
use tui_pane::ResolvedPane;
use tui_pane::ResolvedPaneLayout;
use unicode_width::UnicodeWidthStr;

use super::constants::CPU_PANE_WIDTH;
use super::constants::PANE_BORDER_HEIGHT;
use super::cpu;
use super::lang;
use super::spec::PaneId;
use super::targets;
use crate::tui::columns::ProjectColumnLayout;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum BottomRow {
    Diagnostics,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum PaneVisibility {
    BeforeFirstDraw,
    Hidden,
    Visible,
}

impl PaneVisibility {
    pub(in crate::tui) fn in_layout(layout: &ResolvedPaneLayout<PaneId>, pane: PaneId) -> Self {
        if layout.panes.is_empty() {
            Self::BeforeFirstDraw
        } else if layout.area(pane).is_empty() {
            Self::Hidden
        } else {
            Self::Visible
        }
    }
}

pub(super) fn derived_layout(bottom_row: BottomRow) -> PaneGridLayout<PaneId> {
    let mut placements = vec![
        PanePlacement {
            pane:     PaneId::ProjectList,
            row:      0,
            col:      0,
            row_span: 2,
            col_span: 1,
        },
        PanePlacement {
            pane:     PaneId::Package,
            row:      0,
            col:      1,
            row_span: 1,
            col_span: 1,
        },
        PanePlacement {
            pane:     PaneId::Git,
            row:      0,
            col:      2,
            row_span: 1,
            col_span: 1,
        },
        PanePlacement {
            pane:     PaneId::Lang,
            row:      1,
            col:      1,
            row_span: 1,
            col_span: 1,
        },
        PanePlacement {
            pane:     PaneId::Cpu,
            row:      1,
            col:      2,
            row_span: 1,
            col_span: 1,
        },
        PanePlacement {
            pane:     PaneId::Targets,
            row:      1,
            col:      3,
            row_span: 1,
            col_span: 1,
        },
    ];

    match bottom_row {
        BottomRow::Diagnostics => {
            placements.push(PanePlacement {
                pane:     PaneId::Lints,
                row:      2,
                col:      0,
                row_span: 1,
                col_span: 1,
            });
            placements.push(PanePlacement {
                pane:     PaneId::CiRuns,
                row:      2,
                col:      1,
                row_span: 1,
                col_span: 3,
            });
        },
        BottomRow::Output => {
            placements.push(PanePlacement {
                pane:     PaneId::Output,
                row:      2,
                col:      0,
                row_span: 1,
                col_span: 4,
            });
        },
    }

    PaneGridLayout { placements }
}

pub(in crate::tui) fn tab_order(bottom_row: BottomRow) -> Vec<PaneId> {
    derived_layout(bottom_row).tab_order()
}

pub(in crate::tui) fn resolve_layout(
    area: Rect,
    project_column: ProjectColumnLayout,
    core_count: usize,
    bottom_row: BottomRow,
    top_required_inner: u16,
    targets_preferred_width: u16,
) -> ResolvedPaneLayout<PaneId> {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(tiled_row_constraints(
            core_count,
            area.height,
            top_required_inner,
        ))
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(project_column_width(
                area.width,
                project_column,
                targets_preferred_width,
            )),
            Constraint::Min(0),
        ])
        .split(area);

    let panes = derived_layout(bottom_row)
        .placements
        .into_iter()
        .map(|placement| ResolvedPane {
            pane: placement.pane,
            area: resolve_pane_area(
                rows.as_ref(),
                cols.as_ref(),
                placement.pane,
                bottom_row,
                targets_preferred_width,
            ),
        })
        .collect();

    ResolvedPaneLayout::new(panes)
}

fn resolve_pane_area(
    rows: &[Rect],
    cols: &[Rect],
    pane: PaneId,
    bottom_row: BottomRow,
    targets_preferred_width: u16,
) -> Rect {
    let project_col = cols[0];
    let right_col = cols[1];
    let top_right_area = Rect::new(right_col.x, rows[0].y, right_col.width, rows[0].height);
    let middle_right_area = Rect::new(right_col.x, rows[1].y, right_col.width, rows[1].height);
    let top_right = top_pane_areas(top_right_area);
    let middle_panes = middle_pane_constraints(middle_right_area.width, targets_preferred_width);
    let middle_areas =
        Layout::horizontal(middle_panes.iter().map(|(_, size)| *size)).split(middle_right_area);
    let middle_area = |id| {
        middle_panes
            .iter()
            .position(|(pane, _)| *pane == id)
            .map_or(Rect::ZERO, |index| middle_areas[index])
    };

    match pane {
        PaneId::ProjectList => Rect::new(
            project_col.x,
            rows[0].y,
            project_col.width,
            rows[1]
                .y
                .saturating_add(rows[1].height)
                .saturating_sub(rows[0].y),
        ),
        PaneId::Package => top_right[0],
        PaneId::Git => top_right[1],
        PaneId::Lang => middle_area(PaneId::Lang),
        PaneId::Cpu => middle_area(PaneId::Cpu),
        PaneId::Targets => middle_area(PaneId::Targets),
        PaneId::Lints => rows[2].intersection(project_col),
        PaneId::CiRuns => rows[2].intersection(right_col),
        PaneId::Output if matches!(bottom_row, BottomRow::Output) => rows[2],
        PaneId::Output
        | PaneId::Toasts
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap
        | PaneId::Sccache => Rect::ZERO,
    }
}

fn middle_pane_constraints(width: u16, targets_preferred_width: u16) -> Vec<(PaneId, Constraint)> {
    let targets_min = targets::minimum_width();
    let cpu_width = cpu_column_width();
    let show_targets = width >= targets_min;
    let targets_width = if show_targets { targets_min } else { 0 };
    let show_cpu = show_targets && width >= targets_width.saturating_add(cpu_width);
    let occupied = targets_width.saturating_add(if show_cpu { cpu_width } else { 0 });
    let language_min = lang::minimum_width();
    let show_languages = width >= occupied.saturating_add(language_min);
    let mut middle_panes = Vec::new();
    if show_languages {
        middle_panes.push((PaneId::Lang, Constraint::Min(language_min)));
    }
    if show_cpu {
        middle_panes.push((PaneId::Cpu, Constraint::Length(cpu_width)));
    }
    if show_targets {
        middle_panes.push((
            PaneId::Targets,
            Constraint::Min(
                targets_preferred_width.max(targets_min).min(
                    width
                        .saturating_sub(if show_cpu { cpu_width } else { 0 })
                        .saturating_sub(if show_languages { language_min } else { 0 }),
                ),
            ),
        ));
    }
    middle_panes
}

/// Heights for the three tiled rows. The top row (Details/Git) is sized to the
/// tallest project's content (`top_required_inner`, measured across all
/// projects, plus the pane border) so the middle row (Lang/CPU/Targets) grows
/// into the space the previous fixed split left empty above it. The top is
/// capped so the middle always keeps at least the CPU pane's required height,
/// and the bottom row keeps the size it had under the previous fixed-middle
/// split. On a screen too small for all three, fall back to proportional rows.
fn tiled_row_constraints(
    core_count: usize,
    total_height: u16,
    top_required_inner: u16,
) -> [Constraint; 3] {
    let cpu_floor = cpu::cpu_required_pane_height(core_count);
    let minimum_outer_rows = 8;
    let top_content = top_required_inner.saturating_add(PANE_BORDER_HEIGHT);

    // `bottom` preserves the old bottom-row height: with the middle pinned to `cpu_floor`, the old
    // 35:25 top/bottom split gave the bottom 25/60 of `prior_slack`.
    let prior_slack = total_height.saturating_sub(cpu_floor);
    let bottom = prior_slack.saturating_mul(25) / 60;

    let reserved = cpu_floor
        .saturating_add(bottom)
        .saturating_add(minimum_outer_rows);
    if total_height >= reserved {
        // Clamp `top_content` to `max_top` so the middle row between `top` and `bottom` never falls
        // below `cpu_floor`.
        let max_top = total_height
            .saturating_sub(cpu_floor)
            .saturating_sub(bottom);
        let top = top_content.clamp(minimum_outer_rows, max_top);
        [
            Constraint::Length(top),
            Constraint::Fill(1),
            Constraint::Length(bottom),
        ]
    } else {
        [
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ]
    }
}

fn project_column_width(
    window_width: u16,
    project_column: ProjectColumnLayout,
    targets_preferred_width: u16,
) -> u16 {
    let natural = project_column
        .preferred_width
        .min((window_width / 2).max(project_column.minimum_width))
        .min(crate::tui::columns::ProjectListWidths::pane_width_limit(
            window_width,
            project_column.lint_column,
        ));
    let preferred_middle = targets_preferred_width
        .max(targets::minimum_width())
        .saturating_add(cpu_column_width())
        .saturating_add(lang::minimum_width());
    if targets_preferred_width > targets::minimum_width()
        && window_width
            >= project_column
                .minimum_width
                .saturating_add(preferred_middle)
    {
        natural.min(window_width.saturating_sub(preferred_middle))
    } else {
        natural
    }
}

const fn cpu_column_width() -> u16 { CPU_PANE_WIDTH }

/// Outer widths of the two top-row panes (Details and Git) for the given outer
/// area and project-list column width. The cross-project top-row height
/// measurement wraps each pane's description to these widths, and
/// [`resolve_pane_area`] lays the panes out at the same widths, so the
/// measured height matches the rendered layout.
pub(in crate::tui) fn top_pane_widths(
    area: Rect,
    project_column: ProjectColumnLayout,
    targets_preferred_width: u16,
) -> (u16, u16) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(project_column_width(
                area.width,
                project_column,
                targets_preferred_width,
            )),
            Constraint::Min(0),
        ])
        .split(area);
    let right_col = cols[1];
    let top_right = top_pane_areas(Rect::new(right_col.x, area.y, right_col.width, area.height));
    (top_right[0].width, top_right[1].width)
}

fn minimum_git_width_for_whole_core_label() -> u16 {
    let row_margins = "  …".width();
    u16::try_from(2 * "Core".width() + row_margins + tui_pane::BLOCK_BORDER_WIDTH)
        .unwrap_or(u16::MAX)
}

fn top_pane_areas(area: Rect) -> [Rect; 2] {
    let minimum = minimum_git_width_for_whole_core_label();
    if area.width < minimum {
        return [Rect::ZERO; 2];
    }
    if area.width < minimum.saturating_mul(2) {
        return [Rect::ZERO, area];
    }
    let areas = Layout::horizontal(tui_pane::constraints_for_sizes(&[
        super::size_spec(PaneId::Package, CPU_PANE_WIDTH).width,
        super::size_spec(PaneId::Git, CPU_PANE_WIDTH).width,
    ]))
    .split(area);
    [areas[0], areas[1]]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use ratatui::layout::Rect;
    use tui_pane::PaneGridLayout;
    use tui_pane::PanePlacement;

    use super::BottomRow;
    use super::PANE_BORDER_HEIGHT;
    use super::ProjectColumnLayout;
    use super::derived_layout;
    use super::resolve_layout;
    use crate::tui::columns::LintColumnStatus;
    use crate::tui::panes::PaneId;

    #[test]
    fn tiled_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&derived_layout(BottomRow::Diagnostics));
    }

    #[test]
    fn output_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&derived_layout(BottomRow::Output));
    }

    #[test]
    fn derived_output_layout_keeps_cpu_between_lang_and_targets() {
        let order = derived_layout(BottomRow::Output).tab_order();
        assert_eq!(
            order,
            vec![
                PaneId::ProjectList,
                PaneId::Package,
                PaneId::Git,
                PaneId::Lang,
                PaneId::Cpu,
                PaneId::Targets,
                PaneId::Output,
            ]
        );
    }

    #[test]
    fn tab_order_is_derived_from_grid_position() {
        let layout = PaneGridLayout {
            placements: vec![
                PanePlacement {
                    pane:     PaneId::Targets,
                    row:      1,
                    col:      2,
                    row_span: 1,
                    col_span: 1,
                },
                PanePlacement {
                    pane:     PaneId::ProjectList,
                    row:      0,
                    col:      0,
                    row_span: 2,
                    col_span: 1,
                },
                PanePlacement {
                    pane:     PaneId::Git,
                    row:      0,
                    col:      2,
                    row_span: 1,
                    col_span: 1,
                },
                PanePlacement {
                    pane:     PaneId::Package,
                    row:      0,
                    col:      1,
                    row_span: 1,
                    col_span: 1,
                },
            ],
        };

        assert_eq!(
            layout.tab_order(),
            vec![
                PaneId::ProjectList,
                PaneId::Package,
                PaneId::Git,
                PaneId::Targets,
            ]
        );
    }

    #[test]
    fn resolved_layout_keeps_top_row_flush_with_targets() {
        let layout = resolve_layout(
            Rect::new(0, 0, 120, 30),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            20,
            super::targets::minimum_width(),
        );
        let package = layout.area(PaneId::Package);
        let git = layout.area(PaneId::Git);
        let targets = layout.area(PaneId::Targets);
        let right = Rect::new(30, 0, 90, 30);

        assert_eq!(package.x, right.x);
        assert_eq!(
            git.x.saturating_add(git.width),
            right.x.saturating_add(right.width)
        );
        assert_eq!(package.width.saturating_add(git.width), right.width);
        assert_eq!(
            targets.x.saturating_add(targets.width),
            right.x.saturating_add(right.width)
        );
    }

    #[test]
    fn resolved_layout_floors_cpu_at_required_height_when_top_is_tall() {
        // `resolve_layout` caps an oversized top row so `PaneId::Cpu` retains at least
        // `cpu::cpu_required_pane_height(12)`.
        let layout = resolve_layout(
            Rect::new(0, 0, 120, 40),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            100,
            super::targets::minimum_width(),
        );

        assert_eq!(
            layout.area(PaneId::Cpu).height,
            super::cpu::cpu_required_pane_height(12)
        );
    }

    #[test]
    fn resolved_layout_sizes_top_row_to_content_when_it_fits() {
        // With room to spare, `resolve_layout` gives `PaneId::Package` exactly `top_inner +
        // PANE_BORDER_HEIGHT`, leaving the remaining height to the middle row.
        let top_inner = 8;
        let layout = resolve_layout(
            Rect::new(0, 0, 120, 41),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            top_inner,
            super::targets::minimum_width(),
        );

        assert_eq!(
            layout.area(PaneId::Package).height,
            top_inner + PANE_BORDER_HEIGHT
        );
    }

    #[test]
    fn resolved_layout_grows_middle_when_top_content_is_short() {
        // `short` gives `PaneId::Cpu` more than `cpu_floor` and `PaneId::Targets` more height than
        // `tall`, because its shorter top row frees middle-row space.
        let cpu_floor = super::cpu::cpu_required_pane_height(12);
        let tall_resolved_pane_layout = resolve_layout(
            Rect::new(0, 0, 120, 40),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            100,
            super::targets::minimum_width(),
        );
        let short_resolved_pane_layout = resolve_layout(
            Rect::new(0, 0, 120, 40),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            4,
            super::targets::minimum_width(),
        );

        assert!(short_resolved_pane_layout.area(PaneId::Cpu).height > cpu_floor);
        assert!(
            short_resolved_pane_layout.area(PaneId::Targets).height
                > tall_resolved_pane_layout.area(PaneId::Targets).height
        );
    }

    fn assert_layout_has_no_overlaps(layout: &PaneGridLayout<PaneId>) {
        let mut occupied = HashSet::new();
        for placement in &layout.placements {
            for row in placement.row..placement.row + placement.row_span {
                for col in placement.col..placement.col + placement.col_span {
                    assert!(occupied.insert((row, col)));
                }
            }
        }
    }
}
