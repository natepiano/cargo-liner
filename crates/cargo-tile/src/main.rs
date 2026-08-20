//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane` framework.

mod constants;

use std::io;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;
use tui_pane::AppContext;
use tui_pane::FRAME_POLL_MILLIS;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkFocusId;
use tui_pane::NoToastAction;

use crate::constants::QUIT_KEY;

/// The panes `cargo-tile` supplies to [`Framework`]; one variant per
/// app-side pane.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum AppPaneId {
    /// The tile grid, still a placeholder body.
    Tiles,
}

/// Whether the event loop in [`run`] draws another frame.
enum Flow {
    /// Draw the next frame.
    Continue,
    /// Leave the loop and restore the terminal.
    Quit,
}

/// Top-level app state the framework borrows itself back through, per
/// [`AppContext`].
struct App {
    framework: Framework<Self>,
}

impl App {
    /// Draw the placeholder body into `frame`, titled with the pane
    /// [`Framework::focused`] reports.
    fn render(&self, frame: &mut Frame) {
        let focus = match *self.framework().focused() {
            FocusedPane::App(AppPaneId::Tiles) => "tiles",
            FocusedPane::Framework(FrameworkFocusId::Toasts) => "toasts",
        };
        let block = Block::bordered().title(format!(" cargo-tile — {focus} "));
        let body = Paragraph::new(format!("press {QUIT_KEY} to quit")).block(block);
        frame.render_widget(body, frame.area());
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            framework: Framework::new(FocusedPane::App(AppPaneId::Tiles)),
        }
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

fn main() -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal);
    ratatui::try_restore().and(result)
}

/// Draw and poll until [`Flow::Quit`].
fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let app = App::default();
    loop {
        terminal.draw(|frame| app.render(frame))?;
        match next_flow()? {
            Flow::Continue => (),
            Flow::Quit => return Ok(()),
        }
    }
}

/// Wait one frame poll interval for an input event and classify it.
fn next_flow() -> io::Result<Flow> {
    if !event::poll(Duration::from_millis(FRAME_POLL_MILLIS))? {
        return Ok(Flow::Continue);
    }
    let flow = match event::read()? {
        Event::Key(key)
            if key.kind == KeyEventKind::Press && key.code == KeyCode::Char(QUIT_KEY) =>
        {
            Flow::Quit
        },
        _ => Flow::Continue,
    };
    Ok(flow)
}
