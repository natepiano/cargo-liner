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
use crate::project::LanguageStats;
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

/// What the Languages and Targets panes need from the tiled layout.
#[derive(Clone, Copy)]
pub(in crate::tui) struct MiddlePaneNeeds {
    /// The Languages width that draws every name of the selected project whole.
    lang_width:    u16,
    /// The Targets width that draws every name of the selected project whole.
    targets_width: u16,
    /// The inner rows of the Languages pane for the project that needs the most.
    lang_rows:     u16,
    /// The inner rows of the Targets pane for the project that needs the most.
    targets_rows:  u16,
}

impl MiddlePaneNeeds {
    /// The widths for the selected project. The rows start at zero.
    pub(in crate::tui) const fn new(lang_width: u16, targets_width: u16) -> Self {
        Self {
            lang_width,
            targets_width,
            lang_rows: 0,
            targets_rows: 0,
        }
    }

    pub(in crate::tui) fn minimum(targets_width: u16) -> Self {
        Self::new(lang::minimum_width(), targets_width)
    }

    pub(in crate::tui) fn for_selected(stats: &LanguageStats, targets_width: u16) -> Self {
        Self::new(lang::preferred_width(stats), targets_width)
    }

    /// Sets the rows of the project that needs the most in each pane.
    pub(in crate::tui) fn with_rows(self, most_language_rows: usize, targets_rows: u16) -> Self {
        Self {
            lang_rows: lang::inner_rows(most_language_rows),
            targets_rows,
            ..self
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
    middle: MiddlePaneNeeds,
) -> ResolvedPaneLayout<PaneId> {
    let split_rows = Layout::default()
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
            Constraint::Length(project_column_width(area.width, project_column, middle)),
            Constraint::Min(0),
        ])
        .split(area);
    let mut rows = [split_rows[0], split_rows[1], split_rows[2]];
    let middle_need = middle_pane_constraints(cols[1].width, middle)
        .iter()
        .map(|(pane, _)| match pane {
            PaneId::Lang => middle.lang_rows.saturating_add(PANE_BORDER_HEIGHT),
            PaneId::Targets => middle.targets_rows.saturating_add(PANE_BORDER_HEIGHT),
            _ => cpu::cpu_required_pane_height(core_count),
        })
        .max()
        .unwrap_or(0);
    give_free_middle_rows_to_top(
        &mut rows,
        top_required_inner.saturating_add(PANE_BORDER_HEIGHT),
        middle_need,
    );

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
                middle,
            ),
        })
        .collect();

    ResolvedPaneLayout::new(panes)
}

/// Moves the middle rows that no shown middle pane needs to the top row, up
/// to the rows the top row needs. The bottom row keeps its height.
fn give_free_middle_rows_to_top(rows: &mut [Rect; 3], top_need: u16, middle_need: u16) {
    let moved = top_need
        .saturating_sub(rows[0].height)
        .min(rows[1].height.saturating_sub(middle_need));
    rows[0].height = rows[0].height.saturating_add(moved);
    rows[1].y = rows[1].y.saturating_add(moved);
    rows[1].height = rows[1].height.saturating_sub(moved);
}

fn resolve_pane_area(
    rows: &[Rect],
    cols: &[Rect],
    pane: PaneId,
    bottom_row: BottomRow,
    middle: MiddlePaneNeeds,
) -> Rect {
    let project_col = cols[0];
    let right_col = cols[1];
    let top_right_area = Rect::new(right_col.x, rows[0].y, right_col.width, rows[0].height);
    let middle_right_area = Rect::new(right_col.x, rows[1].y, right_col.width, rows[1].height);
    let top_right = top_pane_areas(top_right_area);
    let middle_panes = middle_pane_constraints(middle_right_area.width, middle);
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

fn middle_pane_constraints(width: u16, middle: MiddlePaneNeeds) -> Vec<(PaneId, Constraint)> {
    let targets_min = targets::minimum_width();
    let cpu_width = cpu_column_width();
    let show_targets = width >= targets_min;
    let targets_floor = if show_targets { targets_min } else { 0 };
    let show_cpu = show_targets && width >= targets_floor.saturating_add(cpu_width);
    let cpu_taken = if show_cpu { cpu_width } else { 0 };
    let language_min = lang::minimum_width();
    let show_languages = width
        >= targets_floor
            .saturating_add(cpu_taken)
            .saturating_add(language_min);
    let mut middle_panes = Vec::new();
    if show_languages {
        let lang_width = language_width_before_targets_cut(width, cpu_taken, middle);
        middle_panes.push((PaneId::Lang, Constraint::Length(lang_width)));
        if show_cpu {
            middle_panes.push((PaneId::Cpu, Constraint::Length(cpu_width)));
        }
        middle_panes.push((
            PaneId::Targets,
            Constraint::Length(width.saturating_sub(cpu_taken).saturating_sub(lang_width)),
        ));
        return middle_panes;
    }
    if show_cpu {
        middle_panes.push((PaneId::Cpu, Constraint::Length(cpu_width)));
    }
    if show_targets {
        middle_panes.push((
            PaneId::Targets,
            Constraint::Min(
                middle
                    .targets_width
                    .max(targets_min)
                    .min(width.saturating_sub(cpu_taken)),
            ),
        ));
    }
    middle_panes
}

/// Give Languages name cells to Targets when the wide row cannot fit both.
fn language_width_before_targets_cut(width: u16, cpu_width: u16, middle: MiddlePaneNeeds) -> u16 {
    let language_min = lang::minimum_width();
    let targets_first = middle
        .targets_width
        .max(targets::minimum_width())
        .min(width.saturating_sub(cpu_width).saturating_sub(language_min));
    width
        .saturating_sub(cpu_width)
        .saturating_sub(targets_first)
        .min(middle.lang_width.max(language_min))
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
    middle: MiddlePaneNeeds,
) -> u16 {
    let natural = project_column
        .preferred_width
        .min((window_width / 2).max(project_column.minimum_width))
        .min(crate::tui::columns::ProjectListWidths::pane_width_limit(
            window_width,
            project_column.lint_column,
        ));
    // The list never cuts a name on screen to widen a middle pane.
    let floor = project_column
        .shown_width
        .max(project_column.minimum_width)
        .min(natural);
    let lang_min = lang::minimum_width();
    let targets_min = targets::minimum_width();
    let minimum_middle = targets_min
        .saturating_add(cpu_column_width())
        .saturating_add(lang_min);
    let preferred_middle = middle
        .targets_width
        .max(targets_min)
        .saturating_add(cpu_column_width())
        .saturating_add(middle.lang_width.max(lang_min));
    if preferred_middle > minimum_middle && window_width >= floor.saturating_add(minimum_middle) {
        natural
            .min(window_width.saturating_sub(preferred_middle))
            .max(floor)
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
    middle: MiddlePaneNeeds,
) -> (u16, u16) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(project_column_width(area.width, project_column, middle)),
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
    use super::MiddlePaneNeeds;
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
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            20,
            MiddlePaneNeeds::minimum(super::targets::minimum_width()),
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
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            100,
            MiddlePaneNeeds::minimum(super::targets::minimum_width()),
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
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            top_inner,
            MiddlePaneNeeds::minimum(super::targets::minimum_width()),
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
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            100,
            MiddlePaneNeeds::minimum(super::targets::minimum_width()),
        );
        let short_resolved_pane_layout = resolve_layout(
            Rect::new(0, 0, 120, 40),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            4,
            MiddlePaneNeeds::minimum(super::targets::minimum_width()),
        );

        assert!(short_resolved_pane_layout.area(PaneId::Cpu).height > cpu_floor);
        assert!(
            short_resolved_pane_layout.area(PaneId::Targets).height
                > tall_resolved_pane_layout.area(PaneId::Targets).height
        );
    }

    fn column(width: u16) -> ProjectColumnLayout {
        ProjectColumnLayout {
            preferred_width: width,
            minimum_width:   width,
            shown_width:     width,
            lint_column:     LintColumnStatus::Hidden,
        }
    }

    #[test]
    fn middle_panes_take_only_cells_no_shown_name_needs() {
        let shown = ProjectColumnLayout {
            preferred_width: 60,
            minimum_width:   40,
            shown_width:     56,
            lint_column:     LintColumnStatus::Hidden,
        };
        let layout = resolve_layout(
            Rect::new(0, 0, 160, 44),
            shown,
            12,
            BottomRow::Diagnostics,
            14,
            MiddlePaneNeeds::minimum(70),
        );

        assert_eq!(layout.area(PaneId::ProjectList).width, 56);
    }

    #[test]
    fn languages_take_their_whole_width_before_targets_grow() {
        let area = Rect::new(0, 0, 160, 44);
        for (projects_width, targets_need, expected_lang, expected_targets) in [
            (
                40,
                super::targets::minimum_width(),
                super::lang::minimum_width(),
                52,
            ),
            (60, 35, super::lang::minimum_width(), 32),
        ] {
            let layout = resolve_layout(
                area,
                column(projects_width),
                12,
                BottomRow::Diagnostics,
                14,
                MiddlePaneNeeds::minimum(targets_need),
            );
            let lang = layout.area(PaneId::Lang);
            let targets = layout.area(PaneId::Targets);

            assert_eq!(lang.width, expected_lang);
            assert_eq!(targets.width, expected_targets);
            assert!(targets.width >= super::targets::minimum_width());
            assert_eq!(layout.area(PaneId::ProjectList).width, projects_width);
            assert_eq!(
                targets.x.saturating_add(targets.width),
                area.width,
                "Targets takes the cells Languages does not need"
            );
        }
    }

    #[test]
    fn detail_row_takes_rows_no_project_needs_in_targets() {
        let width = 46_u16.saturating_add(super::targets::minimum_width());
        let area = Rect::new(0, 0, width, 45);
        let resolve = |targets_rows| {
            resolve_layout(
                area,
                column(46),
                32,
                BottomRow::Diagnostics,
                20,
                MiddlePaneNeeds::minimum(super::targets::minimum_width())
                    .with_rows(0, targets_rows),
            )
        };
        let short = resolve(5);
        let tall = resolve(u16::MAX);

        assert!(short.area(PaneId::Cpu).is_empty());
        assert_eq!(
            short.area(PaneId::Git).height,
            20 + PANE_BORDER_HEIGHT,
            "the detail row takes the free rows"
        );
        assert!(short.area(PaneId::Targets).height >= 5 + PANE_BORDER_HEIGHT);
        assert!(tall.area(PaneId::Git).height < short.area(PaneId::Git).height);
        assert_eq!(
            short.area(PaneId::CiRuns),
            tall.area(PaneId::CiRuns),
            "the bottom row keeps its rows"
        );
    }

    #[test]
    fn shown_cpu_keeps_the_row_line() {
        let resolve = |targets_rows| {
            resolve_layout(
                Rect::new(0, 0, 160, 45),
                column(46),
                32,
                BottomRow::Diagnostics,
                20,
                MiddlePaneNeeds::minimum(super::targets::minimum_width())
                    .with_rows(0, targets_rows),
            )
        };

        assert_eq!(
            resolve(1).area(PaneId::Git),
            resolve(u16::MAX).area(PaneId::Git)
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
