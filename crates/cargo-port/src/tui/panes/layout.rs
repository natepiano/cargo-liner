use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;

use super::cpu;
use super::cpu::CPU_PANE_WIDTH;
use super::spec::PaneId;
use crate::tui::pane;
use crate::tui::pane::PaneAxisSize;
use crate::tui::pane::PaneGridLayout;
use crate::tui::pane::PanePlacement;
use crate::tui::pane::ResolvedPane;
use crate::tui::pane::ResolvedPaneLayout;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomRow {
    Diagnostics,
    Output,
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

pub fn tab_order(bottom_row: BottomRow) -> Vec<PaneId> { derived_layout(bottom_row).tab_order() }

pub fn resolve_layout(
    area: Rect,
    left_width: u16,
    core_count: usize,
    bottom_row: BottomRow,
) -> ResolvedPaneLayout<PaneId> {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(tiled_row_constraints(core_count, area.height))
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(area);

    let panes = derived_layout(bottom_row)
        .placements
        .into_iter()
        .map(|placement| ResolvedPane {
            pane: placement.pane,
            area: resolve_pane_area(rows.as_ref(), cols.as_ref(), placement.pane, bottom_row),
        })
        .collect();

    ResolvedPaneLayout::new(panes)
}

fn resolve_pane_area(rows: &[Rect], cols: &[Rect], pane: PaneId, bottom_row: BottomRow) -> Rect {
    let project_col = cols[0];
    let right_col = cols[1];
    let top_right_area = Rect::new(right_col.x, rows[0].y, right_col.width, rows[0].height);
    let middle_right_area = Rect::new(right_col.x, rows[1].y, right_col.width, rows[1].height);
    let top_right = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(pane::constraints_for_sizes(&[
            super::size_spec(PaneId::Package, CPU_PANE_WIDTH).width,
            super::size_spec(PaneId::Git, CPU_PANE_WIDTH).width,
        ]))
        .split(top_right_area);
    let middle_right = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(pane::constraints_for_sizes(&[
            super::size_spec(PaneId::Lang, CPU_PANE_WIDTH).width,
            PaneAxisSize::Fixed(cpu_column_width()),
            super::size_spec(PaneId::Targets, CPU_PANE_WIDTH).width,
        ]))
        .split(middle_right_area);

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
        PaneId::Lang => middle_right[0],
        PaneId::Cpu => middle_right[1],
        PaneId::Targets => middle_right[2],
        PaneId::Lints => rows[2].intersection(project_col),
        PaneId::CiRuns => rows[2].intersection(right_col),
        PaneId::Output if matches!(bottom_row, BottomRow::Output) => rows[2],
        PaneId::Output | PaneId::Toasts | PaneId::Settings | PaneId::Finder | PaneId::Keymap => {
            Rect::ZERO
        },
    }
}

fn tiled_row_constraints(core_count: usize, total_height: u16) -> [Constraint; 3] {
    let desired_middle = cpu::cpu_required_pane_height(core_count);
    let minimum_outer_rows = 8;

    if total_height >= desired_middle.saturating_add(minimum_outer_rows) {
        [
            Constraint::Fill(35),
            Constraint::Length(desired_middle),
            Constraint::Fill(25),
        ]
    } else {
        [
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ]
    }
}

const fn cpu_column_width() -> u16 { CPU_PANE_WIDTH }

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use ratatui::layout::Rect;

    use super::BottomRow;
    use super::derived_layout;
    use super::resolve_layout;
    use crate::tui::pane::PaneGridLayout;
    use crate::tui::pane::PanePlacement;
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
        let layout = resolve_layout(Rect::new(0, 0, 120, 30), 30, 12, BottomRow::Diagnostics);
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
    fn resolved_layout_gives_cpu_its_required_height_when_room_exists() {
        let layout = resolve_layout(Rect::new(0, 0, 120, 40), 30, 12, BottomRow::Diagnostics);

        assert_eq!(
            layout.area(PaneId::Cpu).height,
            super::cpu::cpu_required_pane_height(12)
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
