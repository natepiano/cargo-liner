//! Account commands publish on their own cadence without blocking a frame.

#![expect(
    clippy::expect_used,
    reason = "scheduler handshakes must finish within the fixture deadline"
)]

use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::sync::mpsc::SyncSender;
use std::time::Duration;

use super::accounts_tests;
use crate::census::accounts;
use crate::census::accounts::AccountsCommand;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsMonitor;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::HierarchyError;

const DEADLINE: Duration = Duration::from_millis(250);

/// Each invocation is held until the test supplies a command outcome.
struct Invocation {
    command: String,
    reply:   SyncSender<Result<AccountsReading, AccountsFailure>>,
}

fn controlled_command(calls: Sender<Invocation>) -> AccountsCommand {
    Box::new(move |command| {
        let (reply, result) = mpsc::sync_channel(1);
        calls
            .send(Invocation {
                command: command.to_owned(),
                reply,
            })
            .map_err(|_| AccountsFailure::Command(HierarchyError::CouldNotStart))?;
        result
            .recv_timeout(DEADLINE)
            .unwrap_or(Err(AccountsFailure::Command(HierarchyError::TimedOut)))
    })
}

fn reading(left: u8) -> AccountsReading {
    AccountsReading::parse(
        &serde_json::to_vec(&serde_json::json!({
            "accounts":[{"label":"codex2","tool":"codex","roles":["workers"],
                "weekly_left_percent":left,"resets_at":null,"credits":"unlimited",
                "use_credits":true,"read_at":"2026-10-16T00:00:00Z"}],"problems":[]
        }))
        .expect("reading JSON"),
    )
    .expect("wire reading")
}

#[test]
fn accounts_refresh_background_cadence_publishes_changed_readings_and_recovers_after_failure() {
    let (calls, started) = mpsc::channel();
    let mut monitor = AccountsMonitor::spawn_with(
        "fixture accounts --stored".into(),
        controlled_command(calls),
        Duration::from_millis(1),
    );
    let first = started
        .recv_timeout(DEADLINE)
        .expect("background command starts");
    assert_eq!(first.command, "fixture accounts --stored");
    assert!(
        !monitor.refresh(),
        "a frame does not wait for a blocked command"
    );
    assert!(matches!(monitor.table, AccountsTable::AwaitingFirstReading));
    first.reply.send(Ok(reading(85))).expect("first answer");
    // Starting a subsequent invocation proves the scheduler published the prior answer.
    let second = started
        .recv_timeout(DEADLINE)
        .expect("scheduler independently starts the next refresh");
    assert!(monitor.refresh());
    assert!(
        accounts_tests::text(&accounts_tests::drawn(&monitor.table))
            .join("\n")
            .contains("85%")
    );
    assert!(!monitor.refresh(), "no command reruns on a frame");
    second.reply.send(Ok(reading(22))).expect("changed usage");
    let third = started.recv_timeout(DEADLINE).expect("third refresh");
    assert!(monitor.refresh());
    let refreshed = accounts_tests::text(&accounts_tests::drawn(&monitor.table)).join("\n");
    assert!(
        refreshed.contains("22%") && !refreshed.contains("85%"),
        "{refreshed}"
    );
    third
        .reply
        .send(AccountsReading::parse(b"not JSON"))
        .expect("bad JSON answer");
    let fourth = started
        .recv_timeout(DEADLINE)
        .expect("refresh after malformed answer");
    assert!(monitor.refresh());
    let failed = accounts_tests::text(&accounts_tests::drawn(&monitor.table)).join("\n");
    assert!(
        failed.contains("22%") && failed.contains("invalid JSON"),
        "{failed}"
    );
    assert!(matches!(
        monitor.table,
        AccountsTable::FailedKeepingLast { .. }
    ));
    fourth.reply.send(Ok(reading(11))).expect("recovered usage");
    let fifth = started
        .recv_timeout(DEADLINE)
        .expect("refresh after recovery");
    assert!(monitor.refresh());
    let recovered = accounts_tests::text(&accounts_tests::drawn(&monitor.table)).join("\n");
    assert!(
        recovered.contains("11%") && !recovered.contains("invalid JSON"),
        "{recovered}"
    );
    assert!(matches!(monitor.table, AccountsTable::Ready(_)));
    drop(monitor);
    fifth
        .reply
        .send(Ok(reading(11)))
        .expect("release final background command");
}

#[test]
fn accounts_refresh_real_failed_and_malformed_commands_leave_the_agents_table_visible() {
    for command in ["exit 7", "printf 'this is not JSON'"] {
        let result = accounts::read_accounts(command);
        assert!(result.is_err(), "{command}");
        let mut table = AccountsTable::AwaitingFirstReading;
        table.apply(result);
        let output = accounts_tests::text(&accounts_tests::drawn(&table)).join("\n");
        assert!(
            output.contains("zeta-session") && output.contains("alpha-session"),
            "{output}"
        );
        assert!(output.contains("accounts:"), "{output}");
        assert!(!output.contains("codex2"), "no invented rows: {output}");
    }
}

#[test]
fn accounts_refresh_unconfigured_source_never_invokes_the_command_runner() {
    let (calls, started) = mpsc::channel();
    let mut monitor =
        AccountsMonitor::spawn_with("  ".into(), controlled_command(calls), Duration::ZERO);
    assert!(!monitor.refresh());
    assert!(matches!(monitor.table, AccountsTable::NotConfigured));
    assert!(
        started.try_recv().is_err(),
        "unconfigured command must not run"
    );
    assert!(
        !accounts_tests::text(&accounts_tests::drawn(&monitor.table))
            .join("\n")
            .contains("accounts")
    );
}
