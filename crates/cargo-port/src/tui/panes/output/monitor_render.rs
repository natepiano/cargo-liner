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
use tui_pane::format_progressive;
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
use super::presentation::OwnedOutputPresentation;
use super::render::fill_row;
use super::render::parse_output_line;
use super::selection::OutputCursor;
use super::selection::OutputCursorTarget;
use super::selection::OutputSelectionRange;
use super::selection::VisualSelectionPermission;
use crate::build_monitor::BuildProfileLabel;
use crate::build_monitor::BuildSessionActivity;
use crate::build_monitor::CargoCommandSelector;
use crate::build_monitor::CargoSubcommand;
use crate::build_monitor::CargoSubcommandRecognition;
use crate::build_monitor::CompileActivity;
use crate::build_monitor::CompiledCrateIdentity;
use crate::build_monitor::CompilerAttribution;
use crate::build_monitor::CompilerKind;
use crate::build_monitor::MonitorSessionOwnership;
use crate::build_monitor::MonitorStaleness;
use crate::build_monitor::SessionScope;
use crate::build_monitor::TargetDirectoryEvidence;
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
    /// `scroll_offset` is [`Viewport::scroll_offset`], the same first visible
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
    hit_map: &mut MonitorHitMap,
) {
    let Some(owned_body) = owned_body else {
        render_monitor(frame, area, monitor_presentation, None, cursor, hit_map);
        return;
    };
    if producer_is_in_scope(monitor_presentation, owned_body.owned.producer()) {
        render_monitor(
            frame,
            area,
            monitor_presentation,
            Some(owned_body),
            cursor,
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
    producing_column(monitor_presentation, producer).is_some()
}

/// The scope's own column for the run that produced the retained output, when
/// the scope still covers it.
fn producing_column(
    monitor_presentation: MonitorPresentation<'_>,
    producer: OwnedRunId,
) -> Option<MonitorColumn<'_>> {
    let MonitorPresentation::Columns(monitor_columns) = monitor_presentation else {
        return None;
    };
    monitor_columns
        .columns()
        .find(|column| column_produced(*column, producer))
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
) -> usize {
    let Some(producing_column) = producing_column(monitor_presentation, producer) else {
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
        .saturating_sub(producing_column.session_row().compile_activities().len())
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
    hit_map: &mut MonitorHitMap,
) {
    if area.height < MONITOR_INDICATOR_HEIGHT {
        return;
    }
    let monitor_staleness = match monitor_presentation {
        MonitorPresentation::Empty(_) => MonitorStaleness::Live,
        MonitorPresentation::Columns(monitor_columns) => monitor_columns.monitor_staleness(),
    };
    let indicator = Rect::new(area.x, area.y, area.width, MONITOR_INDICATOR_HEIGHT);
    frame.render_widget(Paragraph::new(indicator_line(monitor_staleness)), indicator);

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
        MonitorPresentation::Empty(monitor_empty_state) => {
            hit_map.record(HitRegion::Drawn(body), OutputMonitorHit::EmptyMonitor);
            frame.render_widget(Paragraph::new(empty_state_line(monitor_empty_state)), body);
        },
        MonitorPresentation::Columns(monitor_columns) => {
            render_columns(frame, body, monitor_columns, owned_body, cursor, hit_map);
        },
    }
}

/// The always-visible enabled indicator, carrying the staleness marker.
fn indicator_line(monitor_staleness: MonitorStaleness) -> Line<'static> {
    let text = match monitor_staleness {
        MonitorStaleness::Live => " Build monitor on",
        MonitorStaleness::Stale => " Build monitor on — stale",
    };
    Line::from(Span::styled(text, Style::default().fg(label_color())))
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

/// Split `area` into the column strip and the scope-level unattributed section,
/// then draw both.
fn render_columns(
    frame: &mut Frame,
    area: Rect,
    monitor_columns: MonitorColumns<'_>,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
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
    render_column_strip(frame, strip, monitor_columns, owned_body, cursor, hit_map);

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
            column,
            column_index,
            owned_body,
            cursor,
            hit_map,
        );
    }
}

/// One root Cargo invocation: its header fields, then one selectable row per
/// compile activity attributed to it.
fn render_column(
    frame: &mut Frame,
    area: Rect,
    column: MonitorColumn<'_>,
    column_index: usize,
    owned_body: Option<OwnedBody<'_>>,
    cursor: &OutputCursor,
    hit_map: &mut MonitorHitMap,
) {
    let width = usize::from(area.width);
    let now = Instant::now();
    let mut lines =
        Vec::with_capacity(COLUMN_HEADER_HEIGHT + column.session_row().compile_activities().len());

    let header_selected = matches!(
        cursor.target(),
        OutputCursorTarget::Header(build_session_id) if build_session_id == column.build_session_id()
    );
    for (index, text) in header_fields(column, now).into_iter().enumerate() {
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

    for (row_index, compile_activity) in
        column.session_row().compile_activities().iter().enumerate()
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

/// The header fields, in display order: the operative Cargo command and its
/// selectors, the checkout it builds, the resolved profile with the root PID
/// and elapsed time, and what it is doing now.
fn header_fields(column: MonitorColumn<'_>, now: Instant) -> [String; COLUMN_HEADER_HEIGHT] {
    let build_session = column.session_row().build_session();
    let root_observation = build_session.root_observation();
    [
        cargo_command_label(column),
        checkout_label(column),
        format!(
            "{} · pid {} · {}",
            profile_label(column),
            root_observation.root_pid(),
            format_progressive(column.elapsed(now).as_secs()),
        ),
        state_label(column),
    ]
}

/// `cargo <subcommand>` followed by the selectors as the user typed them.
fn cargo_command_label(column: MonitorColumn<'_>) -> String {
    let operative_cargo_command = column
        .session_row()
        .build_session()
        .operative_cargo_command();
    let subcommand = match operative_cargo_command.subcommand() {
        CargoSubcommand::Named(name) => name.as_str(),
        CargoSubcommand::Absent => "",
    };
    let selectors: Vec<String> = operative_cargo_command
        .selectors()
        .iter()
        .map(selector_label)
        .collect();
    let command = if selectors.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", selectors.join(" "))
    };
    let command = command.trim_end();
    // An alias or plugin cannot be decided from argv, so the header says the
    // subcommand was read rather than recognized.
    match column
        .session_row()
        .build_session()
        .cargo_subcommand_recognition()
    {
        CargoSubcommandRecognition::Build | CargoSubcommandRecognition::NonBuild => {
            command.to_string()
        },
        CargoSubcommandRecognition::Unrecognized => format!("{command} (alias)"),
    }
}

/// One selector argument, rendered the way it was written.
fn selector_label(cargo_command_selector: &CargoCommandSelector) -> String {
    match cargo_command_selector {
        CargoCommandSelector::Package(name) => format!("-p {name}"),
        CargoCommandSelector::AllPackages => "--workspace".to_string(),
        CargoCommandSelector::Library => "--lib".to_string(),
        CargoCommandSelector::Binary(name) => format!("--bin {name}"),
        CargoCommandSelector::AllBinaries => "--bins".to_string(),
        CargoCommandSelector::Example(name) => format!("--example {name}"),
        CargoCommandSelector::AllExamples => "--examples".to_string(),
        CargoCommandSelector::Test(name) => format!("--test {name}"),
        CargoCommandSelector::AllTests => "--tests".to_string(),
        CargoCommandSelector::Benchmark(name) => format!("--bench {name}"),
        CargoCommandSelector::AllBenchmarks => "--benches".to_string(),
        CargoCommandSelector::AllTargets => "--all-targets".to_string(),
    }
}

/// The checkout this session builds, falling back to where it writes when no
/// method related it to the project list.
fn checkout_label(column: MonitorColumn<'_>) -> String {
    let build_session = column.session_row().build_session();
    match build_session.session_scope() {
        SessionScope::Resolved { root, .. } => root.path().to_string(),
        SessionScope::Unresolved => match build_session.session_target_directory().evidence() {
            TargetDirectoryEvidence::Determined(canonical_target_directory) => {
                format!("writes {}", canonical_target_directory.path())
            },
            TargetDirectoryEvidence::Unobservable => "checkout unresolved".to_string(),
        },
    }
}

/// The resolved profile, keeping a manifest's custom name.
fn profile_label(column: MonitorColumn<'_>) -> String {
    match column.session_row().build_session().build_profile().label() {
        BuildProfileLabel::Dev => "dev".to_string(),
        BuildProfileLabel::Release => "release".to_string(),
        BuildProfileLabel::Custom(name) => name.clone(),
    }
}

/// What the session is doing this cycle, and whether Cargo Port launched it.
fn state_label(column: MonitorColumn<'_>) -> String {
    let activity = match column.build_session_activity() {
        BuildSessionActivity::Compiling => "compiling",
        BuildSessionActivity::RunningBuildScript => "build script",
        BuildSessionActivity::Linking => "linking",
        BuildSessionActivity::ActiveWithoutCompiler => "active",
    };
    match column.session_ownership() {
        MonitorSessionOwnership::Owned(_) => format!("{activity} · launched here"),
        MonitorSessionOwnership::External => activity.to_string(),
    }
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
