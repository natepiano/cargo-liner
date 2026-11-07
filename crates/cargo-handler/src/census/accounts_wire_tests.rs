//! Account permission and read outcomes cross the wire independently.
#![expect(clippy::expect_used, reason = "fixture JSON must parse")]
use super::accounts::AccountReadStatus;
use super::accounts::AccountRole;
use super::accounts::AccountsReading;
use super::accounts::CreditPermission;
use super::accounts::Reading;
use super::accounts::RoleSessionCounts;

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
        "role_counts":[{"busy":6,"shell":3,"idle":4,"total":13},null],
        "role_counts_by_machine":{
            "natedev":{"units":{"busy":2,"shell":1,"idle":3,"total":6}},
            "mac":{"units":{"busy":4,"shell":2,"idle":1,"total":7}}
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
            busy:  6,
            shell: Reading::Known(3),
            idle:  4,
            total: 13,
        })
    );
    assert_eq!(roles[1].counts, Reading::Unavailable);
}

#[test]
fn account_role_counts_keep_unknown_shell_and_source_total() {
    let reading = AccountsReading::parse(
        br#"{"accounts":[{
        "label":"codex2","roles":["workers"],
        "role_counts":[{"busy":2,"shell":null,"idle":3,"total":8}],
        "weekly_left_percent":50,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null
    }],"problems":[]}"#,
    )
    .expect("Codex counts");
    assert_eq!(
        reading.accounts[0].roles[0].counts,
        Reading::Known(RoleSessionCounts {
            busy:  2,
            shell: Reading::Unavailable,
            idle:  3,
            total: 8,
        })
    );
}

#[test]
fn combined_role_counts_preserve_the_source_shell_count() {
    let reading = AccountsReading::parse(
        br#"{"accounts":[{
        "label":"mixed","roles":["workers"],
        "role_counts":[{"busy":5,"shell":5,"idle":8,"total":18}],
        "role_counts_by_machine":{
            "mac":{"workers":{"busy":1,"shell":null,"idle":2,"total":3}},
            "natedev":{"workers":{"busy":4,"shell":5,"idle":6,"total":15}}
        },"weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null}],"problems":[]}"#,
    )
    .expect("machine counts");
    assert_eq!(
        reading.accounts[0].roles[0].counts,
        Reading::Known(RoleSessionCounts {
            busy:  5,
            shell: Reading::Known(5),
            idle:  8,
            total: 18,
        })
    );
}

#[test]
fn machine_role_counts_cannot_overflow_the_combined_source() {
    let bytes = serde_json::to_vec(&serde_json::json!({"accounts":[{
        "label":"overflow","roles":["workers"],
        "role_counts":[{"busy":0,"shell":null,"idle":0,"total":0}],
        "role_counts_by_machine":{
            "mac":{"workers":{"busy":u64::MAX,"shell":null,"idle":0,"total":0}},
            "natedev":{"workers":{"busy":1,"shell":null,"idle":0,"total":0}}
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
            busy:  0,
            shell: Reading::Unavailable,
            idle:  0,
            total: 0,
        })
    );
}

#[test]
fn combined_role_counts_keep_claude_codex_and_spare_entries_in_source_order() {
    let reading = AccountsReading::parse(br#"{"accounts":[{
        "label":"combined","roles":["units","workers","spare"],
        "role_counts":[
            {"busy":6,"shell":3,"idle":4,"total":13},
            {"busy":2,"shell":null,"idle":5,"total":7},null
        ],
        "role_counts_by_machine":{"other":{"units":{"busy":999,"shell":999,"idle":999,"total":999}}},
        "weekly_left_percent":null,"resets_at":null,"credits":null,
        "use_credits":false,"read_at":null}],"problems":[]}"#).expect("combined counts");
    let roles = &reading.accounts[0].roles;
    assert_eq!(roles.len(), 3);
    assert_eq!(roles[0].role, AccountRole::Units);
    assert_eq!(
        roles[0].counts,
        Reading::Known(RoleSessionCounts {
            busy:  6,
            shell: Reading::Known(3),
            idle:  4,
            total: 13,
        })
    );
    assert_eq!(roles[1].role, AccountRole::Workers);
    assert_eq!(
        roles[1].counts,
        Reading::Known(RoleSessionCounts {
            busy:  2,
            shell: Reading::Unavailable,
            idle:  5,
            total: 7,
        })
    );
    assert_eq!(roles[2].role, AccountRole::Spare);
    assert_eq!(roles[2].counts, Reading::Unavailable);
}
