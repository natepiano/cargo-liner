use ratatui::layout::Rect;

use super::spec::PaneId;
use crate::tui::interaction::UiHitbox;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum PaneAxisSize {
    Fixed(u16),
    Fill(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct PaneSizeSpec {
    pub(in super::super) width:  PaneAxisSize,
    pub(in super::super) height: PaneAxisSize,
}

impl PaneSizeSpec {
    pub(in super::super) const fn fill() -> Self {
        Self {
            width:  PaneAxisSize::Fill(1),
            height: PaneAxisSize::Fill(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct PanePlacement {
    pub(in super::super) pane:     PaneId,
    pub(in super::super) row:      usize,
    pub(in super::super) col:      usize,
    pub(in super::super) row_span: usize,
    pub(in super::super) col_span: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct PaneGridLayout {
    pub(in super::super) placements: Vec<PanePlacement>,
}

impl PaneGridLayout {
    pub(in super::super) fn tab_order(self) -> Vec<PaneId> {
        let mut placements = self.placements;
        placements.sort_by_key(|placement| (placement.row, placement.col));
        placements
            .into_iter()
            .map(|placement| placement.pane)
            .collect()
    }
}

#[derive(Default)]
pub(in super::super) struct LayoutCache {
    pub(in super::super) project_list: Rect,
    pub(in super::super) pane_regions: Vec<(PaneId, Rect)>,
    pub(in super::super) ui_hitboxes:  Vec<UiHitbox>,
}

pub(in super::super) fn derived_layout(output_visible: bool) -> PaneGridLayout {
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

    if output_visible {
        placements.push(PanePlacement {
            pane:     PaneId::Output,
            row:      2,
            col:      0,
            row_span: 1,
            col_span: 4,
        });
    } else {
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
    }

    PaneGridLayout { placements }
}

pub(in super::super) fn tab_order(output_visible: bool) -> Vec<PaneId> {
    derived_layout(output_visible).tab_order()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PaneGridLayout;
    use super::PanePlacement;
    use super::derived_layout;
    use crate::tui::panes::PaneId;

    #[test]
    fn tiled_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&derived_layout(false));
    }

    #[test]
    fn output_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&derived_layout(true));
    }

    #[test]
    fn derived_output_layout_keeps_cpu_between_lang_and_targets() {
        let order = derived_layout(true).tab_order();
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

    fn assert_layout_has_no_overlaps(layout: &PaneGridLayout) {
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
