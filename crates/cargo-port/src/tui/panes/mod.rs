use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;

mod ci;
mod cpu;
mod git;
mod lang;
mod lints;
mod package;

#[cfg(test)]
pub(super) use ci::CI_COMPACT_DURATION_WIDTH;
#[cfg(test)]
pub(super) use ci::ci_table_shows_durations;
#[cfg(test)]
pub(super) use ci::ci_total_width;
pub(super) use ci::render_ci_panel;
pub(super) use cpu::CPU_PANE_WIDTH;
pub(super) use cpu::cpu_required_pane_height;
pub(super) use cpu::render_cpu_panel;
#[cfg(test)]
pub(super) use git::git_label_width;
pub(super) use git::render_git_panel;
pub(super) use lang::render_lang_panel_standalone;
pub(super) use lints::render_lints_panel;
pub(super) use package::RenderStyles;
#[cfg(test)]
pub(super) use package::description_lines;
#[cfg(test)]
pub(super) use package::detail_column_scroll_offset;
#[cfg(test)]
pub(super) use package::package_label_width;
pub(super) use package::render_empty_targets_panel;
pub(super) use package::render_package_panel;
pub(super) use package::render_targets_panel;
#[cfg(test)]
pub(super) use package::stats_column_width;

use super::constants::ACTIVE_BORDER_COLOR;
use super::constants::INACTIVE_BORDER_COLOR;
use super::constants::INACTIVE_TITLE_COLOR;
use super::constants::TITLE_COLOR;
use super::cpu::CpuSnapshot;
use super::detail::CiData;
use super::detail::GitData;
use super::detail::LintsData;
use super::detail::PackageData;
use super::detail::TargetsData;
use super::types::Pane;
use super::types::PaneId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PaneTitleGroup<'a> {
    pub label:  Cow<'a, str>,
    pub len:    usize,
    pub cursor: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PaneTitleCount<'a> {
    None,
    Single {
        len:    usize,
        cursor: Option<usize>,
    },
    Grouped(Vec<PaneTitleGroup<'a>>),
}

impl PaneTitleCount<'_> {
    fn count_text(len: usize, cursor: Option<usize>) -> String {
        if let Some(pos) = cursor
            && pos < len
        {
            crate::tui::types::scroll_indicator(pos, len)
        } else {
            len.to_string()
        }
    }

    pub(super) fn body(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Single { len, cursor } => format!("({})", Self::count_text(*len, *cursor)),
            Self::Grouped(groups) => groups
                .iter()
                .map(|group| {
                    format!(
                        "{} ({})",
                        group.label,
                        Self::count_text(group.len, group.cursor)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

pub(super) fn pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
    let body = count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title} {body} ")
    }
}

pub(super) fn prefixed_pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
    let body = count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title}: {body} ")
    }
}

#[derive(Clone, Copy)]
pub(super) struct PaneChrome {
    pub active_border:   Style,
    pub inactive_border: Style,
    pub active_title:    Style,
    pub inactive_title:  Style,
}

impl PaneChrome {
    pub(super) fn block(self, title: String, focused: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(if focused {
                self.active_title
            } else {
                self.inactive_title
            })
            .border_style(if focused {
                self.active_border
            } else {
                self.inactive_border
            })
    }

    pub(super) const fn with_inactive_border(self, inactive_border: Style) -> Self {
        Self {
            inactive_border,
            ..self
        }
    }
}

pub(super) fn default_pane_chrome() -> PaneChrome {
    let title_style = Style::default().add_modifier(Modifier::BOLD);
    PaneChrome {
        active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
        inactive_border: Style::default(),
        active_title:    title_style.fg(TITLE_COLOR),
        inactive_title:  title_style.fg(INACTIVE_TITLE_COLOR),
    }
}

pub(super) fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PanePlacement {
    pub pane:     PaneId,
    pub row:      usize,
    pub col:      usize,
    pub row_span: usize,
    pub col_span: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PaneGridLayout {
    pub placements: Vec<PanePlacement>,
}

impl PaneGridLayout {
    pub(super) fn tab_order(self) -> Vec<PaneId> {
        let mut placements = self.placements;
        placements.sort_by_key(|placement| (placement.row, placement.col));
        placements
            .into_iter()
            .map(|placement| placement.pane)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneBehavior {
    ProjectList,
    DetailFields,
    DetailTargets,
    Cpu,
    Lints,
    CiRuns,
    Output,
    Toasts,
    Overlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneAxisSize {
    Fixed(u16),
    Fill(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PaneSizeSpec {
    pub width:  PaneAxisSize,
    pub height: PaneAxisSize,
}

impl PaneSizeSpec {
    pub(super) const fn fill() -> Self {
        Self {
            width:  PaneAxisSize::Fill(1),
            height: PaneAxisSize::Fill(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneRule {
    Horizontal {
        area:        Rect,
        connector_x: Option<u16>,
    },
    Vertical {
        area: Rect,
    },
    Symbol {
        area:  Rect,
        glyph: char,
    },
}

pub(super) fn constraints_for_sizes(sizes: &[PaneAxisSize]) -> Vec<Constraint> {
    sizes
        .iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(length) => Constraint::Length(*length),
            PaneAxisSize::Fill(weight) => Constraint::Fill(*weight),
        })
        .collect()
}

pub(super) fn render_rules(frame: &mut Frame, rules: &[PaneRule], style: Style) {
    for rule in rules {
        match *rule {
            PaneRule::Horizontal { area, connector_x } => {
                render_horizontal_rule(frame, area, style, connector_x);
            },
            PaneRule::Vertical { area } => render_vertical_rule(frame, area, style),
            PaneRule::Symbol { area, glyph } => render_symbol_rule(frame, area, style, glyph),
        }
    }
}

fn render_horizontal_rule(frame: &mut Frame, area: Rect, style: Style, connector_x: Option<u16>) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = (0..area.width)
        .map(|offset| {
            let x = area.x.saturating_add(offset);
            if offset == 0 {
                '├'
            } else if offset == area.width.saturating_sub(1) {
                '┤'
            } else if connector_x == Some(x) {
                '┬'
            } else {
                '─'
            }
        })
        .collect::<String>();
    frame.render_widget(Paragraph::new(Line::from(Span::styled(line, style))), area);
}

fn render_vertical_rule(frame: &mut Frame, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = (0..area.height)
        .map(|_| Line::from(Span::styled("│", style)))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_symbol_rule(frame: &mut Frame, area: Rect, style: Style, glyph: char) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(glyph.to_string(), style))),
        area,
    );
}

pub(super) struct PaneManager {
    panes:            Vec<Pane>,
    pub package_data: Option<PackageData>,
    pub git_data:     Option<GitData>,
    pub cpu_data:     Option<CpuSnapshot>,
    pub targets_data: Option<TargetsData>,
    pub ci_data:      Option<CiData>,
    pub lints_data:   Option<LintsData>,
}

impl PaneManager {
    pub fn pane(&self, id: PaneId) -> &Pane { &self.panes[id.index()] }

    pub fn pane_mut(&mut self, id: PaneId) -> &mut Pane { &mut self.panes[id.index()] }

    pub fn new() -> Self {
        Self {
            panes:        vec![Pane::new(); PaneId::pane_count()],
            package_data: None,
            git_data:     None,
            cpu_data:     None,
            targets_data: None,
            ci_data:      None,
            lints_data:   None,
        }
    }

    pub fn clear_hover(&mut self) {
        for pane in &mut self.panes {
            pane.set_hovered(None);
        }
    }

    pub(super) const fn behavior(id: PaneId) -> PaneBehavior {
        match id {
            PaneId::ProjectList => PaneBehavior::ProjectList,
            PaneId::Package | PaneId::Lang | PaneId::Git => PaneBehavior::DetailFields,
            PaneId::Cpu => PaneBehavior::Cpu,
            PaneId::Targets => PaneBehavior::DetailTargets,
            PaneId::Lints => PaneBehavior::Lints,
            PaneId::CiRuns => PaneBehavior::CiRuns,
            PaneId::Output => PaneBehavior::Output,
            PaneId::Toasts => PaneBehavior::Toasts,
            PaneId::Settings | PaneId::Finder | PaneId::Keymap => PaneBehavior::Overlay,
        }
    }

    pub(super) const fn has_row_hitboxes(id: PaneId) -> bool {
        matches!(
            Self::behavior(id),
            PaneBehavior::DetailFields | PaneBehavior::DetailTargets
        )
    }

    pub(super) const fn size_spec(id: PaneId) -> PaneSizeSpec {
        match id {
            PaneId::Cpu => PaneSizeSpec {
                width:  PaneAxisSize::Fixed(CPU_PANE_WIDTH),
                height: PaneAxisSize::Fill(1),
            },
            _ => PaneSizeSpec::fill(),
        }
    }

    pub(super) fn derived_layout(output_visible: bool) -> PaneGridLayout {
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

    pub(super) fn tab_order(output_visible: bool) -> Vec<PaneId> {
        Self::derived_layout(output_visible).tab_order()
    }

    pub fn set_detail_data(
        &mut self,
        package_data: PackageData,
        git_data: GitData,
        targets_data: TargetsData,
        ci_data: CiData,
        lints_data: LintsData,
    ) {
        self.package_data = Some(package_data);
        self.git_data = Some(git_data);
        self.targets_data = Some(targets_data);
        self.ci_data = Some(ci_data);
        self.lints_data = Some(lints_data);
    }

    pub fn clear_detail_data(&mut self) {
        self.package_data = None;
        self.git_data = None;
        self.targets_data = None;
        self.ci_data = None;
        self.lints_data = None;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PaneGridLayout;
    use super::PaneId;
    use super::PaneManager;
    use super::PanePlacement;
    use super::PaneTitleCount;
    use super::PaneTitleGroup;
    use super::pane_title;
    use super::prefixed_pane_title;

    #[test]
    fn tiled_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&PaneManager::derived_layout(false));
    }

    #[test]
    fn output_layout_has_no_overlapping_cells() {
        assert_layout_has_no_overlaps(&PaneManager::derived_layout(true));
    }

    #[test]
    fn derived_output_layout_keeps_cpu_between_lang_and_targets() {
        let order = PaneManager::derived_layout(true).tab_order();
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
                super::PaneId::ProjectList,
                super::PaneId::Package,
                super::PaneId::Git,
                super::PaneId::Targets,
            ]
        );
    }

    #[test]
    fn single_title_count_formats_cursor_position() {
        assert_eq!(
            pane_title(
                "Languages",
                &PaneTitleCount::Single {
                    len:    4,
                    cursor: Some(1),
                }
            ),
            " Languages (2 of 4) "
        );
    }

    #[test]
    fn single_title_count_ignores_out_of_range_cursor() {
        assert_eq!(
            pane_title(
                "Lint Runs",
                &PaneTitleCount::Single {
                    len:    3,
                    cursor: Some(9),
                }
            ),
            " Lint Runs (3) "
        );
    }

    #[test]
    fn grouped_title_count_formats_each_group() {
        assert_eq!(
            prefixed_pane_title(
                "Targets",
                &PaneTitleCount::Grouped(vec![
                    PaneTitleGroup {
                        label:  "Binary".into(),
                        len:    1,
                        cursor: Some(0),
                    },
                    PaneTitleGroup {
                        label:  "Examples".into(),
                        len:    3,
                        cursor: None,
                    },
                ])
            ),
            " Targets: Binary (1 of 1), Examples (3) "
        );
    }

    fn assert_layout_has_no_overlaps(layout: &PaneGridLayout) {
        let mut occupied = HashSet::new();
        for placement in &layout.placements {
            for row in placement.row..placement.row + placement.row_span {
                for col in placement.col..placement.col + placement.col_span {
                    assert!(
                        occupied.insert((row, col)),
                        "pane {:?} overlaps cell ({row}, {col})",
                        placement.pane
                    );
                }
            }
        }
    }
}
