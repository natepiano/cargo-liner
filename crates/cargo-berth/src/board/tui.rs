//! Terminal rendering of the headless [`BoardModel`] projection.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::io::IsTerminal;
use std::io::Stdout;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use serde_json::Map;
use serde_json::Value;
use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::GlobalAction;
use tui_pane::GridLines;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapError;
use tui_pane::NavAction;
use tui_pane::Navigation;
use tui_pane::NoToastAction;
use tui_pane::Pane;
use tui_pane::PaneBorders;
use tui_pane::PaneChrome;
use tui_pane::PaneFrame;
use tui_pane::StatusBar;
use tui_pane::draw_clipped;
use unicode_width::UnicodeWidthStr;

use super::BoardModel;
use super::error::BoardError;
use super::rows::BoardReservationVisibility;
use super::rows::HumanTargetVisibility;
use crate::ledger::JournalPosition;
use crate::ledger::LedgerError;
use crate::reconcile::ReconcileError;

const FOOTER_HEIGHT: u16 = 1;
const HORIZONTAL_SCROLL_STEP: u16 = 4;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const OMITTED_MODEL_FIELDS: &[&str] = &[
    "journal_position",
    "recovered_bypasses_this_invocation",
    "integration_order",
    "targets",
    "git_cost",
];
const RESERVATION_FIELDS: &[&str] = &[
    "ready_now",
    "waiting",
    "unconstrained_reservations",
    "resolved",
];
const CONSTRAINT_FIELDS: &[&str] = &["settled_ordering_constraints", "unresolved_overlaps"];
const ANSWER_FIELDS: &[&str] = &[
    "live_overlap_answers",
    "released_overlap_answer_count",
    "available_forced_permits",
    "bypass_audit",
];
const INCURSION_FIELDS: &[&str] = &["outstanding_incursions", "recorded_incursion_answers"];
const ALERT_FIELDS: &[&str] = &["alerts"];

/// Whether both terminal streams needed by the interactive board are attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalAttachment {
    /// Standard input and standard output are terminals.
    Attached,
    /// At least one required stream is redirected.
    Detached,
}

/// Detect whether the human board can safely take over a terminal.
pub(crate) fn terminal_attachment() -> TerminalAttachment {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        TerminalAttachment::Attached
    } else {
        TerminalAttachment::Detached
    }
}

/// Run the terminal board until its keymap dispatches quit.
pub(crate) fn run(
    source: &mut impl BoardModelSource,
    model: &BoardModel,
) -> Result<(), BoardTerminalViewRunFailure> {
    let mut application =
        BoardApplication::new(model).map_err(BoardTerminalViewRunFailure::BeforeOpening)?;
    let mut terminal = setup_terminal().map_err(|error| {
        BoardTerminalViewRunFailure::BeforeOpening(BoardTerminalViewOpeningFailure::TerminalSetup(
            error,
        ))
    })?;
    let interaction = event_loop(&mut terminal, &mut application, source, &mut TerminalEvents);
    let restoration = restore_terminal(&mut terminal);
    finish_terminal_view(interaction, restoration)
}

fn finish_terminal_view(
    interaction: BoardTerminalViewInteractionOutcome,
    restoration: io::Result<()>,
) -> Result<(), BoardTerminalViewRunFailure> {
    match (interaction, restoration) {
        (BoardTerminalViewInteractionOutcome::Completed, Ok(())) => Ok(()),
        (BoardTerminalViewInteractionOutcome::FailedBeforeFirstFrame(error), Ok(())) => {
            Err(BoardTerminalViewRunFailure::BeforeOpening(
                BoardTerminalViewOpeningFailure::FirstFramePresentation(error),
            ))
        },
        (
            BoardTerminalViewInteractionOutcome::FailedBeforeFirstFrame(frame_presentation),
            Err(restoration),
        ) => Err(BoardTerminalViewRunFailure::BeforeOpening(
            BoardTerminalViewOpeningFailure::FirstFramePresentationAndRestoration {
                frame_presentation,
                restoration,
            },
        )),
        (BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(error), Ok(())) => {
            Err(BoardTerminalViewRunFailure::AfterOpening(
                BoardTerminalViewAfterOpeningFailure::Interaction(error),
            ))
        },
        (BoardTerminalViewInteractionOutcome::Completed, Err(error)) => {
            Err(BoardTerminalViewRunFailure::AfterOpening(
                BoardTerminalViewAfterOpeningFailure::Restoration(error),
            ))
        },
        (
            BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(interaction),
            Err(restoration),
        ) => Err(BoardTerminalViewRunFailure::AfterOpening(
            BoardTerminalViewAfterOpeningFailure::InteractionAndRestoration {
                interaction,
                restoration,
            },
        )),
    }
}

/// Stable identities for the manually rendered board panes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BoardPaneId {
    Overview,
    Reservations,
    Constraints,
    Answers,
    Incursions,
    Alerts,
}

impl BoardPaneId {
    #[cfg(test)]
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::Reservations,
        Self::Constraints,
        Self::Answers,
        Self::Incursions,
        Self::Alerts,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Overview => " Overview ",
            Self::Reservations => " Reservations ",
            Self::Constraints => " Integration constraints ",
            Self::Answers => " Answers and bypasses ",
            Self::Incursions => " Incursions ",
            Self::Alerts => " Alerts ",
        }
    }

    const fn model_fields(self) -> &'static [&'static str] {
        match self {
            Self::Overview => &[],
            Self::Reservations => RESERVATION_FIELDS,
            Self::Constraints => CONSTRAINT_FIELDS,
            Self::Answers => ANSWER_FIELDS,
            Self::Incursions => INCURSION_FIELDS,
            Self::Alerts => ALERT_FIELDS,
        }
    }
}

/// Scrollable text assigned to one terminal pane.
struct BoardPaneDocument {
    text:              String,
    line_count:        usize,
    widest_line_width: usize,
}

impl BoardPaneDocument {
    fn from_model_fields(
        fields: Map<String, Value>,
        visibility: HumanTargetVisibility,
    ) -> Result<Self, BoardTerminalViewOpeningFailure> {
        let mut rendered_fields = fields;
        visibility.omit_json_targets(&mut rendered_fields);
        let text = serde_json::to_string_pretty(&Value::Object(rendered_fields))
            .map_err(BoardTerminalViewOpeningFailure::ModelSerialization)?;
        Ok(Self::from_text(text))
    }

    fn from_text(text: String) -> Self {
        let line_count = text.lines().count();
        let widest_line_width = text
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or_default();
        Self {
            text,
            line_count,
            widest_line_width,
        }
    }
}

/// Fixed pane assignment for every top-level board-model value.
struct BoardPaneDocuments {
    overview:     BoardPaneDocument,
    reservations: BoardPaneDocument,
    constraints:  BoardPaneDocument,
    answers:      BoardPaneDocument,
    incursions:   BoardPaneDocument,
    alerts:       BoardPaneDocument,
}

impl BoardPaneDocuments {
    fn from_model(model: &BoardModel) -> Result<Self, BoardTerminalViewOpeningFailure> {
        let reservation_table = overview_reservation_table(model);
        let model = serde_json::to_value(model)
            .map_err(BoardTerminalViewOpeningFailure::ModelSerialization)?;
        let Value::Object(mut unassigned) = model else {
            return Err(BoardTerminalViewOpeningFailure::SerializedModelWasNotObject);
        };
        let visibility = HumanTargetVisibility::for_target_count(
            unassigned
                .get("targets")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        );
        for field in OMITTED_MODEL_FIELDS {
            unassigned.remove(*field);
        }
        let overview = BoardPaneDocument::from_text(reservation_table);
        let reservations =
            Self::take_document(&mut unassigned, BoardPaneId::Reservations, visibility)?;
        let constraints =
            Self::take_document(&mut unassigned, BoardPaneId::Constraints, visibility)?;
        let answers = Self::take_document(&mut unassigned, BoardPaneId::Answers, visibility)?;
        let incursions = Self::take_document(&mut unassigned, BoardPaneId::Incursions, visibility)?;
        let alerts = Self::take_document(&mut unassigned, BoardPaneId::Alerts, visibility)?;
        if !unassigned.is_empty() {
            return Err(BoardTerminalViewOpeningFailure::UnassignedModelFields(
                unassigned.into_iter().map(|(field, _)| field).collect(),
            ));
        }
        Ok(Self {
            overview,
            reservations,
            constraints,
            answers,
            incursions,
            alerts,
        })
    }

    fn take_document(
        unassigned: &mut Map<String, Value>,
        pane_id: BoardPaneId,
        visibility: HumanTargetVisibility,
    ) -> Result<BoardPaneDocument, BoardTerminalViewOpeningFailure> {
        let mut fields = Map::new();
        for field in pane_id.model_fields() {
            let Some(mut value) = unassigned.remove(*field) else {
                return Err(BoardTerminalViewOpeningFailure::MissingModelField(field));
            };
            if let Value::Object(section) = &mut value {
                section.remove("journal_position");
            }
            fields.insert((*field).to_owned(), value);
        }
        BoardPaneDocument::from_model_fields(fields, visibility)
    }

    const fn get(&self, pane_id: BoardPaneId) -> &BoardPaneDocument {
        match pane_id {
            BoardPaneId::Overview => &self.overview,
            BoardPaneId::Reservations => &self.reservations,
            BoardPaneId::Constraints => &self.constraints,
            BoardPaneId::Answers => &self.answers,
            BoardPaneId::Incursions => &self.incursions,
            BoardPaneId::Alerts => &self.alerts,
        }
    }

    #[cfg(test)]
    fn reassembled_model_value(&self) -> Result<Value, serde_json::Error> {
        let mut model = Map::new();
        for pane_id in BoardPaneId::ALL {
            let document = self.get(pane_id);
            if pane_id == BoardPaneId::Overview {
                continue;
            }
            let fields: Map<String, Value> = serde_json::from_str(&document.text)?;
            for (field, value) in fields {
                assert!(
                    model.insert(field.clone(), value).is_none(),
                    "field assigned twice: {field}"
                );
            }
        }
        Ok(Value::Object(model))
    }
}

fn overview_reservation_table(model: &BoardModel) -> String {
    let mut reservations = model
        .ready_now
        .entries
        .iter()
        .map(|entry| &entry.reservation)
        .chain(model.waiting.entries.iter().map(|entry| &entry.reservation))
        .chain(model.unconstrained_reservations.entries.iter())
        .filter(|reservation| reservation.visibility != BoardReservationVisibility::ResolvedAudit)
        .collect::<Vec<_>>();
    reservations.sort_by_key(|reservation| reservation.reservation_id);
    reservations.dedup_by_key(|reservation| reservation.reservation_id);
    let mut rows = vec![["ID", "Session", "Stage", "Blocking", "Race paths"].map(str::to_owned)];
    rows.extend(
        reservations
            .iter()
            .map(|reservation| reservation.overview_cells()),
    );
    let widths: [usize; 5] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| UnicodeWidthStr::width(row[column].as_str()))
            .max()
            .unwrap_or_default()
    });
    let mut lines = vec!["Reservations".to_owned()];
    for row in &rows {
        let cells = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| {
                format!(
                    "{cell}{}",
                    " ".repeat(width.saturating_sub(UnicodeWidthStr::width(cell.as_str())))
                )
            })
            .collect::<Vec<_>>();
        lines.push(cells.join("  ").trim_end().to_owned());
    }
    if reservations.is_empty() {
        lines.push("no reservations".to_owned());
    }
    lines.join("\n")
}

/// Scroll state retained independently for one board pane.
struct BoardPaneViewport {
    vertical_offset:       usize,
    line_count:            usize,
    visible_rows:          usize,
    horizontal_offset:     u16,
    max_horizontal_offset: u16,
}

impl BoardPaneViewport {
    const fn new() -> Self {
        Self {
            vertical_offset:       0,
            line_count:            0,
            visible_rows:          0,
            horizontal_offset:     0,
            max_horizontal_offset: 0,
        }
    }

    fn observe_layout(&mut self, document: &BoardPaneDocument, area: Rect) {
        self.line_count = document.line_count;
        self.visible_rows = usize::from(area.height);
        self.vertical_offset = self.vertical_offset.min(self.max_vertical_offset());
        let horizontal_overflow = document
            .widest_line_width
            .saturating_sub(usize::from(area.width));
        self.max_horizontal_offset = u16::try_from(horizontal_overflow).unwrap_or(u16::MAX);
        self.horizontal_offset = self.horizontal_offset.min(self.max_horizontal_offset);
    }

    const fn scroll_up(&mut self) { self.vertical_offset = self.vertical_offset.saturating_sub(1); }

    fn scroll_down(&mut self) { self.scroll_down_by(1); }

    const fn scroll_home(&mut self) { self.vertical_offset = 0; }

    const fn scroll_end(&mut self) { self.vertical_offset = self.max_vertical_offset(); }

    fn scroll_page_up(&mut self) { self.scroll_up_by(self.visible_rows.saturating_sub(1).max(1)); }

    fn scroll_page_down(&mut self) {
        self.scroll_down_by(self.visible_rows.saturating_sub(1).max(1));
    }

    fn scroll_half_page_up(&mut self) { self.scroll_up_by((self.visible_rows / 2).max(1)); }

    fn scroll_half_page_down(&mut self) { self.scroll_down_by((self.visible_rows / 2).max(1)); }

    const fn scroll_up_by(&mut self, rows: usize) {
        self.vertical_offset = self.vertical_offset.saturating_sub(rows);
    }

    fn scroll_down_by(&mut self, rows: usize) {
        self.vertical_offset = self
            .vertical_offset
            .saturating_add(rows)
            .min(self.max_vertical_offset());
    }

    const fn max_vertical_offset(&self) -> usize {
        self.line_count.saturating_sub(self.visible_rows)
    }

    const fn pan_left(&mut self) {
        self.horizontal_offset = self
            .horizontal_offset
            .saturating_sub(HORIZONTAL_SCROLL_STEP);
    }

    fn pan_right(&mut self) {
        self.horizontal_offset = self
            .horizontal_offset
            .saturating_add(HORIZONTAL_SCROLL_STEP)
            .min(self.max_horizontal_offset);
    }
}

/// One viewport per registered board pane.
struct BoardPaneViewports {
    overview:     BoardPaneViewport,
    reservations: BoardPaneViewport,
    constraints:  BoardPaneViewport,
    answers:      BoardPaneViewport,
    incursions:   BoardPaneViewport,
    alerts:       BoardPaneViewport,
}

impl BoardPaneViewports {
    const fn new() -> Self {
        Self {
            overview:     BoardPaneViewport::new(),
            reservations: BoardPaneViewport::new(),
            constraints:  BoardPaneViewport::new(),
            answers:      BoardPaneViewport::new(),
            incursions:   BoardPaneViewport::new(),
            alerts:       BoardPaneViewport::new(),
        }
    }

    const fn get_mut(&mut self, pane_id: BoardPaneId) -> &mut BoardPaneViewport {
        match pane_id {
            BoardPaneId::Overview => &mut self.overview,
            BoardPaneId::Reservations => &mut self.reservations,
            BoardPaneId::Constraints => &mut self.constraints,
            BoardPaneId::Answers => &mut self.answers,
            BoardPaneId::Incursions => &mut self.incursions,
            BoardPaneId::Alerts => &mut self.alerts,
        }
    }
}

/// A refresh prerequisite failed while the last good board remains visible.
#[derive(Debug)]
pub(crate) enum BoardRefreshFailure {
    /// The ledger's movement stamp could not be read.
    LedgerRead(LedgerError),
    /// Repository reconciliation failed.
    Reconcile(Box<ReconcileError>),
    /// Repository enrollment is absent at the expected configuration path.
    RepositoryNotConfigured(PathBuf),
    /// Reconciled facts could not be projected into a board.
    Model(BoardError),
    /// The board could not be assigned to terminal panes.
    PaneDocuments(BoardTerminalViewOpeningFailure),
}

impl Display for BoardRefreshFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LedgerRead(error) => error.fmt(formatter),
            Self::Reconcile(error) => error.fmt(formatter),
            Self::RepositoryNotConfigured(path) => {
                write!(formatter, "configuration unavailable: {}", path.display())
            },
            Self::Model(error) => error.fmt(formatter),
            Self::PaneDocuments(error) => error.fmt(formatter),
        }
    }
}

impl Error for BoardRefreshFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LedgerRead(error) => Some(error),
            Self::Reconcile(error) => Some(error.as_ref()),
            Self::RepositoryNotConfigured(_) => None,
            Self::Model(error) => Some(error),
            Self::PaneDocuments(error) => Some(error),
        }
    }
}

/// Rebuilds the board from the same repository invocation as its initial model.
pub(crate) trait BoardModelSource {
    /// Read ledger movement without reconciling repository facts.
    fn current_position(&mut self) -> Result<JournalPosition, BoardRefreshFailure>;
    /// Reconcile repository facts and project a fresh board.
    fn rebuild(&mut self) -> Result<BoardModel, BoardRefreshFailure>;
}

/// Result of a refresh check; failure retains the last good documents and position.
enum BoardRefreshResult {
    /// Fresh documents and their journal position replaced the previous model.
    Rebuilt,
    /// No journal movement requires reconciliation.
    Unchanged,
    /// The last good documents and position remain available for retry.
    Failed(BoardRefreshFailure),
}

/// Application state borrowed back through [`AppContext`].
struct BoardApplication {
    framework:        Framework<Self>,
    keymap:           Rc<Keymap<Self>>,
    documents:        BoardPaneDocuments,
    viewports:        BoardPaneViewports,
    journal_position: JournalPosition,
    refresh_result:   BoardRefreshResult,
}

impl BoardApplication {
    fn new(model: &BoardModel) -> Result<Self, BoardTerminalViewOpeningFailure> {
        Self::from_documents(
            BoardPaneDocuments::from_model(model)?,
            model.journal_position(),
        )
    }

    fn from_documents(
        documents: BoardPaneDocuments,
        journal_position: JournalPosition,
    ) -> Result<Self, BoardTerminalViewOpeningFailure> {
        let mut framework = Framework::new(FocusedPane::App(BoardPaneId::Overview));
        let keymap = Keymap::builder()
            .register_navigation::<BoardNavigation>()
            .map_err(BoardTerminalViewOpeningFailure::Keymap)?
            .register_pane::<OverviewPane>()
            .register_pane::<ReservationsPane>()
            .register_pane::<ConstraintsPane>()
            .register_pane::<AnswersPane>()
            .register_pane::<IncursionsPane>()
            .register_pane::<AlertsPane>()
            .build_into(&mut framework)
            .map_err(BoardTerminalViewOpeningFailure::Keymap)?;
        Ok(Self {
            framework,
            keymap: Rc::new(keymap),
            documents,
            viewports: BoardPaneViewports::new(),
            journal_position,
            refresh_result: BoardRefreshResult::Unchanged,
        })
    }

    fn refresh(&mut self, source: &mut impl BoardModelSource) {
        self.refresh_result = match self.prepare_refresh(source) {
            Ok(result) => result,
            Err(error) => BoardRefreshResult::Failed(error),
        };
    }

    fn prepare_refresh(
        &mut self,
        source: &mut impl BoardModelSource,
    ) -> Result<BoardRefreshResult, BoardRefreshFailure> {
        let position = source.current_position()?;
        if position == self.journal_position
            && !matches!(self.refresh_result, BoardRefreshResult::Failed(_))
        {
            return Ok(BoardRefreshResult::Unchanged);
        }
        let model = source.rebuild()?;
        let documents =
            BoardPaneDocuments::from_model(&model).map_err(BoardRefreshFailure::PaneDocuments)?;
        self.documents = documents;
        self.journal_position = model.journal_position();
        Ok(BoardRefreshResult::Rebuilt)
    }
}

impl AppContext for BoardApplication {
    type AppPaneId = BoardPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

struct OverviewPane;
struct ReservationsPane;
struct ConstraintsPane;
struct AnswersPane;
struct IncursionsPane;
struct AlertsPane;

impl Pane<BoardApplication> for OverviewPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Overview;
}

impl Pane<BoardApplication> for ReservationsPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Reservations;
}

impl Pane<BoardApplication> for ConstraintsPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Constraints;
}

impl Pane<BoardApplication> for AnswersPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Answers;
}

impl Pane<BoardApplication> for IncursionsPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Incursions;
}

impl Pane<BoardApplication> for AlertsPane {
    const APP_PANE_ID: BoardPaneId = BoardPaneId::Alerts;
}

/// Navigation dispatcher over the focused pane's retained viewport.
struct BoardNavigation;

impl Navigation<BoardApplication> for BoardNavigation {
    const SECTION_NAME: &'static str = "Board navigation";

    fn dispatcher() -> fn(NavAction, FocusedPane<BoardPaneId>, &mut BoardApplication) {
        dispatch_navigation
    }
}

fn dispatch_navigation(
    action: NavAction,
    focused: FocusedPane<BoardPaneId>,
    application: &mut BoardApplication,
) {
    let FocusedPane::App(pane_id) = focused else {
        return;
    };
    let viewport = application.viewports.get_mut(pane_id);
    match action {
        NavAction::Up => viewport.scroll_up(),
        NavAction::Down => viewport.scroll_down(),
        NavAction::Left => viewport.pan_left(),
        NavAction::Right => viewport.pan_right(),
        NavAction::Home => viewport.scroll_home(),
        NavAction::End => viewport.scroll_end(),
        NavAction::PageUp => viewport.scroll_page_up(),
        NavAction::PageDown => viewport.scroll_page_down(),
        NavAction::HalfPageUp => viewport.scroll_half_page_up(),
        NavAction::HalfPageDown => viewport.scroll_half_page_down(),
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, BoardTerminalSetupFailure> {
    enable_raw_mode().map_err(BoardTerminalSetupFailure::RawModeAcquisition)?;
    let mut stdout = io::stdout();
    if let Err(acquisition) = execute!(stdout, EnterAlternateScreen) {
        let rollback = TerminalSetupRollbackOutcome::after_raw_mode(disable_raw_mode());
        return Err(BoardTerminalSetupFailure::AlternateScreenAcquisition {
            acquisition,
            rollback,
        });
    }
    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => Ok(terminal),
        Err(construction) => {
            let mut stdout = io::stdout();
            let alternate_screen = execute!(stdout, LeaveAlternateScreen);
            let raw_mode = disable_raw_mode();
            let rollback = TerminalSetupRollbackOutcome::after_terminal_acquisition(
                alternate_screen,
                raw_mode,
            );
            Err(BoardTerminalSetupFailure::Construction {
                construction,
                rollback,
            })
        },
    }
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let raw_mode = disable_raw_mode();
    let alternate_screen = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor = terminal.show_cursor();
    raw_mode.and(alternate_screen).and(cursor)
}

/// Events and monotonic time share one boundary so refresh deadlines are testable.
trait BoardEvents {
    fn now(&self) -> Instant;
    fn next_event(&mut self, timeout: Duration) -> io::Result<BoardEvent>;
}

enum BoardEvent {
    Input(Event),
    DeadlineReached,
}

struct TerminalEvents;

impl BoardEvents for TerminalEvents {
    fn now(&self) -> Instant { Instant::now() }

    fn next_event(&mut self, timeout: Duration) -> io::Result<BoardEvent> {
        if event::poll(timeout)? {
            event::read().map(BoardEvent::Input)
        } else {
            Ok(BoardEvent::DeadlineReached)
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<impl Backend<Error: Error + Send + Sync + 'static>>,
    application: &mut BoardApplication,
    source: &mut impl BoardModelSource,
    events: &mut impl BoardEvents,
) -> BoardTerminalViewInteractionOutcome {
    if let Err(error) = terminal.draw(|frame| draw(frame, application)) {
        return BoardTerminalViewInteractionOutcome::FailedBeforeFirstFrame(io::Error::other(
            error,
        ));
    }
    let mut refresh_deadline = events.now() + REFRESH_INTERVAL;
    while !application.framework.quit_requested() {
        if events.now() >= refresh_deadline {
            refresh_deadline = events.now() + REFRESH_INTERVAL;
            application.refresh(source);
            if let Err(error) = terminal.draw(|frame| draw(frame, application)) {
                return BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(
                    io::Error::other(error),
                );
            }
        }
        let event = match events
            .next_event(refresh_deadline.saturating_duration_since(events.now()))
        {
            Ok(event) => event,
            Err(error) => return BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(error),
        };
        match event {
            BoardEvent::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                handle_key(application, key);
            },
            BoardEvent::Input(_) | BoardEvent::DeadlineReached => {},
        }
        if !application.framework.quit_requested()
            && let Err(error) = terminal.draw(|frame| draw(frame, application))
        {
            return BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(io::Error::other(
                error,
            ));
        }
    }
    BoardTerminalViewInteractionOutcome::Completed
}

enum BoardTerminalViewInteractionOutcome {
    Completed,
    FailedBeforeFirstFrame(io::Error),
    FailedAfterFirstFrame(io::Error),
}

fn handle_key(application: &mut BoardApplication, key: KeyEvent) {
    let keymap = Rc::clone(&application.keymap);
    let binding = KeyBind::from(key);
    if let Some(action) = keymap.framework_globals().action_for(&binding) {
        if matches!(
            action,
            GlobalAction::Quit | GlobalAction::NextPane | GlobalAction::PrevPane
        ) {
            keymap.dispatch_framework_global(action, application);
        }
        return;
    }
    if let FocusedPane::App(pane_id) = *application.framework.focused()
        && keymap.dispatch_app_pane(pane_id, &binding, application) == KeyOutcome::Consumed
    {
        return;
    }
    if let Some(action) = keymap
        .navigation()
        .and_then(|navigation| navigation.action_for(&binding))
    {
        (BoardNavigation::dispatcher())(action, *application.framework.focused(), application);
    }
}

fn draw(frame: &mut Frame, application: &mut BoardApplication) {
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(FOOTER_HEIGHT)])
        .areas(frame.area());
    draw_focused_pane(frame, application, body);
    draw_footer(frame, application, footer);
}

fn draw_focused_pane(frame: &mut Frame, application: &mut BoardApplication, area: Rect) {
    let FocusedPane::App(pane_id) = *application.framework.focused() else {
        return;
    };
    let document = application.documents.get(pane_id);
    let viewport = application.viewports.get_mut(pane_id);
    let pane_frame = PaneFrame::new(area).with_focus(true);
    let inner = pane_frame.inner();
    viewport.observe_layout(document, inner);
    let vertical_offset = u16::try_from(viewport.vertical_offset).unwrap_or(u16::MAX);
    draw_clipped(frame.buffer_mut(), pane_frame, |buffer, content_area| {
        Paragraph::new(document.text.as_str())
            .scroll((vertical_offset, viewport.horizontal_offset))
            .render(content_area, buffer);
    });
    let mut grid = GridLines::new(area);
    grid.add_titled(pane_frame, pane_id.title());
    grid.render(frame.buffer_mut(), pane_chrome(), PaneBorders::Separate);
}

fn draw_footer(frame: &mut Frame, application: &BoardApplication, area: Rect) {
    if let BoardRefreshResult::Failed(error) = &application.refresh_result {
        frame.render_widget(Paragraph::new(format!("Refresh failed: {error}")), area);
        return;
    }
    let status_bar = status_bar(&application.keymap);
    let [navigation, lifecycle] =
        Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(status_bar.nav)), navigation);
    frame.render_widget(
        Paragraph::new(Line::from(status_bar.global)).alignment(Alignment::Right),
        lifecycle,
    );
}

fn status_bar(keymap: &Keymap<BoardApplication>) -> StatusBar {
    let navigation = keymap.navigation();
    let vertical = navigation
        .and_then(|scope| scope.key_for(NavAction::Up))
        .map_or_else(
            || "Up/Down".to_owned(),
            tui_pane::KeySequence::display_short,
        );
    let horizontal = navigation
        .and_then(|scope| scope.key_for(NavAction::Left))
        .map_or_else(
            || "Left/Right".to_owned(),
            tui_pane::KeySequence::display_short,
        );
    let pane = keymap
        .framework_globals()
        .key_for(GlobalAction::NextPane)
        .map_or_else(|| "Tab".to_owned(), tui_pane::KeySequence::display_short);
    let quit = keymap
        .framework_globals()
        .key_for(GlobalAction::Quit)
        .map_or_else(|| "q".to_owned(), tui_pane::KeySequence::display_short);
    StatusBar {
        nav:         vec![
            footer_key(vertical),
            Span::raw(" scroll  "),
            footer_key(horizontal),
            Span::raw(" pan  "),
            footer_key(pane),
            Span::raw(" pane"),
        ],
        pane_action: Vec::new(),
        global:      vec![footer_key(quit), Span::raw(" quit ")],
    }
}

fn footer_key(key: String) -> Span<'static> {
    Span::styled(
        key,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

const fn pane_chrome() -> PaneChrome {
    PaneChrome {
        active_border:   Style::new().fg(Color::Cyan),
        inactive_border: Style::new().fg(Color::DarkGray),
        active_title:    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        inactive_title:  Style::new().fg(Color::Gray),
    }
}

/// Why terminal acquisition did not produce a usable terminal view.
#[derive(Debug)]
pub(crate) enum BoardTerminalSetupFailure {
    /// Raw mode could not be acquired, so no terminal state needs rolling back.
    RawModeAcquisition(io::Error),
    /// Entering the alternate screen failed after raw mode was acquired.
    AlternateScreenAcquisition {
        /// The error returned while entering the alternate screen.
        acquisition: io::Error,
        /// Whether the acquired raw mode was restored.
        rollback:    TerminalSetupRollbackOutcome,
    },
    /// Ratatui terminal construction failed after terminal state was acquired.
    Construction {
        /// The error returned while constructing the terminal backend.
        construction: io::Error,
        /// Whether raw mode and the alternate screen were restored.
        rollback:     TerminalSetupRollbackOutcome,
    },
}

impl Display for BoardTerminalSetupFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RawModeAcquisition(error) => {
                write!(
                    formatter,
                    "terminal setup failed while enabling raw mode: {error}"
                )
            },
            Self::AlternateScreenAcquisition {
                acquisition,
                rollback,
            } => {
                write!(
                    formatter,
                    "terminal setup failed while entering the alternate screen: {acquisition}"
                )?;
                rollback.fmt_after_setup_failure(formatter)
            },
            Self::Construction {
                construction,
                rollback,
            } => {
                write!(
                    formatter,
                    "terminal setup failed while constructing the terminal view: {construction}"
                )?;
                rollback.fmt_after_setup_failure(formatter)
            },
        }
    }
}

impl Error for BoardTerminalSetupFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RawModeAcquisition(error) => Some(error),
            Self::AlternateScreenAcquisition { acquisition, .. } => Some(acquisition),
            Self::Construction { construction, .. } => Some(construction),
        }
    }
}

/// Whether setup failure cleanup restored every terminal state it had acquired.
#[derive(Debug)]
pub(crate) enum TerminalSetupRollbackOutcome {
    /// Every acquired terminal state was restored.
    Restored,
    /// At least one restoration operation failed, so manual reset may be necessary.
    MayNeedReset(TerminalSetupRollbackFailure),
}

impl TerminalSetupRollbackOutcome {
    fn after_raw_mode(raw_mode: io::Result<()>) -> Self {
        match raw_mode {
            Ok(()) => Self::Restored,
            Err(error) => Self::MayNeedReset(TerminalSetupRollbackFailure::RawMode(error)),
        }
    }

    fn after_terminal_acquisition(
        alternate_screen: io::Result<()>,
        raw_mode: io::Result<()>,
    ) -> Self {
        match (alternate_screen, raw_mode) {
            (Ok(()), Ok(())) => Self::Restored,
            (Err(error), Ok(())) => {
                Self::MayNeedReset(TerminalSetupRollbackFailure::AlternateScreen(error))
            },
            (Ok(()), Err(error)) => {
                Self::MayNeedReset(TerminalSetupRollbackFailure::RawMode(error))
            },
            (Err(alternate_screen), Err(raw_mode)) => {
                Self::MayNeedReset(TerminalSetupRollbackFailure::AlternateScreenAndRawMode {
                    alternate_screen,
                    raw_mode,
                })
            },
        }
    }

    fn fmt_after_setup_failure(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Restored => Ok(()),
            Self::MayNeedReset(failure) => write!(
                formatter,
                "; terminal rollback failed: {failure}; the terminal may need to be reset"
            ),
        }
    }
}

/// Terminal state that setup failure cleanup could not restore.
#[derive(Debug)]
pub(crate) enum TerminalSetupRollbackFailure {
    /// Raw mode could not be disabled.
    RawMode(io::Error),
    /// The alternate screen could not be left.
    AlternateScreen(io::Error),
    /// Neither the alternate screen nor raw mode could be restored.
    AlternateScreenAndRawMode {
        /// The error returned while leaving the alternate screen.
        alternate_screen: io::Error,
        /// The error returned while disabling raw mode.
        raw_mode:         io::Error,
    },
}

impl Display for TerminalSetupRollbackFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RawMode(error) => write!(formatter, "raw mode was not disabled ({error})"),
            Self::AlternateScreen(error) => {
                write!(formatter, "the alternate screen was not left ({error})")
            },
            Self::AlternateScreenAndRawMode {
                alternate_screen,
                raw_mode,
            } => write!(
                formatter,
                "the alternate screen was not left ({alternate_screen}) and raw mode was not disabled ({raw_mode})"
            ),
        }
    }
}

/// Whether the terminal board failed before or after the user could see it.
#[derive(Debug)]
pub(crate) enum BoardTerminalViewRunFailure {
    /// Model projection, setup, or first-frame presentation prevented opening.
    BeforeOpening(BoardTerminalViewOpeningFailure),
    /// Interaction or restoration failed after a completed frame was presented.
    AfterOpening(BoardTerminalViewAfterOpeningFailure),
}

impl Display for BoardTerminalViewRunFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeOpening(failure) => failure.fmt(formatter),
            Self::AfterOpening(failure) => failure.fmt(formatter),
        }
    }
}

impl Error for BoardTerminalViewRunFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BeforeOpening(failure) => Some(failure),
            Self::AfterOpening(failure) => Some(failure),
        }
    }
}

/// A failure that prevented the terminal board from opening.
#[derive(Debug)]
pub(crate) enum BoardTerminalViewOpeningFailure {
    /// Serializing the already-built model or one of its exact pane slices failed.
    ModelSerialization(serde_json::Error),
    /// The model stopped serializing as the top-level object its JSON contract promises.
    SerializedModelWasNotObject,
    /// A field assigned to a pane disappeared from the model.
    MissingModelField(&'static str),
    /// A new model field has no deliberate terminal-pane assignment yet.
    UnassignedModelFields(Vec<String>),
    /// The registered pane or navigation keymap is internally inconsistent.
    Keymap(KeymapError),
    /// Terminal setup failed before the first frame could be presented.
    TerminalSetup(BoardTerminalSetupFailure),
    /// The first board frame could not be presented.
    FirstFramePresentation(io::Error),
    /// The first frame failed and the acquired terminal state could not be fully restored.
    FirstFramePresentationAndRestoration {
        /// The first-frame presentation failure.
        frame_presentation: io::Error,
        /// The terminal restoration failure.
        restoration:        io::Error,
    },
}

impl Display for BoardTerminalViewOpeningFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelSerialization(error) => {
                write!(formatter, "the board model could not be projected: {error}")
            },
            Self::SerializedModelWasNotObject => {
                formatter.write_str("the board model did not serialize as an object")
            },
            Self::MissingModelField(field) => {
                write!(formatter, "the board model no longer carries `{field}`")
            },
            Self::UnassignedModelFields(fields) => write!(
                formatter,
                "the terminal view has no pane assignment for {}",
                fields.join(", ")
            ),
            Self::Keymap(error) => write!(formatter, "the board keymap is invalid: {error}"),
            Self::TerminalSetup(error) => error.fmt(formatter),
            Self::FirstFramePresentation(error) => {
                write!(
                    formatter,
                    "the first board frame could not be presented: {error}"
                )
            },
            Self::FirstFramePresentationAndRestoration {
                frame_presentation,
                restoration,
            } => write!(
                formatter,
                "the first board frame could not be presented ({frame_presentation}) and terminal restoration also failed ({restoration}); the terminal may need to be reset"
            ),
        }
    }
}

impl Error for BoardTerminalViewOpeningFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ModelSerialization(error) => Some(error),
            Self::Keymap(error) => Some(error),
            Self::TerminalSetup(error) => Some(error),
            Self::FirstFramePresentation(error)
            | Self::FirstFramePresentationAndRestoration {
                frame_presentation: error,
                ..
            } => Some(error),
            Self::SerializedModelWasNotObject
            | Self::MissingModelField(_)
            | Self::UnassignedModelFields(_) => None,
        }
    }
}

/// A terminal-board failure after the view opened and displayed model facts.
#[derive(Debug)]
pub(crate) enum BoardTerminalViewAfterOpeningFailure {
    /// Input polling or frame rendering failed after acquisition.
    Interaction(io::Error),
    /// Raw-mode, alternate-screen, or cursor restoration failed.
    Restoration(io::Error),
    /// Interaction and the subsequent restoration both failed.
    InteractionAndRestoration {
        /// The input or rendering failure.
        interaction: io::Error,
        /// The restoration failure.
        restoration: io::Error,
    },
}

impl Display for BoardTerminalViewAfterOpeningFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interaction(error) => {
                write!(formatter, "terminal input or rendering failed: {error}")
            },
            Self::Restoration(error) => {
                write!(formatter, "terminal restoration failed: {error}")
            },
            Self::InteractionAndRestoration {
                interaction,
                restoration,
            } => write!(
                formatter,
                "terminal input or rendering failed ({interaction}) and restoration also failed ({restoration})"
            ),
        }
    }
}

impl Error for BoardTerminalViewAfterOpeningFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Interaction(error) | Self::Restoration(error) => Some(error),
            Self::InteractionAndRestoration { interaction, .. } => Some(interaction),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "tests should stop when a pane document stops being valid JSON"
)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::VecDeque;
    use std::fs;
    use std::io::Write;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::TerminalOptions;
    use ratatui::Viewport;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::answer::ConflictAuthorization;
    use crate::board::test_support;
    use crate::board::test_support::BoardFixture;
    use crate::board::test_support::FixtureResult;
    use crate::board::test_support::OverlapAnswerFixture;
    use crate::ledger::Ledger;
    use crate::ledger::WorktreeContext;
    use crate::reservation::ReleaseDisposition;
    use crate::session::SessionReservationIdentity;

    const EXPECTED_OMITTED_FIELDS: &[&str] = &[
        "journal_position",
        "recovered_bypasses_this_invocation",
        "integration_order",
        "targets",
        "git_cost",
    ];

    const MARKER_ID: &str = "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";

    struct ScriptedModelSource {
        position: JournalPosition,
        rebuilds: usize,
        checks:   usize,
        models:   VecDeque<Result<BoardModel, BoardRefreshFailure>>,
    }

    impl ScriptedModelSource {
        fn new(model: BoardModel) -> Self {
            Self {
                position: model.journal_position(),
                rebuilds: 0,
                checks:   0,
                models:   [Ok(model)].into(),
            }
        }
    }

    impl BoardModelSource for ScriptedModelSource {
        fn current_position(&mut self) -> Result<JournalPosition, BoardRefreshFailure> {
            self.checks += 1;
            Ok(self.position)
        }

        fn rebuild(&mut self) -> Result<BoardModel, BoardRefreshFailure> {
            self.rebuilds += 1;
            self.models
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected rebuild"))
        }
    }

    struct ReconciledFixtureSource {
        fixture: BoardFixture,
        ledger:  Ledger,
    }

    impl BoardModelSource for ReconciledFixtureSource {
        fn current_position(&mut self) -> Result<JournalPosition, BoardRefreshFailure> {
            self.ledger
                .current_journal_position()
                .map_err(BoardRefreshFailure::LedgerRead)
        }

        fn rebuild(&mut self) -> Result<BoardModel, BoardRefreshFailure> {
            // `ReconciledFixtureSource::rebuild` uses real reconciliation and `BoardModel::build`.
            Ok(self
                .fixture
                .model()
                .unwrap_or_else(|error| panic!("fixture rebuild: {error}")))
        }
    }

    struct ScriptedEvents {
        now:    Instant,
        inputs: VecDeque<(Duration, BoardEvent)>,
    }

    impl ScriptedEvents {
        fn idle_then_quit() -> Self {
            Self {
                now:    Instant::now(),
                inputs: [
                    (REFRESH_INTERVAL, BoardEvent::DeadlineReached),
                    (
                        Duration::ZERO,
                        BoardEvent::Input(Event::Key(KeyEvent::new(
                            KeyCode::Char('q'),
                            KeyModifiers::NONE,
                        ))),
                    ),
                ]
                .into(),
            }
        }
    }

    impl BoardEvents for ScriptedEvents {
        fn now(&self) -> Instant { self.now }

        fn next_event(&mut self, timeout: Duration) -> io::Result<BoardEvent> {
            let (elapsed, event) = self
                .inputs
                .pop_front()
                .ok_or_else(|| io::Error::other("script exhausted"))?;
            assert!(
                elapsed <= timeout,
                "the loop must poll only until its deadline"
            );
            self.now += elapsed;
            Ok(event)
        }
    }

    fn rendered_application(application: &mut BoardApplication) -> FixtureResult<String> {
        let mut terminal = Terminal::new(TestBackend::new(160, 60))?;
        terminal.draw(|frame| draw(frame, application))?;
        Ok(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect())
    }

    fn changed_model() -> BoardModel {
        let mut value = model_value(&[]);
        value["journal_position"]["generation"] = serde_json::json!(10);
        value["journal_position"]["journal_byte_offset"] = serde_json::json!(500);
        serde_json::from_value(value).unwrap_or_else(|error| panic!("changed fixture: {error}"))
    }

    #[test]
    fn refresh_renders_a_new_reservation_in_overview_and_reservations() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let initial = fixture.model()?;
        let mut application = BoardApplication::new(&initial)?;
        let ledger = Ledger::open(fixture.repository.path())?;
        assert_eq!(
            ledger.current_journal_position()?,
            initial.journal_position()
        );
        assert!(!rendered_application(&mut application)?.contains("arrived.rs"));
        fixture.claim(
            &fixture.main_actor(),
            "arrived.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let mut source = ScriptedModelSource::new(fixture.model()?);
        assert_eq!(source.position, ledger.current_journal_position()?);
        application.refresh(&mut source);
        assert!(matches!(
            application.refresh_result,
            BoardRefreshResult::Rebuilt
        ));
        assert!(rendered_application(&mut application)?.contains("arrived.rs"));
        handle_key(
            &mut application,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert!(rendered_application(&mut application)?.contains("arrived.rs"));
        assert_eq!(application.journal_position, source.position);
        Ok(())
    }

    #[test]
    fn undecodable_projection_triggers_refresh_and_is_republished() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let initial = fixture.model()?;
        let mut application = BoardApplication::new(&initial)?;
        let ledger = Ledger::open(fixture.repository.path())?;
        let projection_path = fixture
            .repository
            .path()
            .join(".git/cargo-berth/reservations.json");
        let mut source = ReconciledFixtureSource { fixture, ledger };
        for invalid_projection in ["not JSON", r#"{"schema_version":4294967295}"#] {
            fs::write(&projection_path, invalid_projection)?;
            let position = source.current_position()?;
            assert!(matches!(position, JournalPosition::Undecodable { .. }));
            assert_ne!(position, application.journal_position);
            // `source.current_position` must leave the cache at `projection_path`
            // untouched until the rebuild.
            assert_eq!(fs::read_to_string(&projection_path)?, invalid_projection);
            application.refresh(&mut source);
            assert!(matches!(
                application.refresh_result,
                BoardRefreshResult::Rebuilt
            ));
            let republished = source.current_position()?;
            assert!(matches!(republished, JournalPosition::Published { .. }));
            assert_eq!(republished, application.journal_position);
            application.refresh(&mut source);
            assert!(matches!(
                application.refresh_result,
                BoardRefreshResult::Unchanged
            ));
        }
        Ok(())
    }

    #[test]
    fn failed_refresh_keeps_rows_and_position_and_retries() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        fixture.claim(
            &fixture.main_actor(),
            "previous.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let initial = fixture.model()?;
        let mut application = BoardApplication::new(&initial)?;
        fixture.claim(
            &fixture.main_actor(),
            "arrived.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let mut source = ScriptedModelSource::new(fixture.model()?);
        source
            .models
            .push_front(Err(BoardRefreshFailure::LedgerRead(LedgerError::Io(
                io::Error::other("cannot read repository"),
            ))));
        application.refresh(&mut source);
        assert_eq!(application.journal_position, initial.journal_position());
        let screen = rendered_application(&mut application)?;
        assert!(screen.contains("previous.rs"));
        assert!(!screen.contains("arrived.rs"));
        assert!(screen.contains("Refresh failed: ledger I/O failed: cannot read repository"));
        application.refresh(&mut source);
        assert_eq!(source.rebuilds, 2);
        assert_eq!(application.journal_position, source.position);
        let screen = rendered_application(&mut application)?;
        assert!(screen.contains("arrived.rs"));
        assert!(!screen.contains("Refresh failed"));
        Ok(())
    }

    #[test]
    fn refresh_preserves_pane_selection_and_clamps_every_scroll_offset() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        fixture.claim(
            &fixture.main_actor(),
            "a_very_long_original_reservation_path_for_horizontal_scrolling.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let mut initial_value = serde_json::to_value(fixture.model()?)?;
        let alert = model_value(&[])["alerts"]["entries"][0].clone();
        initial_value["alerts"]["entries"] = serde_json::json!(vec![alert.clone(); 12]);
        let initial = serde_json::from_value(initial_value)?;
        let mut application = BoardApplication::new(&initial)?;
        let mut terminal = Terminal::new(TestBackend::new(50, 8))?;
        let mut offsets = Vec::new();
        // Start `application.viewports` at valid offsets in the old `application.documents`,
        // including panes that will shrink.
        for pane in BoardPaneId::ALL {
            assert_eq!(application.framework.focused(), &FocusedPane::App(pane));
            terminal.draw(|frame| draw(frame, &mut application))?;
            let viewport = application.viewports.get_mut(pane);
            viewport.vertical_offset = viewport.max_vertical_offset();
            viewport.horizontal_offset = viewport.max_horizontal_offset;
            offsets.push((pane, viewport.vertical_offset, viewport.horizontal_offset));
            handle_key(
                &mut application,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            );
        }
        fixture.claim(
            &fixture.main_actor(),
            "new_reservation.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let mut replacement = serde_json::to_value(fixture.model()?)?;
        replacement["alerts"]["entries"] = serde_json::json!([alert]);
        let mut source = ScriptedModelSource::new(serde_json::from_value(replacement)?);
        let new_documents = BoardPaneDocuments::from_model(
            source
                .models
                .front()
                .and_then(|model| model.as_ref().ok())
                .ok_or("missing replacement")?,
        )?;
        assert!(
            new_documents.reservations.line_count > application.documents.reservations.line_count
        );
        assert!(new_documents.alerts.line_count < application.documents.alerts.line_count);
        handle_key(
            &mut application,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        );
        application.refresh(&mut source);
        assert_eq!(
            application.framework.focused(),
            &FocusedPane::App(BoardPaneId::Reservations)
        );
        for (pane, old_vertical, old_horizontal) in offsets {
            // Draw each `pane` with `terminal` at the same size, as when the user next visits it.
            while application.framework.focused() != &FocusedPane::App(pane) {
                handle_key(
                    &mut application,
                    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                );
            }
            terminal.draw(|frame| draw(frame, &mut application))?;
            let viewport = application.viewports.get_mut(pane);
            assert_eq!(
                viewport.vertical_offset,
                old_vertical.min(viewport.max_vertical_offset())
            );
            assert_eq!(
                viewport.horizontal_offset,
                old_horizontal.min(viewport.max_horizontal_offset)
            );
            if pane == BoardPaneId::Alerts {
                assert!(
                    old_vertical > viewport.vertical_offset,
                    "shrinking content must clamp a previously valid offset"
                );
            }
            if pane == BoardPaneId::Reservations {
                assert!(old_vertical > 0);
                assert_eq!(
                    viewport.vertical_offset, old_vertical,
                    "growing content keeps the offset"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn refresh_failures_keep_their_source_and_name_missing_configuration() {
        let failure = BoardRefreshFailure::LedgerRead(LedgerError::Io(io::Error::other(
            "journal unavailable",
        )));
        assert_eq!(
            failure.to_string(),
            "ledger I/O failed: journal unavailable"
        );
        assert!(failure.source().is_some());
        let missing =
            BoardRefreshFailure::RepositoryNotConfigured(PathBuf::from("/repository/berth.toml"));
        assert_eq!(
            missing.to_string(),
            "configuration unavailable: /repository/berth.toml"
        );
        assert!(missing.source().is_none());
    }

    #[test]
    fn unchanged_journal_position_skips_model_reconciliation() -> FixtureResult<()> {
        let initial = model(&[]);
        let mut application = BoardApplication::new(&initial)?;
        let mut source = ScriptedModelSource::new(initial);
        let mut events = ScriptedEvents::idle_then_quit();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        assert!(matches!(
            event_loop(&mut terminal, &mut application, &mut source, &mut events),
            BoardTerminalViewInteractionOutcome::Completed
        ));
        assert_eq!(source.checks, 1);
        assert_eq!(source.rebuilds, 0);
        Ok(())
    }

    #[test]
    fn sustained_input_refreshes_at_the_monotonic_deadline() -> FixtureResult<()> {
        let mut application = BoardApplication::new(&model(&[]))?;
        let mut source = ScriptedModelSource::new(changed_model());
        let mut events = ScriptedEvents {
            now:    Instant::now(),
            inputs: (0..10)
                .map(|_| {
                    (
                        Duration::from_millis(100),
                        BoardEvent::Input(Event::Resize(100, 30)),
                    )
                })
                .chain([(
                    Duration::ZERO,
                    BoardEvent::Input(Event::Key(KeyEvent::new(
                        KeyCode::Char('q'),
                        KeyModifiers::NONE,
                    ))),
                )])
                .collect(),
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        assert!(matches!(
            event_loop(&mut terminal, &mut application, &mut source, &mut events),
            BoardTerminalViewInteractionOutcome::Completed
        ));
        assert_eq!(source.checks, 1);
        assert_eq!(source.rebuilds, 1);
        assert_eq!(application.journal_position, source.position);
        Ok(())
    }

    struct FirstFrameWriteFailure;

    impl Write for FirstFrameWriteFailure {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("first frame write failed"))
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn model_value(recovered_bypasses: &[&str]) -> Value {
        let position = serde_json::json!({
            "generation": 9,
            "journal_byte_offset": 412,
        });
        let section = |entry: Value| {
            serde_json::json!({
                "journal_position": position,
                "entries": [entry],
            })
        };
        let empty_section = || {
            serde_json::json!({
                "journal_position": position,
                "entries": [],
            })
        };
        serde_json::json!({
            "journal_position": position,
            "recovered_bypasses_this_invocation": recovered_bypasses,
            "integration_order": "constraints_recorded",
            "targets": [],
            "ready_now": empty_section(),
            "waiting": empty_section(),
            "settled_ordering_constraints": empty_section(),
            "unresolved_overlaps": empty_section(),
            "live_overlap_answers": empty_section(),
            "released_overlap_answer_count": 0,
            "unconstrained_reservations": empty_section(),
            "resolved": empty_section(),
            "available_forced_permits": empty_section(),
            "bypass_audit": empty_section(),
            "outstanding_incursions": empty_section(),
            "recorded_incursion_answers": empty_section(),
            "alerts": section(serde_json::json!({
                "kind": "stale_reservation",
                "reservation_id": RESERVATION_ID,
                "freshness": {
                    "status": "stale",
                    "last_activity_at": "2020-01-01T00:00:00.000Z"
                },
                "resolution": {"action": "renew", "reservation_id": RESERVATION_ID}
            })),
            "git_cost": {
                "trunk_resolution_calls": 1,
                "worktree_list_calls": 1,
                "reservation_evidence_revalidations": 1,
                "protected_predecessor_ancestry_queries": 1,
                "worktree_ahead_behind_computations": 1,
                "orphan_recovery_evidence_queries": 1
            }
        })
    }

    fn model(recovered_bypasses: &[&str]) -> BoardModel {
        serde_json::from_value(model_value(recovered_bypasses))
            .unwrap_or_else(|error| panic!("fixture should deserialize as BoardModel: {error}"))
    }

    fn render_overview(model: &BoardModel) -> FixtureResult<String> {
        let mut application = BoardApplication::new(model)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| draw(frame, &mut application))?;
        let buffer = terminal.backend().buffer();
        Ok((0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[test]
    fn overview_renders_active_unconstrained_reservation() -> FixtureResult<()> {
        let board = BoardFixture::new()?;
        let actor = board.main_actor();
        let claim = board.claim(&actor, "src/lib.rs", ConflictAuthorization::NoConflict)?;
        let reservations = Ledger::open(board.repository.path())?
            .read_validated_journal()?
            .into_reservations()?;
        let run = reservations.reservation(claim.reservation_id)?.actor().run;
        let context = WorktreeContext::discover(board.repository.path())?;
        fs::write(
            context.ledger_directory().join("session-identities.json"),
            b"{\"identities\":{}}",
        )?;
        let model = board.model()?;

        let rendered = render_overview(&model)?;
        let row = rendered
            .lines()
            .find(|line| line.contains("src/lib.rs"))
            .ok_or_else(|| io::Error::other("Overview should show the reserved race path"))?;
        let id = claim.reservation_id.to_string();
        assert!(
            row.contains(id.rsplit('-').next().unwrap_or_default()),
            "{row}"
        );
        assert!(row.contains("active"), "{row}");
        assert!(row.contains("blocking"), "{row}");
        let run = run.to_string();
        assert!(
            row.contains(&format!(
                "run {}",
                run.rsplit('-').next().unwrap_or_default()
            )),
            "{row}"
        );
        assert!(rendered.contains("Session"), "{rendered}");
        assert!(rendered.contains("Race paths"), "{rendered}");
        let wire = serde_json::to_value(&model)?;
        assert!(
            wire["unconstrained_reservations"]["entries"][0]
                .get("session")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn overview_distinguishes_sibling_phase_sessions_in_one_run() -> FixtureResult<()> {
        let board = BoardFixture::new()?;
        let actor = board.main_actor();
        let first = board.claim(&actor, "a.rs", ConflictAuthorization::NoConflict)?;
        let second = board.claim(&actor, "b.rs", ConflictAuthorization::NoConflict)?;
        let reservations = Ledger::open(board.repository.path())?
            .read_validated_journal()?
            .into_reservations()?;
        let run = reservations.reservation(first.reservation_id)?.actor().run;
        assert_eq!(
            run,
            reservations.reservation(second.reservation_id)?.actor().run
        );
        let identities = BTreeMap::from([
            (
                "7393c2a2-b68d-4697-85a2-2146a7146aff.phase-1",
                SessionReservationIdentity::new(run, first.reservation_id),
            ),
            (
                "7393c2a2-b68d-4697-85a2-2146a7146aff.phase-2",
                SessionReservationIdentity::new(run, second.reservation_id),
            ),
        ]);
        let context = WorktreeContext::discover(board.repository.path())?;
        fs::write(
            context.ledger_directory().join("session-identities.json"),
            serde_json::to_vec(&serde_json::json!({"identities": identities}))?,
        )?;

        let rendered = render_overview(&board.model()?)?;
        for (claim, path, session) in [
            (first, "a.rs", "2146a7146aff.phase-1"),
            (second, "b.rs", "2146a7146aff.phase-2"),
        ] {
            let row = rendered
                .lines()
                .find(|line| line.contains(path))
                .ok_or_else(|| io::Error::other("Overview should show each phase's path"))?;
            let id = claim.reservation_id.to_string();
            let cells = row.trim_matches('│').split_whitespace().collect::<Vec<_>>();
            assert_eq!(cells[0], id.rsplit('-').next().unwrap_or_default(), "{row}");
            assert_eq!(cells[1], session, "{row}");
        }
        Ok(())
    }

    #[test]
    fn overview_phase_session_and_alias_leave_race_paths_visible_at_100_columns()
    -> FixtureResult<()> {
        let board = BoardFixture::new()?;
        let actor = board.main_actor();
        let claim = board.claim(&actor, "src/phase.rs", ConflictAuthorization::NoConflict)?;
        let reservations = Ledger::open(board.repository.path())?
            .read_validated_journal()?
            .into_reservations()?;
        let run = reservations.reservation(claim.reservation_id)?.actor().run;
        let identities = BTreeMap::from([
            (
                "7393c2a2-b68d-4697-85a2-2146a7146aff.phase-9",
                SessionReservationIdentity::new(run, claim.reservation_id),
            ),
            (
                "8393c2a2-b68d-4697-85a2-3146a7146aff.phase-9",
                SessionReservationIdentity::new(run, claim.reservation_id),
            ),
        ]);
        let context = WorktreeContext::discover(board.repository.path())?;
        fs::write(
            context.ledger_directory().join("session-identities.json"),
            serde_json::to_vec(&serde_json::json!({"identities": identities}))?,
        )?;

        let rendered = render_overview(&board.model()?)?;
        let row = rendered
            .lines()
            .find(|line| line.contains("2146a7146aff.phase-9"))
            .ok_or_else(|| io::Error::other("Overview should show the compact phase session"))?;
        assert!(row.contains("3146a7146aff.phase-9"), "{row}");
        assert!(row.contains("src/phase.rs"), "{row}");
        assert!(row.ends_with('│'), "{row}");
        Ok(())
    }

    #[test]
    fn overview_renders_empty_table_after_terminal_reservation() -> FixtureResult<()> {
        let board = BoardFixture::new()?;
        let actor = board.main_actor();
        let claim = board.claim(&actor, "retired.rs", ConflictAuthorization::NoConflict)?;
        board.release(
            &actor,
            claim.reservation_id,
            ReleaseDisposition::Abandoned("work was dropped".parse()?),
        )?;
        let rendered = render_overview(&board.model()?)?;

        assert!(rendered.contains("Race paths"), "{rendered}");
        assert!(rendered.contains("no reservations"), "{rendered}");
        assert!(!rendered.contains("retired.rs"), "{rendered}");
        Ok(())
    }

    #[test]
    fn overview_lists_waiting_reservations_once_despite_multiple_holds() -> FixtureResult<()> {
        let mut fixture = test_support::answered_board(OverlapAnswerFixture::Defer)?;
        let held = fixture.model.waiting.entries[0].clone();
        fixture.model.waiting.entries.push(held);
        let rendered = render_overview(&fixture.model)?;

        for reservation_id in [fixture.blocker_id, fixture.requester_id] {
            let id = reservation_id.to_string();
            let short_id = id.rsplit('-').next().unwrap_or_default();
            assert_eq!(rendered.matches(short_id).count(), 1, "{rendered}");
        }
        Ok(())
    }

    fn reassembled_model_value(documents: &BoardPaneDocuments) -> Value {
        documents
            .reassembled_model_value()
            .unwrap_or_else(|error| panic!("pane documents should remain JSON objects: {error}"))
    }

    #[test]
    fn every_model_fact_is_reachable_through_exact_pane_documents() {
        let model = model(&[MARKER_ID]);
        let mut expected = serde_json::to_value(&model)
            .unwrap_or_else(|error| panic!("BoardModel should serialize: {error}"));
        let expected_fields = expected
            .as_object_mut()
            .unwrap_or_else(|| panic!("board should be an object"));
        for field in EXPECTED_OMITTED_FIELDS {
            expected_fields.remove(*field);
        }
        for value in expected_fields.values_mut() {
            if let Value::Object(section) = value {
                section.remove("journal_position");
            }
        }
        let documents = BoardPaneDocuments::from_model(&model)
            .unwrap_or_else(|error| panic!("model should project: {error}"));

        assert_eq!(reassembled_model_value(&documents), expected);
    }

    #[test]
    fn pane_documents_omit_raw_diagnostics_and_nested_positions() -> FixtureResult<()> {
        let documents = BoardPaneDocuments::from_model(&model(&[MARKER_ID]))?;
        for pane in BoardPaneId::ALL {
            for field in EXPECTED_OMITTED_FIELDS {
                assert!(
                    !documents.get(pane).text.contains(field),
                    "{pane:?}: {field}"
                );
            }
        }
        assert_eq!(
            documents.overview.text,
            overview_reservation_table(&model(&[]))
        );
        Ok(())
    }

    #[test]
    fn stale_alert_renders_renew_with_the_alerts_reservation_id() {
        let model = model(&[]);
        let documents = BoardPaneDocuments::from_model(&model)
            .unwrap_or_else(|error| panic!("model should project: {error}"));
        let model = reassembled_model_value(&documents);
        let alert = &model["alerts"]["entries"][0];

        assert_eq!(alert["resolution"]["action"], "renew");
        assert_eq!(
            alert["resolution"]["reservation_id"],
            alert["reservation_id"]
        );
    }

    #[test]
    fn keymap_cycles_registered_panes_and_dispatches_quit() {
        let model = model(&[]);
        let mut application = BoardApplication::new(&model)
            .unwrap_or_else(|error| panic!("application should build: {error}"));

        handle_key(
            &mut application,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert_eq!(
            application.framework.focused(),
            &FocusedPane::App(BoardPaneId::Reservations)
        );

        handle_key(
            &mut application,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(application.framework.quit_requested());
    }

    #[test]
    fn first_frame_failure_preserves_the_board_for_headless_reporting() {
        let model = model(&[MARKER_ID]);
        let mut application = BoardApplication::new(&model)
            .unwrap_or_else(|error| panic!("application should build: {error}"));
        let backend = CrosstermBackend::new(FirstFrameWriteFailure);
        let viewport = Viewport::Fixed(Rect::new(0, 0, 100, 30));
        let mut terminal = Terminal::with_options(backend, TerminalOptions { viewport })
            .unwrap_or_else(|error| panic!("fixed test terminal should build: {error}"));

        let mut source = ScriptedModelSource::new(model);
        let mut events = ScriptedEvents::idle_then_quit();
        let interaction = event_loop(&mut terminal, &mut application, &mut source, &mut events);
        let failure = finish_terminal_view(interaction, Ok(()));

        assert!(matches!(
            failure,
            Err(BoardTerminalViewRunFailure::BeforeOpening(
                BoardTerminalViewOpeningFailure::FirstFramePresentation(_)
            ))
        ));
    }

    #[test]
    fn setup_failure_reports_every_failed_terminal_rollback() {
        let rollback = TerminalSetupRollbackOutcome::after_terminal_acquisition(
            Err(io::Error::other("alternate screen rollback failed")),
            Err(io::Error::other("raw mode rollback failed")),
        );
        let failure = BoardTerminalSetupFailure::Construction {
            construction: io::Error::other("terminal construction failed"),
            rollback,
        };

        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("alternate screen rollback failed"));
        assert!(diagnostic.contains("raw mode rollback failed"));
        assert!(diagnostic.contains("the terminal may need to be reset"));
    }

    #[test]
    fn document_navigation_moves_and_clamps_the_scroll_offset() {
        let document = BoardPaneDocument {
            text:              String::new(),
            line_count:        10,
            widest_line_width: 0,
        };
        let mut viewport = BoardPaneViewport::new();
        viewport.observe_layout(&document, Rect::new(0, 0, 20, 4));

        viewport.scroll_down();
        assert_eq!(viewport.vertical_offset, 1);

        viewport.scroll_page_down();
        assert_eq!(viewport.vertical_offset, 4);

        viewport.scroll_end();
        assert_eq!(viewport.vertical_offset, 6);
        viewport.scroll_down();
        assert_eq!(viewport.vertical_offset, 6);

        viewport.scroll_home();
        assert_eq!(viewport.vertical_offset, 0);
    }

    #[test]
    fn widest_line_uses_terminal_cell_width() {
        let mut fields = Map::new();
        fields.insert("path".to_owned(), Value::String("表".to_owned()));

        let document =
            BoardPaneDocument::from_model_fields(fields, HumanTargetVisibility::MultipleTargets)
                .unwrap_or_else(|error| panic!("document should serialize: {error}"));
        let wide_line = "  \"path\": \"表\"";

        assert_eq!(
            document.widest_line_width,
            UnicodeWidthStr::width(wide_line)
        );
        assert!(document.widest_line_width > wide_line.chars().count());
    }

    #[test]
    fn f002_trunk_only_panes_omit_reservation_targets() {
        let mut fields = Map::new();
        fields.insert(
            "targets".to_owned(),
            serde_json::json!([{"ref": "refs/heads/main"}]),
        );
        fields.insert(
            "ready_now".to_owned(),
            serde_json::json!({"entries": [{"reservation": {"target": "main"}}]}),
        );
        fields.insert(
            "unconstrained_reservations".to_owned(),
            serde_json::json!({"entries": [{"target": "main"}]}),
        );
        fields.insert(
            "resolved".to_owned(),
            serde_json::json!({"entries": [{"target": "main"}]}),
        );
        let trunk_only =
            BoardPaneDocument::from_model_fields(fields.clone(), HumanTargetVisibility::TrunkOnly)
                .unwrap_or_else(|error| panic!("trunk-only document should serialize: {error}"));
        assert!(!trunk_only.text.contains("target"), "{}", trunk_only.text);
        let multiple =
            BoardPaneDocument::from_model_fields(fields, HumanTargetVisibility::MultipleTargets)
                .unwrap_or_else(|error| panic!("multi-target document should serialize: {error}"));
        assert!(multiple.text.contains("\"target\": \"main\""));
    }
}
