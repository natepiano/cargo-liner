//! Terminal rendering of the headless [`BoardModel`] projection.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::io::IsTerminal;
use std::io::Stdout;
use std::rc::Rc;

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

const FOOTER_HEIGHT: u16 = 1;
const HORIZONTAL_SCROLL_STEP: u16 = 4;
const OVERVIEW_FIELDS: &[&str] = &[
    "journal_position",
    "recovered_bypasses_this_invocation",
    "integration_order",
    "git_cost",
];
const RESERVATION_FIELDS: &[&str] = &["ready_now", "unconstrained_reservations", "resolved"];
const CONSTRAINT_FIELDS: &[&str] = &[
    "waiting",
    "settled_ordering_constraints",
    "unresolved_overlaps",
];
const ANSWER_FIELDS: &[&str] = &[
    "recorded_overlap_answers",
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
pub(crate) fn run(model: &BoardModel) -> Result<(), BoardTerminalViewRunFailure> {
    let mut application =
        BoardApplication::new(model).map_err(BoardTerminalViewRunFailure::BeforeOpening)?;
    let mut terminal = setup_terminal().map_err(|error| {
        BoardTerminalViewRunFailure::BeforeOpening(BoardTerminalViewOpeningFailure::TerminalSetup(
            error,
        ))
    })?;
    let interaction = event_loop(&mut terminal, &mut application);
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
            Self::Overview => OVERVIEW_FIELDS,
            Self::Reservations => RESERVATION_FIELDS,
            Self::Constraints => CONSTRAINT_FIELDS,
            Self::Answers => ANSWER_FIELDS,
            Self::Incursions => INCURSION_FIELDS,
            Self::Alerts => ALERT_FIELDS,
        }
    }
}

/// Exact JSON facts assigned to one terminal pane.
struct BoardPaneDocument {
    text:              String,
    line_count:        usize,
    widest_line_width: usize,
}

impl BoardPaneDocument {
    fn from_model_fields(
        fields: Map<String, Value>,
    ) -> Result<Self, BoardTerminalViewOpeningFailure> {
        let text = serde_json::to_string_pretty(&Value::Object(fields))
            .map_err(BoardTerminalViewOpeningFailure::ModelSerialization)?;
        let line_count = text.lines().count();
        let widest_line_width = text
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or_default();
        Ok(Self {
            text,
            line_count,
            widest_line_width,
        })
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
        let model = serde_json::to_value(model)
            .map_err(BoardTerminalViewOpeningFailure::ModelSerialization)?;
        Self::from_model_value(model)
    }

    fn from_model_value(model: Value) -> Result<Self, BoardTerminalViewOpeningFailure> {
        let Value::Object(mut unassigned) = model else {
            return Err(BoardTerminalViewOpeningFailure::SerializedModelWasNotObject);
        };
        let overview = Self::take_document(&mut unassigned, BoardPaneId::Overview)?;
        let reservations = Self::take_document(&mut unassigned, BoardPaneId::Reservations)?;
        let constraints = Self::take_document(&mut unassigned, BoardPaneId::Constraints)?;
        let answers = Self::take_document(&mut unassigned, BoardPaneId::Answers)?;
        let incursions = Self::take_document(&mut unassigned, BoardPaneId::Incursions)?;
        let alerts = Self::take_document(&mut unassigned, BoardPaneId::Alerts)?;
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
    ) -> Result<BoardPaneDocument, BoardTerminalViewOpeningFailure> {
        let mut fields = Map::new();
        for field in pane_id.model_fields() {
            let Some(value) = unassigned.remove(*field) else {
                return Err(BoardTerminalViewOpeningFailure::MissingModelField(field));
            };
            fields.insert((*field).to_owned(), value);
        }
        BoardPaneDocument::from_model_fields(fields)
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
            let fields: Map<String, Value> = serde_json::from_str(&self.get(pane_id).text)?;
            model.extend(fields);
        }
        Ok(Value::Object(model))
    }
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

/// Application state borrowed back through [`AppContext`].
struct BoardApplication {
    framework: Framework<Self>,
    keymap:    Rc<Keymap<Self>>,
    documents: BoardPaneDocuments,
    viewports: BoardPaneViewports,
}

impl BoardApplication {
    fn new(model: &BoardModel) -> Result<Self, BoardTerminalViewOpeningFailure> {
        Self::from_documents(BoardPaneDocuments::from_model(model)?)
    }

    fn from_documents(
        documents: BoardPaneDocuments,
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
        })
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

fn event_loop(
    terminal: &mut Terminal<impl Backend<Error = io::Error>>,
    application: &mut BoardApplication,
) -> BoardTerminalViewInteractionOutcome {
    if let Err(error) = terminal.draw(|frame| draw(frame, application)) {
        return BoardTerminalViewInteractionOutcome::FailedBeforeFirstFrame(error);
    }

    while !application.framework.quit_requested() {
        let event = match event::read() {
            Ok(event) => event,
            Err(error) => {
                return BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(error);
            },
        };
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(application, key),
            _ => {},
        }
        if !application.framework.quit_requested()
            && let Err(error) = terminal.draw(|frame| draw(frame, application))
        {
            return BoardTerminalViewInteractionOutcome::FailedAfterFirstFrame(error);
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
    use std::io::Write;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::TerminalOptions;
    use ratatui::Viewport;

    use super::*;

    const MARKER_ID: &str = "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";

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
            "ready_now": empty_section(),
            "waiting": empty_section(),
            "settled_ordering_constraints": empty_section(),
            "unresolved_overlaps": empty_section(),
            "recorded_overlap_answers": empty_section(),
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

    fn reassembled_model_value(documents: &BoardPaneDocuments) -> Value {
        documents
            .reassembled_model_value()
            .unwrap_or_else(|error| panic!("pane documents should remain JSON objects: {error}"))
    }

    #[test]
    fn every_model_fact_is_reachable_through_exact_pane_documents() {
        let model = model(&[MARKER_ID]);
        let expected = serde_json::to_value(&model)
            .unwrap_or_else(|error| panic!("BoardModel should serialize: {error}"));
        let documents = BoardPaneDocuments::from_model(&model)
            .unwrap_or_else(|error| panic!("model should project: {error}"));

        assert_eq!(reassembled_model_value(&documents), expected);
    }

    #[test]
    fn recovered_bypasses_render_only_in_the_model_that_reports_them() {
        let reporting_model = model(&[MARKER_ID]);
        let later_model = model(&[]);
        let reporting = BoardPaneDocuments::from_model(&reporting_model)
            .unwrap_or_else(|error| panic!("reporting model should project: {error}"));
        let later = BoardPaneDocuments::from_model(&later_model)
            .unwrap_or_else(|error| panic!("later model should project: {error}"));

        assert_eq!(
            reassembled_model_value(&reporting)["recovered_bypasses_this_invocation"],
            serde_json::json!([MARKER_ID])
        );
        assert_eq!(
            reassembled_model_value(&later)["recovered_bypasses_this_invocation"],
            serde_json::json!([])
        );
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

        let interaction = event_loop(&mut terminal, &mut application);
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

        let document = BoardPaneDocument::from_model_fields(fields)
            .unwrap_or_else(|error| panic!("document should serialize: {error}"));
        let wide_line = "  \"path\": \"表\"";

        assert_eq!(
            document.widest_line_width,
            UnicodeWidthStr::width(wide_line)
        );
        assert!(document.widest_line_width > wide_line.chars().count());
    }
}
