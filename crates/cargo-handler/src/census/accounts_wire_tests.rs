//! Account permission and read outcomes cross the wire independently.
#![expect(clippy::expect_used, reason = "fixture JSON must parse")]
use serde_json::Value;

use super::accounts::AccountReadStatus;
use super::accounts::AccountRole;
use super::accounts::AccountsReading;
use super::accounts::CreditPermission;
use super::accounts::Reading;
use super::accounts::RoleCountOverflow;
use super::accounts::RoleSessionCounts;
use crate::theme::Role;

fn board_count_report(counts: &Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"accounts":[{
        "label":"board-account","roles":["units"],"role_counts":[counts],
        "weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null
    }],"problems":[]}))
    .expect("board account JSON")
}

#[test]
fn board_role_counts_require_every_state_key() -> Result<(), String> {
    let counts = serde_json::json!({"busy":2,"idle":3,"paused":5,"exited":7,"total":17});
    let mut additional = counts.clone();
    additional["future_count"] = serde_json::json!({"values":[1,2,3]});
    for (label, counts) in [("five keys", counts), ("additional key", additional)] {
        let reading = AccountsReading::parse(&board_count_report(&counts))
            .map_err(|error| format!("{label}: {error:?}"))?;
        let Reading::Known(parsed) = &reading.accounts[0].roles[0].counts else {
            return Err("recorded board counts are unavailable".to_owned());
        };
        assert_eq!((parsed.busy, parsed.idle, parsed.total), (2, 3, 17));
        assert_eq!((parsed.paused, parsed.exited), (5, 7));
        for field in ["busy", "idle", "paused", "exited", "total"] {
            let mut missing = counts.clone();
            missing.as_object_mut().expect("count object").remove(field);
            assert!(
                AccountsReading::parse(&board_count_report(&missing)).is_err(),
                "missing {field} must reject the reading"
            );
        }
        let mut retired = counts;
        retired["shell"] = serde_json::json!(null);
        assert!(
            AccountsReading::parse(&board_count_report(&retired)).is_err(),
            "shell must reject the reading even when its value is null"
        );
    }
    Ok(())
}

#[test]
fn board_role_counts_reject_the_retired_shell_key() {
    let counts = serde_json::json!({"busy":2,"idle":3,"paused":5,"exited":7,"total":17,"shell":11});
    assert!(
        AccountsReading::parse(&board_count_report(&counts)).is_err(),
        "shell is not a board count"
    );
}

#[test]
fn board_role_count_addition_checks_each_state_for_overflow() -> Result<(), String> {
    for field in [
        "busy", "starting", "waiting", "idle", "paused", "exited", "total",
    ] {
        let mut maximum = serde_json::json!({"busy":0,"starting":0,"waiting":0,"idle":0,"paused":0,"exited":0,"total":0});
        maximum[field] = serde_json::json!(u64::MAX);
        let first = AccountsReading::parse(&board_count_report(&maximum)).expect("maximum count");
        let second = AccountsReading::parse(&board_count_report(
            &serde_json::json!({"busy":1,"starting":1,"waiting":1,"idle":1,"paused":1,"exited":1,"total":7}),
        ))
        .expect("added count");
        let (Reading::Known(left), Reading::Known(right)) = (
            &first.accounts[0].roles[0].counts,
            &second.accounts[0].roles[0].counts,
        ) else {
            return Err("recorded board counts are unavailable".to_owned());
        };
        assert_eq!(
            left.clone().checked_add(right),
            Err(RoleCountOverflow),
            "overflow in {field}"
        );
    }
    Ok(())
}

#[test]
fn account_wire_keeps_credit_permission_and_read_outcome_independent() {
    for (enabled, read_ok) in [(true, true), (false, false), (true, false), (false, true)] {
        let bytes=serde_json::to_vec(&serde_json::json!({"accounts":[{
            "label":"wire-account","roles":[],"weekly_left_percent":null,"credits":null,"resets_at":null,"read_at":null,
            "use_credits":enabled,"read_ok":read_ok
        }],"problems":[]})).expect("JSON");
        let reading = AccountsReading::parse(&bytes).expect("wire");
        let row = &reading.accounts[0];
        assert_eq!(
            row.credit_permission,
            if enabled {
                CreditPermission::Enabled
            } else {
                CreditPermission::Disabled
            }
        );
        assert_eq!(
            row.read_status,
            if read_ok {
                AccountReadStatus::Ok
            } else {
                AccountReadStatus::Failed
            }
        );
    }
}

#[test]
fn legacy_account_wire_defaults_missing_read_outcome_without_granting_them() {
    let reading=AccountsReading::parse(br#"{"accounts":[{"label":"legacy","roles":[],"weekly_left_percent":50,"credits":2,"resets_at":null,"read_at":"2026-10-17T00:00:00Z","use_credits":true}],"problems":[]}"#).expect("legacy wire");
    let row = &reading.accounts[0];
    assert_eq!(row.credit_permission, CreditPermission::Enabled);
    assert_eq!(
        row.read_status,
        AccountReadStatus::Failed,
        "stored reading alone does not prove latest read worked"
    );
}

#[test]
fn account_role_counts_use_the_combined_source_without_adding_machine_records() {
    let reading = AccountsReading::parse(
        br#"{"accounts":[{
        "label":"claude1","roles":["units","workers"],
        "role_counts":[{"busy":6,"idle":4,"paused":3,"exited":0,"total":13},null],
        "role_counts_by_machine":{
            "natedev":{"units":{"busy":2,"idle":3,"paused":1,"exited":0,"total":6}},
            "mac":{"units":{"busy":4,"idle":1,"paused":2,"exited":0,"total":7}}
        },
        "weekly_left_percent":50,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null
    }],"problems":[]}"#,
    )
    .expect("machine counts");
    let roles = &reading.accounts[0].roles;
    assert_eq!(roles[0].role, AccountRole::Units);
    assert_eq!(
        roles[0].counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     6,
            paused:   3,
            exited:   0,
            idle:     4,
            total:    13,
        })
    );
    assert_eq!(roles[1].counts, Reading::Unavailable);
}

#[test]
fn account_role_counts_keep_zero_states_and_source_total() {
    let reading = AccountsReading::parse(
        br#"{"accounts":[{
        "label":"codex2","roles":["workers"],
        "role_counts":[{"busy":2,"idle":3,"paused":0,"exited":0,"total":8}],
        "weekly_left_percent":50,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null
    }],"problems":[]}"#,
    )
    .expect("Codex counts");
    assert_eq!(
        reading.accounts[0].roles[0].counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     2,
            paused:   0,
            exited:   0,
            idle:     3,
            total:    8,
        })
    );
}

#[test]
fn combined_role_counts_preserve_the_source_paused_count() {
    let reading = AccountsReading::parse(
        br#"{"accounts":[{
        "label":"mixed","roles":["workers"],
        "role_counts":[{"busy":5,"idle":8,"paused":5,"exited":0,"total":18}],
        "role_counts_by_machine":{
            "mac":{"workers":{"busy":1,"idle":2,"paused":0,"exited":0,"total":3}},
            "natedev":{"workers":{"busy":4,"idle":6,"paused":5,"exited":0,"total":15}}
        },"weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null}],"problems":[]}"#,
    )
    .expect("machine counts");
    assert_eq!(
        reading.accounts[0].roles[0].counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     5,
            paused:   5,
            exited:   0,
            idle:     8,
            total:    18,
        })
    );
}

#[test]
fn machine_role_counts_cannot_overflow_the_combined_source() {
    let bytes = serde_json::to_vec(&serde_json::json!({"accounts":[{
        "label":"overflow","roles":["workers"],
        "role_counts":[{"busy":0,"idle":0,"paused":0,"exited":0,"total":0}],
        "role_counts_by_machine":{
            "mac":{"workers":{"busy":u64::MAX,"idle":0,"paused":0,"exited":0,"total":0}},
            "natedev":{"workers":{"busy":1,"idle":0,"paused":0,"exited":0,"total":0}}
        },"weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null}],"problems":[]}))
    .expect("JSON");
    assert_eq!(
        AccountsReading::parse(&bytes)
            .expect("combined source")
            .accounts[0]
            .roles[0]
            .counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     0,
            paused:   0,
            exited:   0,
            idle:     0,
            total:    0,
        })
    );
}

#[test]
fn combined_role_counts_keep_claude_codex_and_spare_entries_in_source_order() {
    let reading = AccountsReading::parse(br#"{"accounts":[{
        "label":"combined","roles":["units","workers","spare"],
        "role_counts":[
            {"busy":6,"idle":4,"paused":3,"exited":0,"total":13},
            {"busy":2,"idle":5,"paused":0,"exited":0,"total":7},null
        ],
        "role_counts_by_machine":{"other":{"units":{"busy":999,"idle":999,"paused":999,"exited":0,"total":999}}},
        "weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null}],"problems":[]}"#).expect("combined counts");
    let roles = &reading.accounts[0].roles;
    assert_eq!(roles.len(), 3);
    assert_eq!(roles[0].role, AccountRole::Units);
    assert_eq!(
        roles[0].counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     6,
            paused:   3,
            exited:   0,
            idle:     4,
            total:    13,
        })
    );
    assert_eq!(roles[1].role, AccountRole::Workers);
    assert_eq!(
        roles[1].counts,
        Reading::Known(RoleSessionCounts {
            starting: 0,
            waiting:  0,
            busy:     2,
            paused:   0,
            exited:   0,
            idle:     5,
            total:    7,
        })
    );
    assert_eq!(roles[2].role, AccountRole::Spare);
    assert_eq!(roles[2].counts, Reading::Unavailable);
}

#[test]
fn stored_accounts_parse_starting_and_waiting_sessions_with_their_role_counts() {
    let reading = AccountsReading::parse(br#"{
        "accounts":[{
            "label":"fixture-account","tool":"codex","roles":["workers"],
            "selected_roles":["workers"],
            "role_counts":[{"busy":1,"starting":2,"waiting":3,"idle":4,"paused":5,"exited":6,"total":21}],
            "sessions":[
                {"session":"fake-starting","state":"starting"},
                {"session":"fake-waiting","state":"waiting"}
            ],
            "weekly_left_percent":50,"resets_at":null,"credits":"unlimited",
            "use_credits":true,"read_at":"2026-11-13T03:16:39+00:00","read_ok":true
        }],"problems":[]
    }"#).expect("stored account report");
    for (session, text, role) in [
        ("fake-starting", "starting", Role::Busy),
        ("fake-waiting", "waiting", Role::Idle),
    ] {
        let state = reading.sessions.get(session);
        assert_eq!(state.text(), text);
        assert_eq!(state.color_role(), role);
    }
    assert_eq!(
        reading.accounts[0].roles[0].counts,
        Reading::Known(RoleSessionCounts {
            busy:     1,
            starting: 2,
            waiting:  3,
            idle:     4,
            paused:   5,
            exited:   6,
            total:    21,
        })
    );
}
