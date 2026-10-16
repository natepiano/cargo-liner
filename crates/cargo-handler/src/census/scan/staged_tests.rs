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
use super::SessionRecord;
use crate::census::Account;
use crate::census::Agent;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::CodexThreadId;
use crate::census::HierarchyEdges;
use crate::census::ServiceTier;
use crate::census::SessionIdentity;
use crate::census::ThreadState;
use crate::census::account::AccountLabels;
use crate::census::account::ConfigHome;
use crate::census::worker_dispatch::WorkerDispatch;

/// The shell detail attached after classification is published.
fn staged_child() -> ChildRow {
    ChildRow {
        thread_id:    CodexThreadId::Unrecorded,
        dispatch:     WorkerDispatch::Plain,
        thread_state: ThreadState::Idle,
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
    let classified = AgentRow {
        session_id:      SessionIdentity::Claude("abcd1234".to_string()),
        agent:           Agent::Claude,
        account:         Account::Home(AccountLabels::default().resolve(
            Path::new("/home/test/.claude1"),
            Some(Path::new("/home/test")),
        )),
        service_tier:    ServiceTier::Unrecorded,
        name:            "handler".to_string(),
        status:          Some("busy".to_string()),
        started:         100,
        process_started: 100,
        pid:             428_044,
        desktop:         None,
        directory:       "~/rust/handler".to_string(),
        branch:          None,
        launched_by:     None,
        children:        Vec::new(),
    };
    let child = staged_child();
    let (sender, updates) = mpsc::channel();

    super::LocalScanner::new(AccountLabels::default()).scan_stages(
        &processes,
        &sessions,
        &[],
        Some(Path::new("/home/test")),
        |rows| {
            sender
                .send(CensusUpdate::Local(rows, HierarchyEdges::default()))
                .expect("the census receiver should remain open");
        },
        |_, rows| {
            let first = updates
                .try_recv()
                .expect("classified agents must be sent before any detail read begins");
            let CensusUpdate::Local(first_rows, _) = first else {
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
    let CensusUpdate::Local(detailed_rows, _) = detailed else {
        panic!("a local scan must publish a local answer");
    };
    assert_eq!(
        detailed_rows,
        [AgentRow {
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
