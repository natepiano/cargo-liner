//! Active phase claims for the real release-command fixture.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;

use cargo_berth_test_support::reservation_row;
use serde_json::Value;
use serde_json::json;

/// Append an active `JournalOperation::Claim` beside an existing phase in the fixture journal.
pub(super) fn append_active_phase_claim(
    journal_path: &Path,
    existing_id: &str,
    scope: &str,
) -> Result<String, Box<dyn Error>> {
    let events = fs::read_to_string(journal_path)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let mut claim = events
        .iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == existing_id)
        .ok_or("the existing phase claim must be present")?
        .clone();
    let reservation_id = uuid::Uuid::now_v7().to_string();
    claim["event_id"] = json!(uuid::Uuid::now_v7());
    claim["reservation_id"] = json!(reservation_id);
    claim["projection_generation"] = json!(
        events.last().ok_or("the journal must have a generation")?["projection_generation"]
            .as_u64()
            .ok_or("the generation must be a number")?
            + 1
    );
    claim["scopes"] = json!([{"kind":"file", "path":scope}]);
    let mut journal = fs::OpenOptions::new().append(true).open(journal_path)?;
    writeln!(journal, "{claim}")?;
    Ok(reservation_id)
}

/// Check each `reservation_row` lifecycle and scope against the supplied phase rows.
pub(super) fn assert_phase_claims(board: &Value, phases: &[(&str, &str, &str)]) {
    for &(id, lifecycle, path) in phases {
        let sibling = reservation_row(board, id);
        assert_eq!(sibling["lifecycle"]["stage"], lifecycle, "{board}");
        assert_eq!(sibling["scopes"], json!([{"kind":"file", "path":path}]));
    }
}
