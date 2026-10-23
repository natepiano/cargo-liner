//! A selected remote seat keeps its identity through the real probe reader.

#![expect(clippy::expect_used, reason = "fixture rows and terminal must exist")]
#![expect(clippy::panic, reason = "missing rendered rows fail the regression")]

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::render;

fn paint(app: &mut App) -> Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(180, 48)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("settled frame");
    terminal.backend().buffer().clone()
}

fn locate(buffer: &Buffer, label: &str) -> Position {
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(index) = line.find(label) {
            return Position::new(
                u16::try_from(line[..index].chars().count()).expect("column"),
                y,
            );
        }
    }
    panic!("missing rendered {label}");
}

fn report(thread: &str, job: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote-fixture","rows":[{
            "agent":"claude","pid":10,"name":"remote-owner","session_id":"remote-owner",
            "started":100,"directory":"/fixture","children":[
                {"kind":"shell","depth":0,"pid":701,"name":"delivery","started":200,
                 "dispatch":{"worker":{"seat":"fixture-impl7","short_seat":"impl7","thread_id":thread,"job":job}}},
                {"kind":"thread","depth":0,"thread_id":thread,"name":"original-worker","started":200}
            ]
        }]
    })).expect("probe bytes")
}

#[test]
fn a_selected_remote_seat_keeps_its_row_after_a_probe_round_trip() {
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.machines.remote = vec!["remote-fixture".into()];
    let (rows, edges) =
        probe::parse_report(&report("thread-one", "first remote task")).expect("initial probe");
    app.census.apply(CensusUpdate::Remote {
        host:  "remote-fixture".into(),
        state: MachineState::Answered(rows, edges.into()),
    });
    let before = paint(&mut app);
    app.click(locate(&before, "impl7 · first remote task"));
    let selected = paint(&mut app);
    let position = locate(&selected, "impl7 · first remote task");
    assert_eq!(
        selected[(position.x, position.y)].bg,
        tui_pane::active_focus_color()
    );

    let (rows, edges) =
        probe::parse_report(&report("thread-one", "updated remote task")).expect("next probe");
    // Serialize the actual domain rows and read them through the same wire boundary again.
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote-fixture","rows":rows,"edges":edges,
    }))
    .expect("returned probe");
    let (rows, edges) = probe::parse_report(&bytes).expect("round-trip probe");
    app.census.apply(CensusUpdate::Remote {
        host:  "remote-fixture".into(),
        state: MachineState::Answered(rows, edges.into()),
    });
    let after = paint(&mut app);
    let position = locate(&after, "impl7 · updated remote task");
    assert_eq!(
        after[(position.x, position.y)].bg,
        tui_pane::active_focus_color()
    );
}
