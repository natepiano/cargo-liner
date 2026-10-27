//! Account permission, spending and read outcomes cross the wire independently.
#![expect(clippy::expect_used, reason = "fixture JSON must parse")]
use super::accounts::AccountReadStatus;
use super::accounts::AccountsReading;
use super::accounts::CreditActivity;
use super::accounts::CreditPermission;

#[test]
fn account_wire_keeps_credit_permission_activity_and_read_outcome_independent() {
    for (enabled, in_use, read_ok) in [
        (true, false, true),
        (false, true, false),
        (true, true, false),
        (false, false, true),
    ] {
        let bytes=serde_json::to_vec(&serde_json::json!({"accounts":[{
            "label":"wire-account","roles":[],"weekly_left_percent":null,"credits":null,"resets_at":null,"read_at":null,
            "use_credits":enabled,"credits_in_use":in_use,"read_ok":read_ok
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
            row.credit_activity,
            if in_use {
                CreditActivity::InUse
            } else {
                CreditActivity::Idle
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
fn legacy_account_wire_defaults_missing_read_and_spending_fields_without_granting_them() {
    let reading=AccountsReading::parse(br#"{"accounts":[{"label":"legacy","roles":[],"weekly_left_percent":50,"credits":2,"resets_at":null,"read_at":"2026-10-17T00:00:00Z","use_credits":true}],"problems":[]}"#).expect("legacy wire");
    let row = &reading.accounts[0];
    assert_eq!(row.credit_permission, CreditPermission::Enabled);
    assert_eq!(
        row.credit_activity,
        CreditActivity::Idle,
        "permission is not spending"
    );
    assert_eq!(
        row.read_status,
        AccountReadStatus::Failed,
        "stored reading alone does not prove latest read worked"
    );
}
