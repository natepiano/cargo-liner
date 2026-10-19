//! Classified agents are published before their detail sources are read.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;

use super::ProcessEntry;
use super::ScanStage;
use super::SessionRecord;
use crate::census::Account;
use crate::census::Agent;
use crate::census::AgentActivity;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::CodexThreadId;
use crate::census::HierarchyEdges;
use crate::census::RoleFallback;
use crate::census::ServiceTier;
use crate::census::SessionIdentity;
use crate::census::account::AccountLabels;
use crate::census::account::ConfigHome;
use crate::census::worker_dispatch::WorkerDispatch;
use crate::census::worker_dispatch::WorkerTask;

/// The shell detail attached after classification is published.
fn staged_child() -> ChildRow {
    ChildRow {
        worker_task:  WorkerTask::Unrecorded,
        thread_id:    CodexThreadId::Unrecorded,
        dispatch:     WorkerDispatch::Plain,
        activity:     AgentActivity::Idle,
        depth:        0,
        kind:         ChildKind::Shell,
        service_tier: ServiceTier::Unrecorded,
        account:      Account::Unrecorded,
        pid:          Some(428_045),
        name:         "build.sh".to_string(),
        started:      110,
        timer:        None,
    }
}

fn staged_agent() -> AgentRow {
    AgentRow {
        role_fallback:   RoleFallback::None,
        session_id:      SessionIdentity::Claude("abcd1234".to_string()),
        agent:           Agent::Claude,
        account:         Account::Home(AccountLabels::default().resolve(
            Path::new("/home/test/.claude1"),
            Some(Path::new("/home/test")),
        )),
        service_tier:    ServiceTier::Unrecorded,
        name:            "handler".to_string(),
        activity:        AgentActivity::from(Some("busy")),
        started:         100,
        process_started: 100,
        pid:             428_044,
        desktop:         None,
        directory:       "~/rust/handler".to_string(),
        branch:          None,
        launched_by:     None,
        children:        Vec::new(),
    }
}

#[test]
fn the_staged_scan_sends_classified_rows_before_reading_details() {
    let processes = [
        ProcessEntry {
            pid:         428_044,
            parent:      Some(1),
            name:        "claude".to_string(),
            arguments:   vec!["claude".to_string()],
            started:     100,
            directory:   Some(PathBuf::from("/home/test/rust/handler")),
            config_home: ConfigHome::Named(PathBuf::from("/home/test/.claude1")),
            claude_pid:  None,
        },
        ProcessEntry {
            pid:         428_045,
            parent:      Some(428_044),
            name:        "sh".to_string(),
            arguments:   vec!["sh".to_string(), "build.sh".to_string()],
            started:     110,
            directory:   Some(PathBuf::from("/home/test/rust/handler")),
            config_home: ConfigHome::Unread,
            claude_pid:  None,
        },
    ];
    let sessions = [SessionRecord {
        pid:        428_044,
        session_id: "abcd1234".to_string(),
        cwd:        Some(PathBuf::from("/home/test/rust/handler")),
        name:       Some("handler".to_string()),
        status:     Some("busy".to_string()),
        entrypoint: None,
    }];
    let classified = staged_agent();
    let child = staged_child();
    let (sender, updates) = mpsc::channel();

    super::LocalScanner::new(AccountLabels::default()).scan_stages(
        &processes,
        &sessions,
        &[],
        Some(Path::new("/home/test")),
        |rows| {
            sender
                .send(CensusUpdate::Local {
                    rows,
                    hierarchy: HierarchyEdges::default().into(),
                    stage: ScanStage::Classified,
                })
                .expect("the census receiver should remain open");
        },
        |_, rows| {
            let first = updates
                .try_recv()
                .expect("classified agents must be sent before any detail read begins");
            let CensusUpdate::Local {
                rows: first_rows, ..
            } = first
            else {
                panic!("a local scan must publish a local answer");
            };
            assert_eq!(first_rows.as_slice(), std::slice::from_ref(&classified));
            assert!(matches!(updates.try_recv(), Err(mpsc::TryRecvError::Empty)));
            rows[0].branch = Some("main".to_string());
            rows[0].children.push(child.clone());
        },
    );

    let detailed = updates
        .try_recv()
        .expect("the enriched agents must be sent after their details are read");
    let CensusUpdate::Local {
        rows: detailed_rows,
        ..
    } = detailed
    else {
        panic!("a local scan must publish a local answer");
    };
    assert_eq!(
        detailed_rows,
        [AgentRow {
            role_fallback: crate::census::RoleFallback::None,
            branch: Some("main".to_string()),
            children: vec![child],
            ..classified
        }]
    );
    assert!(matches!(updates.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn later_scans_publish_completed_rows_without_a_classified_refresh() {
    let processes = [ProcessEntry {
        pid:         428_044,
        parent:      Some(1),
        name:        "claude".to_string(),
        arguments:   vec!["claude".to_string()],
        started:     100,
        directory:   None,
        config_home: ConfigHome::Unread,
        claude_pid:  None,
    }];
    let sessions = [SessionRecord {
        pid:        428_044,
        session_id: "abcd1234".to_string(),
        cwd:        None,
        name:       Some("handler".to_string()),
        status:     Some("busy".to_string()),
        entrypoint: None,
    }];
    let mut scanner = super::LocalScanner::new(AccountLabels::default());
    let mut first_answers = Vec::new();
    scanner.scan_stages(
        &processes,
        &sessions,
        &[],
        None,
        |rows| first_answers.push(rows),
        |_, rows| rows[0].branch = Some("main".to_string()),
    );
    assert_eq!(first_answers.len(), 2);
    let completed = first_answers.pop().expect("the first scan completes");
    let (sender, updates) = mpsc::channel();
    scanner.scan_stages(
        &processes,
        &sessions,
        &[],
        None,
        |rows| sender.send(rows).expect("the census receiver remains open"),
        |_, rows| {
            assert!(
                matches!(updates.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "a refresh must keep the previous completed answer until details arrive"
            );
            rows[0].branch = Some("main".to_string());
        },
    );
    assert_eq!(
        updates.try_recv().expect("the refresh completes"),
        completed
    );
    assert!(matches!(updates.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

/// Missing conversation data retains the known turn and hides raw worker identities.
#[test]
fn activity_scan_missing_transcript_preserves_reported_turn_and_unread_worker_uses_role() {
    let directory = tempfile::tempdir().expect("isolated transcript directory");
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude", "name":"root", "status":"busy", "pid":10, "started":10,
        "directory":"/fixture", "desktop":null, "branch":null, "launched_by":null,
        "children":[{"kind":"thread", "depth":0, "pid":null, "name":"raw-thread-id",
            "thread_id":"raw-thread-id", "started":20, "timer":null}]
    }]))
    .expect("scan fixture rows");
    let mut scanner = super::LocalScanner::new(AccountLabels::default());
    let transcripts =
        std::collections::HashMap::from([(10, directory.path().join("missing.jsonl"))]);
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "thinking");
    assert_eq!(
        rows[0].children[0].worker_task.label(
            crate::census::worker_dispatch::WorkerRoleSource::Unroled,
            &rows[0].children[0].name,
        ),
        "worker"
    );
    std::fs::write(directory.path().join("missing.jsonl"), "").expect("empty transcript writes");
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "thinking");
}

#[test]
fn transcript_cache_changes_paths_and_forgets_sessions_that_disappear()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.jsonl");
    let second = directory.path().join("second.jsonl");
    let tool = |name| {
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":name,"input":{}}]}}).to_string()
    };
    std::fs::write(&first, format!("{}\n", tool("Read")))?;
    std::fs::write(&second, format!("{}\n", tool("Edit")))?;
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"root","status":"busy","pid":10,"started":10,"directory":"/fixture","children":[]
    }]))?;
    let mut scanner = super::LocalScanner::new(AccountLabels::default());
    scanner.attach_activity_and_names(
        &[],
        &std::collections::HashMap::from([(10, first.clone())]),
        &mut rows,
    );
    assert_eq!(rows[0].activity.text(), "reading");
    scanner.attach_activity_and_names(
        &[],
        &std::collections::HashMap::from([(10, second.clone())]),
        &mut rows,
    );
    assert_eq!(rows[0].activity.text(), "writing");
    assert!(!scanner.transcript_activity.contains_key(&first));
    assert!(scanner.transcript_activity.contains_key(&second));
    scanner.attach_activity_and_names(&[], &std::collections::HashMap::new(), &mut []);
    assert!(scanner.transcript_activity.is_empty());
    Ok(())
}
