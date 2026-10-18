//! Assertions for the single audit record accompanying an overlap refusal.

use serde_json::Value;

/// Check that a refusal appends exactly its holder facts, preserving prior journal bytes.
pub(crate) fn assert_one_refusal_appended(
    journal_before: &[u8],
    journal_after: &[u8],
    verb: &str,
    envelope: &Value,
) -> Value {
    let appended = journal_after
        .strip_prefix(journal_before)
        .expect("refusal preserves the existing journal bytes");
    let appended = std::str::from_utf8(appended).expect("appended record is UTF-8");
    let lines = appended.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        1,
        "one record per returned refusal: {appended}"
    );
    assert!(
        appended.ends_with('\n'),
        "journal record ends with a newline"
    );
    let record: Value = serde_json::from_str(lines[0]).expect("refusal record decodes");
    assert_eq!(record["op"], "overlap_refused");
    assert_eq!(record["verb"], verb);
    let holders = record["holders"].as_array().expect("refusing holders");
    let blocked_by = envelope["blocked_by"]
        .as_array()
        .expect("blocked holder ids");
    let conflicts = envelope["payload"]["data"]["conflicts"]
        .as_array()
        .expect("refusal conflicts");
    assert!(!holders.is_empty(), "a refusal names at least one holder");
    assert_eq!(holders.len(), blocked_by.len());
    assert_eq!(holders.len(), conflicts.len());
    for ((holder, id), conflict) in holders.iter().zip(blocked_by).zip(conflicts) {
        assert_eq!(&holder["reservation_id"], id);
        assert_eq!(holder["reservation_id"], conflict["reservation_id"]);
        assert_eq!(holder["scopes"], conflict["overlapping_scopes"]);
    }
    record
}
