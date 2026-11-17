//! The accounts command's session states decode at the command boundary.

#![expect(clippy::expect_used, reason = "the fixture report must decode")]

use super::accounts::SessionStates;
use crate::theme::Role;

#[test]
fn accounts_command_reads_four_status_values_and_old_starting_and_waiting_as_busy() {
    let states = SessionStates::parse(
        br#"{
        "accounts":[{
            "label":"fixture-account","tool":"codex","roles":["workers"],
            "sessions":[
                {"session":"fake-busy","state":"busy"},
                {"session":"fake-idle","state":"idle"},
                {"session":"fake-paused","state":"paused"},
                {"session":"fake-exited","state":"exited"},
                {"session":"fake-starting","state":"starting"},
                {"session":"fake-waiting","state":"waiting"}
            ]
        }],"problems":[]
    }"#,
    )
    .expect("accounts command report");
    for (session, text, role) in [
        ("fake-busy", "busy", Role::Busy),
        ("fake-idle", "idle", Role::Idle),
        ("fake-paused", "paused", Role::Idle),
        ("fake-exited", "exited", Role::Idle),
        ("fake-starting", "busy", Role::Busy),
        ("fake-waiting", "busy", Role::Busy),
    ] {
        let state = states.get(session);
        assert_eq!(state.text(), text, "{session}");
        assert_eq!(state.color_role(), role, "{session}");
    }
}
