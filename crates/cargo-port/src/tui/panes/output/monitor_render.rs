//! Drawing the build-monitor half of the Output pane.
//!
//! Every decision here reads the [`MonitorPresentation`] the pane was given —
//! how many columns exist, which of them fit, what the empty states say — so
//! the drawing cannot disagree with the layout and hit testing that read the
//! same value.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use tui_pane::label_color;

use super::constants::COLUMN_DIVIDER_WIDTH;
use super::constants::COLUMN_HEADER_HEIGHT;
use super::constants::COLUMN_STRIP_MINIMUM_HEIGHT;
use super::constants::MINIMUM_READABLE_COLUMN_WIDTH;
use super::constants::MONITOR_INDICATOR_HEIGHT;
use super::constants::OWNED_OUTPUT_SEPARATOR_HEIGHT;
use super::constants::OWNED_PIN_CAPTION_HEIGHT;
use super::constants::UNATTRIBUTED_SECTION_AREA_DIVISOR;
use super::constants::UNATTRIBUTED_SECTION_CAPTION_HEIGHT;
use super::constants::UNATTRIBUTED_SECTION_MINIMUM_HEIGHT;
use super::hit_map::HitRegion;
use super::hit_map::MonitorHitMap;
use super::hit_map::OutputMonitorHit;
use super::hit_map::row_rect;
use super::presentation::MonitorColumn;
use super::presentation::MonitorColumns;
use super::presentation::MonitorEmptyState;
use super::presentation::MonitorEmptyStateIndexNote;
use super::presentation::MonitorPresentation;
use super::presentation::OutputBuildSetTerminationAggregateProjection;
use super::presentation::OwnedOutputPresentation;
use super::render::fill_row;
use super::render::parse_output_line;
use super::selection::OutputCursor;
use super::selection::OutputCursorTarget;
use super::selection::OutputSelectionRange;
use super::selection::VisualSelectionPermission;
use crate::build_monitor::AdditionalBuildExclusion;
use crate::build_monitor::BuildTerminationAggregateCompletion;
use crate::build_monitor::BuildTerminationSessionCompletion;
use crate::build_monitor::BuildTerminationTerminalRecord;
use crate::build_monitor::CompileActivity;
use crate::build_monitor::CompiledCrateIdentity;
use crate::build_monitor::CompilerAttribution;
use crate::build_monitor::CompilerKind;
use crate::build_monitor::MonitorSessionOwnership;
use crate::build_monitor::MonitorStaleness;
use crate::build_monitor::UnattributedCompileActivity;
use crate::tui::OwnedRunId;
use crate::tui::state::OwnedRunCompletionMarker;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

/// Cargo Port's own captured output as the monitor draws it: the presentation
/// value, the buffer the selection reads, and the rows that selection covers.
///
/// The buffer is passed separately from [`OwnedOutputPresentation::lines`]
/// because a frozen selection reads the buffer it was pinned against rather
/// than the live tail.
#[derive(Clone, Copy)]
pub(super) struct OwnedBody<'a> {
    owned:          OwnedOutputPresentation<'a>,
    source:         &'a [String],
    selected_range: OutputSelectionRange,
    scroll_offset:  usize,
}

impl<'a> OwnedBody<'a> {
    /// `scroll_offset` is [`tui_pane::Viewport::scroll_offset`], the same first visible
    /// captured row the monitor-off view scrolls to, so turning the monitor on
    /// does not turn the owned output into a head-only view.
    pub(super) const fn new(
        owned: OwnedOutputPresentation<'a>,
        source: &'a [String],
        selected_range: OutputSelectionRange,
        scroll_offset: usize,
    ) -> Self {
        Self {
            owned,
            source,
            selected_range,
            scroll_offset,
        }
    }
}

/// Draw the monitor together with Cargo Port's own body, if one is on screen.
///
/// The owned body is joined to the columns by the retaining run's producer id,
/// never by the current lifecycle identity. When that producer is one of the
/// scope's sessions the body extends its column; when it is not, the body is
/// pinned as its own column beside them and stays out of the scope's columns.
pub(super) fn render_monitor_half(
    frame: &mut Frame,
    area: Rect,
    monitor_presentation: MonitorPresentation<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    selected_column_scroll: SelectedColumnScroll,
    hit_map: &mut MonitorHitMap,
) {
    let Some(owned_body) = owned_body else {
        render_monitor(
            frame,
            area,
            monitor_presentation,
            None,
            cursor,
            selected_column_scroll,
            hit_map,
        );
        return;
    };
    if producer_is_in_scope(monitor_presentation, owned_body.owned.producer()) {
        render_monitor(
            frame,
            area,
            monitor_presentation,
            Some(owned_body),
            cursor,
            selected_column_scroll,
            hit_map,
        );
        return;
    }
    let pin_width = pinned_column_width(area.width);
    let monitor_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(pin_width),
        area.height,
    );
    render_monitor(
        frame,
        monitor_area,
        monitor_presentation,
        None,
        cursor,
        selected_column_scroll,
        hit_map,
    );
    let pin_area = Rect::new(
        area.x.saturating_add(monitor_area.width),
        area.y,
        pin_width,
        area.height,
    );
    render_pinned_owned(frame, pin_area, owned_body, cursor, hit_map);
}

/// Whether the run that produced the retained output is one of the scope's own
/// sessions.
fn producer_is_in_scope(
    monitor_presentation: MonitorPresentation<'_>,
    producer: OwnedRunId,
) -> bool {
    producing_column_indexed(monitor_presentation, producer).is_some()
}

/// The scope's own column for the run that produced the retained output, when
/// the scope still covers it.
/// A column together with its position in the monitor's ordered session list.
///
/// The index is what the cursor, the hit map, and the scroll offset all agree
/// on, so it travels with the column rather than beside it.
#[derive(Clone, Copy)]
struct IndexedColumn<'a> {
    column: MonitorColumn<'a>,
    index:  usize,
}

/// The selected column's vertical scroll, as the draw path consumes it.
///
/// Carries the column index the offset was measured against, so every draw site
/// applies it to exactly one column: a bare `usize` would scroll whichever
/// column a caller happened to hand it to.
#[derive(Clone, Copy)]
pub(super) struct SelectedColumnScroll {
    column_index: usize,
    offset:       usize,
}

impl SelectedColumnScroll {
    /// The scroll `offset` belonging to the column at `column_index`.
    pub(super) const fn new(column_index: usize, offset: usize) -> Self {
        Self {
            column_index,
            offset,
        }
    }

    /// How many of that column's activity rows are scrolled off the top. Every
    /// other column draws from its first activity row.
    const fn for_column(self, column_index: usize) -> usize {
        if self.column_index == column_index {
            self.offset
        } else {
            0
        }
    }
}

/// How many activity rows the selected column has, and how many of them fit.
///
/// Feeds [`super::pane::OutputPane::sync_column_scroll`], which reconciles the
/// offset against the cursor before the frame draws with it.
#[derive(Clone, Copy)]
pub(super) struct ColumnActivityExtent {
    /// Rows the column can draw in the strip.
    pub(super) visible_rows: usize,
    /// Rows the column has.
    pub(super) row_count:    usize,
}

impl ColumnActivityExtent {
    /// Nothing to scroll: the monitor is off, or the cursor is not in a column.
    pub(super) const fn none() -> Self {
        Self {
            visible_rows: 0,
            row_count:    0,
        }
    }
}

/// The activity-row extent of the column the cursor is in.
pub(super) fn selected_column_activity_extent(
    area: Rect,
    monitor_presentation: MonitorPresentation<'_>,
    cursor: &OutputCursor,
) -> ColumnActivityExtent {
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return ColumnActivityExtent::none();
    };
    let Some(column) = monitor_columns.columns().nth(cursor.column_index()) else {
        return ColumnActivityExtent::none();
    };
    ColumnActivityExtent {
        visible_rows: column_activity_capacity(area, monitor_columns),
        row_count:    column.session_row().compile_activities().len(),
    }
}

/// How many activity rows a column can draw inside `area`.
///
/// Shares its terms with [`owned_captured_output_height`]: both start from the
/// strip height a column is drawn at and subtract the header. Any change to
/// what a column draws above its activity rows belongs in both.
fn column_activity_capacity(area: Rect, monitor_columns: MonitorColumns<'_>) -> usize {
    let body_height = area.height.saturating_sub(MONITOR_INDICATOR_HEIGHT);
    let strip_height = body_height
        .saturating_sub(UnattributedSectionLayout::within(body_height, monitor_columns).height);
    usize::from(strip_height).saturating_sub(COLUMN_HEADER_HEIGHT)
}

fn producing_column_indexed(
    monitor_presentation: MonitorPresentation<'_>,
    producer: OwnedRunId,
) -> Option<(usize, MonitorColumn<'_>)> {
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return None;
    };
    monitor_columns
        .columns()
        .enumerate()
        .find(|(_, column)| column_produced(*column, producer))
}

/// Whether this column's session is the run that produced the retained output.
fn column_produced(column: MonitorColumn<'_>, producer: OwnedRunId) -> bool {
    match column.session_ownership() {
        MonitorSessionOwnership::Owned(owned_run_id) => owned_run_id == producer,
        MonitorSessionOwnership::External => false,
    }
}

/// How many captured-output rows the owned body is drawn at inside `area`.
///
/// The pane's viewport scrolls that body, so its extent has to be the height the
/// body is actually drawn at: sizing it from the whole pane leaves the last
/// captured rows unreachable while the monitor is on, and lets the view scroll
/// past the end.
pub(super) fn owned_captured_output_height(
    area: Rect,
    monitor_presentation: MonitorPresentation<'_>,
    producer: OwnedRunId,
    selected_column_scroll: SelectedColumnScroll,
) -> usize {
    let Some((producing_column_index, producing_column)) =
        producing_column_indexed(monitor_presentation, producer)
    else {
        return usize::from(area.height)
            .saturating_sub(OWNED_PIN_CAPTION_HEIGHT + OWNED_OUTPUT_SEPARATOR_HEIGHT);
    };
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return 0;
    };
    let body_height = area.height.saturating_sub(MONITOR_INDICATOR_HEIGHT);
    let strip_height = body_height
        .saturating_sub(UnattributedSectionLayout::within(body_height, monitor_columns).height);
    usize::from(strip_height)
        .saturating_sub(COLUMN_HEADER_HEIGHT)
        .saturating_sub(
            producing_column
                .session_row()
                .compile_activities()
                .len()
                .saturating_sub(selected_column_scroll.for_column(producing_column_index)),
        )
        .saturating_sub(OWNED_OUTPUT_SEPARATOR_HEIGHT)
}

/// How wide the pinned owned column is. It keeps a readable width wherever the
/// pane can afford one, and otherwise splits the pane with the monitor rather
/// than disappearing: the pin has to outlive the scope that stopped covering it.
const fn pinned_column_width(available: u16) -> u16 {
    if available >= MINIMUM_READABLE_COLUMN_WIDTH.saturating_mul(2) {
        MINIMUM_READABLE_COLUMN_WIDTH
    } else {
        available / 2
    }
}

/// The owned run's output pinned outside the monitored scope: what produced it,
/// how it ended, and the captured lines under a separator.
fn render_pinned_owned(
    frame: &mut Frame,
    area: Rect,
    owned_body: OwnedBody<'_>,
    cursor: &OutputCursor,
    hit_map: &mut MonitorHitMap,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Block::new()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(label_color())),
        area,
    );
    let text_area = Rect::new(
        area.x.saturating_add(COLUMN_DIVIDER_WIDTH),
        area.y,
        area.width.saturating_sub(COLUMN_DIVIDER_WIDTH),
        area.height,
    );
    let mut lines = vec![Line::from(Span::styled(
        format!(" {}", owned_caption(owned_body.owned)),
        Style::default().fg(label_color()),
    ))];
    lines.extend(owned_output_lines(
        owned_body,
        usize::from(text_area.width),
        cursor,
        hit_map,
        text_area,
        lines.len(),
    ));
    frame.render_widget(Paragraph::new(lines), text_area);
}

/// The pin's caption: the title the output was captured under, and how the run
/// that produced it ended.
fn owned_caption(owned: OwnedOutputPresentation<'_>) -> String {
    let title = match owned.title() {
        OwnedRunOutputTitleRef::Named(title) => title.to_string(),
        OwnedRunOutputTitleRef::Unavailable => "Cargo Port run".to_string(),
    };
    match owned.running_label() {
        OwnedRunRunningLabelRef::Running(name) => format!("{name} — running"),
        OwnedRunRunningLabelRef::NotRunning => match owned.completion_marker() {
            OwnedRunCompletionMarker::NotCompleted => title,
            OwnedRunCompletionMarker::Done => format!("{title} — done"),
            OwnedRunCompletionMarker::Killed => format!("{title} — killed"),
            OwnedRunCompletionMarker::Failed => format!("{title} — failed"),
        },
    }
}

/// The non-selectable separator and the captured output beneath it.
///
/// The separator is drawn as a caption rather than a row so no cursor motion
/// can land on it.
fn owned_output_lines(
    owned_body: OwnedBody<'_>,
    width: usize,
    cursor: &OutputCursor,
    hit_map: &mut MonitorHitMap,
    area: Rect,
    start_line: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        " ── Output ──",
        Style::default().fg(label_color()),
    ))];
    let captured_selected = matches!(
        cursor.target().visual_selection_permission(),
        VisualSelectionPermission::CapturedOutput
    );
    // Only the rows the column has height for are emitted, starting at the
    // viewport's first visible row: ratatui truncates a longer paragraph
    // silently, which would leave the tail of a running build unreachable.
    let drawable_rows = usize::from(area.height).saturating_sub(start_line + lines.len());
    for (row, raw) in owned_body
        .source
        .iter()
        .enumerate()
        .skip(owned_body.scroll_offset)
        .take(drawable_rows)
    {
        hit_map.record(
            row_rect(area, start_line + lines.len()),
            OutputMonitorHit::CapturedOutput {
                producer: owned_body.owned.producer(),
                row,
            },
        );
        let parsed = parse_output_line(raw);
        lines.push(
            if captured_selected && owned_body.selected_range.contains(row) {
                fill_row(parsed, width)
            } else {
                parsed
            },
        );
    }
    lines
}

/// Draw the monitor half of the pane.
///
/// The indicator row is drawn for every state, including the empty ones: an
/// enabled monitor with nothing to show still has to read as enabled.
fn render_monitor(
    frame: &mut Frame,
    area: Rect,
    monitor_presentation: MonitorPresentation<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    selected_column_scroll: SelectedColumnScroll,
    hit_map: &mut MonitorHitMap,
) {
    if area.height < MONITOR_INDICATOR_HEIGHT {
        return;
    }
    let (monitor_staleness, terminal_record_count) = match monitor_presentation {
        MonitorPresentation::Empty(monitor_empty_presentation) => (
            MonitorStaleness::Live,
            monitor_empty_presentation.terminal_records().count(),
        ),
        MonitorPresentation::Columns(monitor_columns) => (
            monitor_columns.monitor_staleness(),
            monitor_columns.terminal_records().count(),
        ),
    };
    let indicator = Rect::new(area.x, area.y, area.width, MONITOR_INDICATOR_HEIGHT);
    frame.render_widget(
        Paragraph::new(indicator_line(monitor_staleness, terminal_record_count)),
        indicator,
    );

    let body = Rect::new(
        area.x,
        area.y.saturating_add(MONITOR_INDICATOR_HEIGHT),
        area.width,
        area.height.saturating_sub(MONITOR_INDICATOR_HEIGHT),
    );
    if body.height == 0 {
        return;
    }
    match monitor_presentation {
        MonitorPresentation::Empty(monitor_empty_presentation) => {
            hit_map.record(HitRegion::Drawn(body), OutputMonitorHit::EmptyMonitor);
            frame.render_widget(
                Paragraph::new(empty_state_lines(monitor_empty_presentation)),
                body,
            );
        },
        MonitorPresentation::Columns(monitor_columns) => {
            render_columns_with_terminal_records(
                frame,
                body,
                monitor_columns,
                owned_body,
                cursor,
                selected_column_scroll,
                hit_map,
            );
        },
    }
}

/// Draw live columns above the row-independent terminal records they do not
/// replace. The registry stays the sole result owner; this only reserves the
/// visible rows needed to project its existing records beside live work.
fn render_columns_with_terminal_records(
    frame: &mut Frame,
    area: Rect,
    monitor_columns: MonitorColumns<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    selected_column_scroll: SelectedColumnScroll,
    hit_map: &mut MonitorHitMap,
) {
    let terminal_records: Vec<_> = monitor_columns.terminal_records().collect();
    let terminal_record_lines = termination_result_lines(&terminal_records);
    let terminal_record_height = u16::try_from(terminal_record_lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    let live_columns_area = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(terminal_record_height),
    );
    if live_columns_area.height > 0 {
        render_columns(
            frame,
            live_columns_area,
            monitor_columns,
            owned_body,
            cursor,
            selected_column_scroll,
            hit_map,
        );
    }
    if terminal_record_height > 0 {
        let terminal_record_area = Rect::new(
            area.x,
            area.y.saturating_add(live_columns_area.height),
            area.width,
            terminal_record_height,
        );
        frame.render_widget(Paragraph::new(terminal_record_lines), terminal_record_area);
    }
}

/// The always-visible enabled indicator, carrying the staleness marker.
fn indicator_line(
    monitor_staleness: MonitorStaleness,
    terminal_record_count: usize,
) -> Line<'static> {
    let text = match monitor_staleness {
        MonitorStaleness::Live => " Build monitor on",
        MonitorStaleness::Stale => " Build monitor on — stale",
    };
    let terminal_note = match terminal_record_count {
        0 => String::new(),
        1 => " — 1 termination result".to_string(),
        _ => format!(" — {terminal_record_count} termination results"),
    };
    Line::from(Span::styled(
        format!("{text}{terminal_note}"),
        Style::default().fg(label_color()),
    ))
}

/// Why the enabled monitor has no columns, followed by the index that answer
/// was resolved against when the scope resolution is what made it empty.
fn empty_state_line(monitor_empty_state: MonitorEmptyState) -> Line<'static> {
    let style = Style::default().fg(label_color());
    let monitor_empty_state_message = monitor_empty_state.message();
    let mut spans = vec![
        Span::styled(" ", style),
        Span::styled(monitor_empty_state_message.headline(), style),
    ];
    match monitor_empty_state_message.index_note() {
        MonitorEmptyStateIndexNote::Absent => {},
        MonitorEmptyStateIndexNote::Index(index_label) => {
            spans.push(Span::styled(" (", style));
            spans.push(Span::styled(index_label, style));
            spans.push(Span::styled(")", style));
        },
    }
    Line::from(spans)
}

/// The empty monitor explanation followed by terminal records that outlived
/// their replaceable live session rows.
fn empty_state_lines(
    monitor_empty_presentation: super::presentation::MonitorEmptyPresentation<'_>,
) -> Vec<Line<'static>> {
    let terminal_records: Vec<_> = monitor_empty_presentation.terminal_records().collect();
    std::iter::once(empty_state_line(
        monitor_empty_presentation.monitor_empty_state(),
    ))
    .chain(termination_result_lines(&terminal_records))
    .collect()
}

fn termination_result_lines(
    terminal_records: &[&BuildTerminationTerminalRecord],
) -> Vec<Line<'static>> {
    let output_build_set_result_groups =
        super::presentation::output_build_set_termination_result_groups(
            terminal_records.iter().copied(),
        );
    output_build_set_result_groups
        .into_iter()
        .flat_map(|output_build_set_result_group| {
            let aggregate = output_build_set_result_group.aggregate();
            let terminal_records = output_build_set_result_group.into_terminal_records();
            std::iter::once(output_build_set_aggregate_line(aggregate))
                .chain(terminal_records.into_iter().map(terminal_record_line))
        })
        .chain(
            terminal_records
                .iter()
                .copied()
                .filter(|terminal_record| {
                    terminal_record.target_set()
                        != crate::build_monitor::BuildTerminationTransactionTargetSet::OutputBuildSet
                })
                .map(terminal_record_line),
        )
        .collect()
}

/// One transaction-level output-build-set outcome projected from the registry's
/// per-root terminal records.
fn output_build_set_aggregate_line(
    output_build_set_termination_aggregate_projection: OutputBuildSetTerminationAggregateProjection,
) -> Line<'static> {
    let result = match output_build_set_termination_aggregate_projection.aggregate_completion() {
        BuildTerminationAggregateCompletion::AllTargetsGone => "all roots gone",
        BuildTerminationAggregateCompletion::RetryUnavailable => "termination incomplete",
        BuildTerminationAggregateCompletion::DeadlineExpired => "termination timed out",
    };
    let additional_build_exclusion =
        match output_build_set_termination_aggregate_projection.additional_build_exclusion() {
            AdditionalBuildExclusion::NoAdditionalBuilds => String::new(),
            AdditionalBuildExclusion::Excluded { count } => {
                format!(" · {count} newer build(s) excluded")
            },
        };
    Line::from(Span::styled(
        format!(
            " Output build set ({} roots) — {result}{additional_build_exclusion}",
            output_build_set_termination_aggregate_projection.confirmed_root_count(),
        ),
        Style::default().fg(label_color()),
    ))
}

/// One retained terminal record named by its frozen root PID.
fn terminal_record_line(
    build_termination_terminal_record: &BuildTerminationTerminalRecord,
) -> Line<'static> {
    let result = match build_termination_terminal_record.session_completion() {
        BuildTerminationSessionCompletion::AlreadyGone => "already gone",
        BuildTerminationSessionCompletion::GoneAfterSignaling => "gone after signal",
        BuildTerminationSessionCompletion::RetryUnavailable => "termination incomplete",
        BuildTerminationSessionCompletion::DeadlineExpired => "termination timed out",
    };
    Line::from(Span::styled(
        format!(
            " Build pid {} — {result}",
            build_termination_terminal_record
                .display_identity()
                .root_pid(),
        ),
        Style::default().fg(label_color()),
    ))
}

/// Split `area` into the column strip and the scope-level unattributed section,
/// then draw both.
fn render_columns(
    frame: &mut Frame,
    area: Rect,
    monitor_columns: MonitorColumns<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    selected_column_scroll: SelectedColumnScroll,
    hit_map: &mut MonitorHitMap,
) {
    let unattributed_section_layout =
        UnattributedSectionLayout::within(area.height, monitor_columns);
    let strip = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height
            .saturating_sub(unattributed_section_layout.height),
    );
    render_column_strip(
        frame,
        strip,
        monitor_columns,
        owned_body,
        cursor,
        selected_column_scroll,
        hit_map,
    );

    if unattributed_section_layout.height > 0 {
        let section = Rect::new(
            area.x,
            area.y.saturating_add(strip.height),
            area.width,
            unattributed_section_layout.height,
        );
        render_unattributed_section(
            frame,
            section,
            monitor_columns,
            unattributed_section_layout,
            cursor,
            hit_map,
        );
    }
}

/// How many rows the scope-level unattributed section takes, and how many of
/// its activities that height leaves off screen.
///
/// The hidden count is carried rather than discarded because this section is
/// the pane's only statement that some observed compiler could not be placed:
/// dropping rows without saying so reads as "everything is attributed".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UnattributedSectionLayout {
    height: u16,
    hidden: usize,
}

impl UnattributedSectionLayout {
    /// Fit the section into `available` rows.
    ///
    /// With no columns beside it the section may use the whole area — the share
    /// cap exists to keep columns on screen, and there are none. With columns,
    /// it takes its share but never less than its caption plus one row, so a
    /// short pane narrows the section instead of hiding it entirely, and never
    /// so much that the columns it sits under are left no rows at all.
    fn within(available: u16, monitor_columns: MonitorColumns<'_>) -> Self {
        let activity_count = monitor_columns.unattributed_activities().len();
        if activity_count == 0 {
            return Self {
                height: 0,
                hidden: 0,
            };
        }
        let budget = if monitor_columns.len() == 0 {
            available
        } else {
            (available / UNATTRIBUTED_SECTION_AREA_DIVISOR)
                .max(UNATTRIBUTED_SECTION_MINIMUM_HEIGHT)
                .min(available.saturating_sub(COLUMN_STRIP_MINIMUM_HEIGHT))
        };
        let wanted = u16::try_from(activity_count)
            .unwrap_or(u16::MAX)
            .saturating_add(UNATTRIBUTED_SECTION_CAPTION_HEIGHT);
        let height = wanted.min(budget);
        Self {
            height,
            hidden: activity_count.saturating_sub(usize::from(
                height.saturating_sub(UNATTRIBUTED_SECTION_CAPTION_HEIGHT),
            )),
        }
    }
}

/// Draw the visible window of columns, evenly splitting the strip.
fn render_column_strip(
    frame: &mut Frame,
    area: Rect,
    monitor_columns: MonitorColumns<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    selected_column_scroll: SelectedColumnScroll,
    hit_map: &mut MonitorHitMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let window = monitor_columns.window(area.width, cursor.column_index());
    let drawn: Vec<(usize, MonitorColumn<'_>)> = monitor_columns
        .columns()
        .enumerate()
        .filter(|(column_index, _)| window.contains(*column_index))
        .collect();
    let Ok(drawn_count) = u16::try_from(drawn.len()) else {
        return;
    };
    if drawn_count == 0 {
        return;
    }
    let column_width = area.width / drawn_count;
    for (offset, (column_index, column)) in drawn.into_iter().enumerate() {
        let Ok(offset) = u16::try_from(offset) else {
            break;
        };
        let column_area = Rect::new(
            area.x.saturating_add(offset.saturating_mul(column_width)),
            area.y,
            column_width,
            area.height,
        );
        // A single column takes the full width with no rule; every later column
        // draws the rule that separates it from the one on its left.
        let text_area = if offset == 0 {
            column_area
        } else {
            frame.render_widget(
                Block::new()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(label_color())),
                column_area,
            );
            Rect::new(
                column_area.x.saturating_add(COLUMN_DIVIDER_WIDTH),
                column_area.y,
                column_area.width.saturating_sub(COLUMN_DIVIDER_WIDTH),
                column_area.height,
            )
        };
        render_column(
            frame,
            text_area,
            IndexedColumn {
                column,
                index: column_index,
            },
            owned_body,
            cursor,
            selected_column_scroll.for_column(column_index),
            hit_map,
        );
    }
}

/// One root Cargo invocation: its header fields, then one selectable row per
/// compile activity attributed to it.
fn render_column(
    frame: &mut Frame,
    area: Rect,
    indexed_column: IndexedColumn<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    activity_scroll: usize,
    hit_map: &mut MonitorHitMap,
) {
    let IndexedColumn {
        column,
        index: column_index,
    } = indexed_column;
    let width = usize::from(area.width);
    let now = Instant::now();
    let mut lines =
        Vec::with_capacity(COLUMN_HEADER_HEIGHT + column.session_row().compile_activities().len());

    let header_selected = matches!(
        cursor.target(),
        OutputCursorTarget::Header(build_session_id) if build_session_id == column.build_session_id()
    );
    for (index, text) in column.header_fields(now).into_iter().enumerate() {
        let line = Line::from(Span::raw(format!(" {text}")));
        hit_map.record(
            row_rect(area, lines.len()),
            OutputMonitorHit::Header {
                build_session_id: column.build_session_id().clone(),
                column_index,
            },
        );
        lines.push(if header_selected && index == 0 {
            fill_row(line, width)
        } else {
            line
        });
    }

    // `skip` after `enumerate`, so `row_index` stays the model index the cursor
    // and the hit map agree on while the drawn position comes from `lines.len()`.
    for (row_index, compile_activity) in column
        .session_row()
        .compile_activities()
        .iter()
        .enumerate()
        .skip(activity_scroll)
    {
        let line = Line::from(Span::raw(format!(" {}", activity_label(compile_activity))));
        hit_map.record(
            row_rect(area, lines.len()),
            OutputMonitorHit::Activity {
                compile_activity_id: compile_activity.compile_activity_id().clone(),
                build_session_id: column.build_session_id().clone(),
                column_index,
                row_index,
            },
        );
        let selected = matches!(
            cursor.target(),
            OutputCursorTarget::Activity(compile_activity_id)
                if compile_activity_id == compile_activity.compile_activity_id()
        );
        lines.push(if selected {
            fill_row(line, width)
        } else {
            line
        });
    }

    // The owned run's own column carries its captured output below its activity
    // rows; every external column ends at its activities.
    if let Some(owned_body) =
        owned_body.filter(|owned_body| column_produced(column, owned_body.owned.producer()))
    {
        lines.extend(owned_output_lines(
            owned_body,
            width,
            cursor,
            hit_map,
            area,
            lines.len(),
        ));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The text of the activity row `compile_activity_id` names, as a copy
/// payload. The row is a live sample rather than a transcript, so what is
/// copied is exactly the line drawn — the same string the user is looking at.
pub(super) fn activity_row_text(
    monitor_presentation: MonitorPresentation<'_>,
    compile_activity_id: &crate::build_monitor::CompileActivityId,
) -> tui_pane::CopySelectionResult {
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return tui_pane::CopySelectionResult::Nothing;
    };
    monitor_columns
        .columns()
        .flat_map(|column| column.session_row().compile_activities())
        .find(|compile_activity| compile_activity.compile_activity_id() == compile_activity_id)
        .map_or(tui_pane::CopySelectionResult::Nothing, |compile_activity| {
            let row = [activity_label(compile_activity)];
            crate::tui::panes::copy_payload_for_output(&row, 0, 0)
        })
}

/// The text of the scope-level unattributed row `compile_activity_id` names.
pub(super) fn unattributed_row_text(
    monitor_presentation: MonitorPresentation<'_>,
    compile_activity_id: &crate::build_monitor::CompileActivityId,
) -> tui_pane::CopySelectionResult {
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return tui_pane::CopySelectionResult::Nothing;
    };
    monitor_columns
        .unattributed_activities()
        .iter()
        .find(|unattributed| unattributed.compile_activity_id() == compile_activity_id)
        .map_or(tui_pane::CopySelectionResult::Nothing, |unattributed| {
            let row = [unattributed_label(unattributed)];
            crate::tui::panes::copy_payload_for_output(&row, 0, 0)
        })
}

/// One activity row: which compiler is running and what it is compiling.
fn activity_label(compile_activity: &CompileActivity) -> String {
    format!(
        "{} {}",
        compiler_kind_label(compile_activity.compiler_kind()),
        crate_identity_label(compile_activity.compiled_crate_identity()),
    )
}

/// The compiler executable behind an activity row.
const fn compiler_kind_label(compiler_kind: CompilerKind) -> &'static str {
    match compiler_kind {
        CompilerKind::Rustc => "rustc",
        CompilerKind::ClippyDriver => "clippy",
        CompilerKind::Rustdoc => "rustdoc",
        CompilerKind::BuildScript => "build.rs",
        CompilerKind::Linker => "link",
        CompilerKind::Wrapper => "wrapper",
    }
}

/// What an activity is compiling, at whatever precision was resolved.
fn crate_identity_label(compiled_crate_identity: &CompiledCrateIdentity) -> String {
    match compiled_crate_identity {
        CompiledCrateIdentity::WorkspacePackage(package_id) => {
            workspace_package_label(&package_id.repr)
        },
        CompiledCrateIdentity::DependencyPackage(manifest_package_identity) => format!(
            "{} {}",
            manifest_package_identity.name(),
            manifest_package_identity.version(),
        ),
        CompiledCrateIdentity::CrateNameOnly(compiled_crate_name) => {
            compiled_crate_name.as_str().to_string()
        },
    }
}

/// The readable tail of a `PackageId`, whose full representation is a URL-like
/// string too long for a column.
fn workspace_package_label(repr: &str) -> String {
    repr.rsplit(['#', '/']).next().unwrap_or(repr).to_string()
}

/// The scope-level activities no single session claims.
///
/// Every row here is observed-only: an ambiguous compiler names more than one
/// live session, so nothing in this section may authorize a termination.
fn render_unattributed_section(
    frame: &mut Frame,
    area: Rect,
    monitor_columns: MonitorColumns<'_>,
    unattributed_section_layout: UnattributedSectionLayout,
    cursor: &OutputCursor,
    hit_map: &mut MonitorHitMap,
) {
    let width = usize::from(area.width);
    let mut lines = vec![unattributed_caption(unattributed_section_layout)];
    let drawn_rows = usize::from(
        unattributed_section_layout
            .height
            .saturating_sub(UNATTRIBUTED_SECTION_CAPTION_HEIGHT),
    );
    for (row_index, unattributed_activity) in monitor_columns
        .unattributed_activities()
        .iter()
        .enumerate()
        .take(drawn_rows)
    {
        let line = Line::from(Span::raw(format!(
            " {}",
            unattributed_label(unattributed_activity)
        )));
        hit_map.record(
            row_rect(area, lines.len()),
            OutputMonitorHit::Unattributed {
                compile_activity_id: unattributed_activity.compile_activity_id().clone(),
                row_index,
            },
        );
        let selected = matches!(
            cursor.target(),
            OutputCursorTarget::Unattributed(compile_activity_id)
                if compile_activity_id == unattributed_activity.compile_activity_id()
        );
        lines.push(if selected {
            fill_row(line, width)
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The section caption, saying how many activities the section had no room for
/// so a narrowed section never reads as a complete one.
fn unattributed_caption(unattributed_section_layout: UnattributedSectionLayout) -> Line<'static> {
    let style = Style::default().fg(label_color());
    let caption = Span::styled(" Attribution unavailable (observed only)", style);
    if unattributed_section_layout.hidden == 0 {
        return Line::from(caption);
    }
    Line::from(vec![
        caption,
        Span::styled(
            format!(" — {} more", unattributed_section_layout.hidden),
            style,
        ),
    ])
}

/// One unattributed row, naming why it could not be attributed.
fn unattributed_label(unattributed_activity: &UnattributedCompileActivity) -> String {
    let reason = match unattributed_activity.compiler_attribution() {
        CompilerAttribution::Ambiguous { .. } => "ambiguous",
        CompilerAttribution::Confirmed(_)
        | CompilerAttribution::UniqueOutputMatch(_)
        | CompilerAttribution::Unattributed => "no session",
    };
    format!(
        "{} {} — {reason}",
        compiler_kind_label(unattributed_activity.compiler_kind()),
        crate_identity_label(unattributed_activity.compiled_crate_identity()),
    )
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::num::NonZeroU64;
    use std::path::Path;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::super::presentation::MonitorVisibility;
    use super::super::presentation::OutputPresentation;
    use super::*;
    use crate::build_monitor::BuildTerminationLifecycleRegistry;
    use crate::build_monitor::ClassifiedRoot;
    use crate::build_monitor::MonitorSnapshot;
    use crate::build_monitor::classified_monitor_snapshot;
    use crate::process_termination::TerminationTargetResult;
    use crate::project::AbsolutePath;
    use crate::tui::compile_visibility::CompileVisibilityState;
    use crate::tui::compile_visibility::MonitorScopeKey;
    use crate::tui::compile_visibility::MonitorScopeResolution;

    const TERMINATED_ROOT_PID: u32 = 9_421;
    const LIVE_ROOT_PID: u32 = 9_422;

    fn actionable_scope() -> CompileVisibilityState {
        CompileVisibilityState::on_for_test(MonitorScopeResolution::Ready(
            MonitorScopeKey::for_test(AbsolutePath::from(Path::new(
                "/tmp/cargo-port-monitor-render",
            ))),
        ))
    }

    fn snapshot(root_pid: u32) -> MonitorSnapshot { snapshots(&[root_pid]) }

    fn snapshots(root_pids: &[u32]) -> MonitorSnapshot {
        let classified_roots: Vec<_> = root_pids
            .iter()
            .map(|root_pid| ClassifiedRoot {
                root_pid:      *root_pid,
                compiler_pids: &[],
            })
            .collect();
        classified_monitor_snapshot(&classified_roots).unwrap_or_else(|error| {
            panic!("classification fixture should build a snapshot: {error}")
        })
    }

    fn terminal_record_line_text(termination_target_result: TerminationTargetResult) -> String {
        let terminal_snapshot = snapshot(TERMINATED_ROOT_PID);
        let MonitorSnapshot::Fresh(monitor_data) = &terminal_snapshot else {
            panic!("classification fixture should produce fresh data");
        };
        let terminal_row = &monitor_data.session_rows()[0];
        let mut build_termination_lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        build_termination_lifecycle_registry
            .record_external_terminal_for_test(terminal_row, termination_target_result);
        let terminal_record = build_termination_lifecycle_registry
            .terminal_records()
            .next()
            .unwrap_or_else(|| panic!("recording a terminal result should retain one record"));
        terminal_record_line(terminal_record)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn terminal_record_lines_preserve_already_gone_gone_after_signal_and_partial_truth() {
        let cases = [
            (
                TerminationTargetResult::AlreadyGone {
                    pid: TERMINATED_ROOT_PID,
                },
                "already gone",
            ),
            (
                TerminationTargetResult::GoneAfterSignaling {
                    pid: TERMINATED_ROOT_PID,
                },
                "gone after signal",
            ),
            (
                TerminationTargetResult::Survived {
                    pid: TERMINATED_ROOT_PID,
                },
                "termination incomplete",
            ),
        ];

        for (termination_target_result, expected_truth) in cases {
            let rendered = terminal_record_line_text(termination_target_result);
            assert!(
                rendered.contains(&format!("Build pid {TERMINATED_ROOT_PID}")),
                "terminal projection retains the target identity: {rendered:?}"
            );
            assert!(
                rendered.contains(expected_truth),
                "terminal projection preserves its semantic outcome: {rendered:?}"
            );
        }
    }

    #[test]
    fn live_columns_render_retained_terminal_result_detail() {
        let terminal_snapshot = snapshot(TERMINATED_ROOT_PID);
        let MonitorSnapshot::Fresh(terminal_data) = &terminal_snapshot else {
            panic!("classification fixture should produce fresh data");
        };
        let terminal_row = &terminal_data.session_rows()[0];
        let mut build_termination_lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        build_termination_lifecycle_registry.record_external_terminal_for_test(
            terminal_row,
            TerminationTargetResult::GoneAfterSignaling {
                pid: TERMINATED_ROOT_PID,
            },
        );
        let live_snapshot = snapshot(LIVE_ROOT_PID);
        let compile_visibility_state = actionable_scope();
        let output_presentation = OutputPresentation::derive(
            &compile_visibility_state,
            &live_snapshot,
            &build_termination_lifecycle_registry,
            crate::tui::state::OwnedRunOutputStateRef::Absent,
            crate::tui::state::OwnedRunRunningLabelRef::NotRunning,
            OwnedRunCompletionMarker::NotCompleted,
        );
        let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
            output_presentation.monitor()
        else {
            panic!("the unrelated current root should keep a live column on screen");
        };
        assert_eq!(monitor_columns.len(), 1);
        assert_eq!(monitor_columns.terminal_records().count(), 1);

        let backend = TestBackend::new(100, 16);
        let mut terminal =
            Terminal::new(backend).unwrap_or_else(|error| panic!("create test terminal: {error}"));
        let cursor = OutputCursor::empty();
        terminal
            .draw(|frame| {
                let mut hit_map = MonitorHitMap::new();
                render_monitor(
                    frame,
                    frame.area(),
                    MonitorPresentation::Columns(monitor_columns),
                    None,
                    &cursor,
                    SelectedColumnScroll::new(0, 0),
                    &mut hit_map,
                );
            })
            .unwrap_or_else(|error| panic!("draw test frame: {error}"));

        let buffer = terminal.backend().buffer();
        let mut rendered = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                rendered.push_str(buffer[(x, y)].symbol());
            }
            rendered.push('\n');
        }
        assert!(
            rendered.contains(&format!("pid {LIVE_ROOT_PID}")),
            "the unrelated current build remains rendered: {rendered:?}"
        );
        assert!(
            rendered.contains(&format!(
                "Build pid {TERMINATED_ROOT_PID} — gone after signal"
            )),
            "the retained terminal projection names its target and semantic outcome: {rendered:?}"
        );
    }

    #[test]
    fn output_build_set_results_keep_each_aggregate_with_its_transaction_roots() {
        let first_snapshot = snapshots(&[9_431, 9_432]);
        let second_snapshot = snapshots(&[9_433, 9_434]);
        let MonitorSnapshot::Fresh(first_data) = &first_snapshot else {
            panic!("first classification fixture should produce fresh data");
        };
        let MonitorSnapshot::Fresh(second_data) = &second_snapshot else {
            panic!("second classification fixture should produce fresh data");
        };
        let Some(second_transaction_id) = NonZeroU64::new(2) else {
            panic!("two is a nonzero transaction identity");
        };
        let mut registry = BuildTerminationLifecycleRegistry::default();
        registry
            .record_output_build_set_terminal_for_test(NonZeroU64::MIN, first_data.session_rows());
        registry.record_output_build_set_terminal_for_test(
            second_transaction_id,
            second_data.session_rows(),
        );
        let terminal_records: Vec<_> = registry.terminal_records().collect();
        let rendered: Vec<_> = termination_result_lines(&terminal_records)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(rendered.len(), 6);
        assert!(rendered[0].contains("Output build set (2 roots)"));
        assert!(rendered[1].contains("Build pid 9431"));
        assert!(rendered[2].contains("Build pid 9432"));
        assert!(rendered[3].contains("Output build set (2 roots)"));
        assert!(rendered[4].contains("Build pid 9433"));
        assert!(rendered[5].contains("Build pid 9434"));
    }
}
