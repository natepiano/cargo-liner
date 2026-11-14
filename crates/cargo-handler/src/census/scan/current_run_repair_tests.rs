//! Current run failures retain reasons without accepting old worker membership.

use super::*;
use crate::census::HierarchySource;
use crate::census::UnknownReason;
use crate::census::hierarchy::StaffFileReader;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// One unit director, `director`, whose current run is `run`, with one worker, `w`.
fn staff(root: &Path, run: &Path) -> Result<Arc<StaffRoster>, Box<dyn std::error::Error>> {
    let path = root.join("staff.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema":2,"rows":[
            {"id":"boss","kind":"showrunner","name":"boss","parent":null,"current_session":"boss"},
            {"id":"unit","kind":"unit","name":"unit","parent":"boss","current_session":"director","run":run},
            {"id":"w","kind":"worker","name":"worker","parent":"unit","current_session":"worker"}
        ]}))?,
    )?;
    Ok(StaffFileReader::default()
        .roster(&HierarchySource::File(path.to_string_lossy().into_owned())))
}

fn observed_start(roster: &StaffRoster, run: &Path, session: &str) -> Observed<u64> {
    let observations = CurrentRunObservations {
        staff:     roster,
        processes: &[],
        records:   &HashMap::new(),
        health:    &HashMap::new(),
        passes:    HashMap::from([(run.to_owned(), worker_dispatch::current_passes(run))]),
    };
    observations.worker(session).activity_started
}

#[test]
fn unreadable_current_passes_never_accept_old_worker_membership() -> TestResult {
    let root = tempfile::tempdir()?;
    let run = root.path().join("run");
    fs::create_dir(&run)?;
    for content in [None, Some("{")] {
        if let Some(content) = content {
            fs::write(run.join(PROGRESS_HISTORY_STATE_FILE), content)?;
        }
        let selected = staff(root.path(), &run)?.for_current_runs();
        assert_eq!(selected.workers("director").count(), 0, "{content:?}");
        assert!(
            matches!(observed_start(&selected, &run, "director"),
                Observed::Unknown(UnknownReason(reason)) if reason.contains(PROGRESS_HISTORY_STATE_FILE)),
            "{content:?}"
        );
    }
    Ok(())
}

#[test]
fn single_pass_source_keeps_its_current_worker() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join(PROGRESS_HISTORY_STATE_FILE),
        serde_json::to_vec(&serde_json::json!({
            "pass":{"instance_id":"p","worker_id":"w","status":"active","started_at":100}
        }))?,
    )?;
    let SourceRead::Loaded(passes) = worker_dispatch::current_passes(root.path()) else {
        return Err("pass source must load".into());
    };
    assert_eq!(
        passes
            .iter()
            .map(|pass| pass.worker.as_str())
            .collect::<Vec<_>>(),
        ["w"]
    );
    let roster = staff(root.path(), root.path())?;
    assert_eq!(roster.for_current_runs().workers("director").count(), 1);
    assert_eq!(
        observed_start(&roster, root.path(), "worker"),
        Observed::Known(100)
    );
    Ok(())
}

#[test]
fn active_pass_precedes_a_completed_pass_for_the_same_worker() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join(PROGRESS_HISTORY_STATE_FILE),
        serde_json::to_vec(&serde_json::json!({"pass":{
            "a":{"worker_id":"w","status":"complete","started_at":200},
            "b":{"worker_id":"w","status":"active","started_at":100}
        }}))?,
    )?;
    let roster = staff(root.path(), root.path())?;
    assert_eq!(
        observed_start(&roster, root.path(), "worker"),
        Observed::Known(100)
    );
    Ok(())
}
