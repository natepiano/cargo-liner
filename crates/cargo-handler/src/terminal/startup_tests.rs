//! Startup drawing while the census is still reading its first answer.

#![expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tui_pane::Repaint;
use tui_pane::TerminalApp;

use super::Ticker;
use crate::app::App;
use crate::census::Census;
use crate::census::CensusUpdate;
use crate::census::schedule;

#[test]
fn the_first_frame_draws_the_scanning_summary_before_a_slow_source_answers() {
    let mut app = App::new_for_test().expect("the test app should build");
    app.census = Census::new("natedev".to_string());
    let (release_source, source_released) = mpsc::channel();
    let (source_started, source_entered) = mpsc::channel();
    let (source_answered, answer_sent) = mpsc::channel();
    let updates = schedule::spawn_with_test_source(move |sender| {
        if source_started.send(()).is_err() || source_released.recv().is_err() {
            return;
        }
        if sender.send(CensusUpdate::Local(Vec::new())).is_ok() {
            let _ = source_answered.send(());
        }
    });
    source_entered
        .recv_timeout(Duration::from_millis(500))
        .expect("the census source should start and wait for its release");
    let (_appearance_sender, appearances) = mpsc::channel();
    let mut ticker = Ticker::new(updates, appearances);
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("the test terminal should open");

    ticker.poll_at(&mut app, Instant::now(), 100);
    app.before_draw();
    let keymap = app.keymap();
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("the first frame should draw while the census source waits");

    let buffer = terminal.backend().buffer();
    let top: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
    let heading: String = (0..80).map(|x| buffer[(x, 1)].symbol()).collect();
    assert!(top.starts_with("┌ summary"), "the summary frame is drawn");
    assert!(
        heading.contains("natedev · scanning"),
        "the first frame shows the machine awaiting its census answer: {heading}"
    );
    assert_eq!(buffer[(0, 22)].symbol(), "└");
    assert_eq!(buffer[(79, 22)].symbol(), "┘");

    release_source
        .send(())
        .expect("the census source should still be waiting");
    answer_sent
        .recv_timeout(Duration::from_millis(500))
        .expect("the census source should answer after release");
    assert_eq!(
        ticker.poll_at(&mut app, Instant::now(), 100),
        Repaint::Needed
    );
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("the census answer should be drawn on the next frame");
    let buffer = terminal.backend().buffer();
    let heading: String = (0..80).map(|x| buffer[(x, 1)].symbol()).collect();
    assert!(
        heading.contains("natedev · no agents"),
        "the later frame replaces the scanning heading with the census answer: {heading}"
    );
}
