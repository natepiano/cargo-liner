//! The probe: this machine's agents as one line of JSON, which
//! `cargo-handler probe` prints and a machine running the summary reads
//! back over ssh.
//!
//! ```json
//! {"schema":3,"machine":"natedev","rows":[{"agent":"claude","service_tier":"unrecorded","name":"enh/handler","activity":{"active":"thinking"},"started":1790000000,"process_started":1790000000,"pid":428044,"desktop":"cargo handler","directory":"~/rust/handler","branch":"enh/handler","launched_by":null,"children":[{"depth":0,"kind":"shell","service_tier":"unrecorded","pid":3911067,"name":"Run the tests","started":1790000100}]}]}
//! ```

use std::io;
use std::io::Write;
use std::process::ExitCode;

use serde::Deserialize;
use serde::Serialize;

use super::Account;
use super::Agent;
use super::AgentActivity;
use super::AgentRow;
use super::HierarchyEdges;
use super::HierarchyNotice;
use super::HierarchySnapshot;
use super::HierarchySource;
use super::SessionIdentity;
use super::account::AccountLabels;
use super::hierarchy;
use super::hierarchy::CommandRunner;
use super::hierarchy::HierarchyAnswer;
use super::hierarchy::HierarchyError;
use super::local_machine_name;
use super::scan::LocalScanner;
use crate::config::CargoHandler;
use crate::config::LoadedConfig;
use crate::constants::BINARY_NAME;
use crate::constants::PROBE_SCHEMA;
use crate::constants::UNREADABLE_PROBE_REASON;
use crate::startup_log;

/// A probe row before compatibility defaults are resolved into domain values.
#[derive(Deserialize)]
pub(super) struct ProbeAgentRow {
    agent:           super::Agent,
    #[serde(default)]
    session_id:      Option<String>,
    #[serde(default)]
    service_tier:    super::ServiceTier,
    #[serde(default)]
    account:         Account,
    name:            String,
    #[serde(default)]
    status:          Option<String>,
    #[serde(default)]
    activity:        Option<super::AgentActivity>,
    started:         u64,
    /// Older probes supply their identity through `started` instead.
    process_started: Option<u64>,
    pid:             u32,
    desktop:         Option<String>,
    directory:       String,
    branch:          Option<String>,
    launched_by:     Option<u32>,
    children:        Vec<super::ChildRow>,
}

impl From<ProbeAgentRow> for AgentRow {
    fn from(row: ProbeAgentRow) -> Self {
        Self {
            agent:           row.agent,
            session_id:      row
                .session_id
                .map_or(SessionIdentity::Unrecorded, SessionIdentity::from),
            service_tier:    row.service_tier,
            account:         row.account,
            name:            row.name,
            activity:        row
                .activity
                .unwrap_or_else(|| AgentActivity::from(row.status.as_deref())),
            started:         row.started,
            process_started: row.process_started.unwrap_or(row.started),
            pid:             row.pid,
            desktop:         row.desktop,
            directory:       row.directory,
            branch:          row.branch,
            launched_by:     row.launched_by,
            children:        row.children,
        }
    }
}

/// What the probe prints.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProbeReport {
    /// [`PROBE_SCHEMA`] of the build that printed it.
    schema:           u32,
    /// The printing machine's short host name.
    machine:          String,
    /// Its agents, oldest first, each with what it is running.
    rows:             Vec<AgentRow>,
    /// Optional for compatibility with older schema-3 probes.
    #[serde(default)]
    edges:            HierarchyEdges,
    /// A failed command must not erase the receiver's last successful links.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hierarchy_failed: bool,
    /// Typed discovery health; older probes omit it and remain readable.
    #[serde(default)]
    hierarchy_notice: HierarchyNotice,
    /// Transport-only worker carriers, ignored by older schema-3 peers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    thread_hosts:     Vec<AgentRow>,
}

impl ProbeReport {
    /// Keep the schema-3 failure flag for receivers that predate typed notices.
    const fn record_failure(&mut self, reason: HierarchyError) {
        self.hierarchy_failed = true;
        self.hierarchy_notice = HierarchyNotice::Failed(reason);
    }

    /// Separate visible agents from carriers at the outgoing wire boundary.
    fn from_answer(machine: String, rows: Vec<AgentRow>, edges: HierarchyEdges) -> Self {
        let (rows, thread_hosts) = rows.into_iter().partition(AgentRow::is_visible);
        Self {
            schema: PROBE_SCHEMA,
            machine,
            rows,
            edges,
            thread_hosts,
            hierarchy_failed: false,
            hierarchy_notice: HierarchyNotice::Clear,
        }
    }

    /// Restore transport-only identity before handing an answer to the census.
    fn into_answer(self) -> (Vec<AgentRow>, HierarchyAnswer) {
        let rows = self
            .rows
            .into_iter()
            .chain(self.thread_hosts.into_iter().map(|mut host| {
                host.agent = Agent::CodexThreadHost;
                host
            }))
            .collect();
        let hierarchy = match self.hierarchy_notice {
            HierarchyNotice::Failed(reason) => HierarchyAnswer::Failed(reason),
            HierarchyNotice::NoSessionMatched => HierarchyAnswer::Unmatched(self.edges),
            HierarchyNotice::Clear if self.hierarchy_failed => {
                HierarchyAnswer::Failed(HierarchyError::ExitedUnsuccessfully)
            },
            HierarchyNotice::Clear => HierarchyAnswer::Answered(self.edges),
        };
        (rows, hierarchy)
    }
}

/// The one field read before the rest, so a machine running another
/// version is told apart from one printing something else entirely.
#[derive(Deserialize)]
struct SchemaHeader {
    /// The version the report says it is.
    schema: u32,
}

/// Scan this machine once and print the report to stdout.
pub(crate) fn print() -> ExitCode {
    startup_log::init();
    let loaded = LoadedConfig::load::<CargoHandler>();
    let source = HierarchySource::from(loaded.config.census.hierarchy_command);
    let mut report = ProbeReport::from_answer(
        local_machine_name(),
        LocalScanner::new(AccountLabels::new(
            &loaded.config.accounts,
            dirs::home_dir().as_deref(),
        ))
        .scan(),
        HierarchyEdges::default(),
    );
    match hierarchy::read(&source, &CommandRunner) {
        Ok(edges) => {
            let snapshot = HierarchySnapshot {
                edges,
                notice: HierarchyNotice::Clear,
            };
            let snapshot = if matches!(source, HierarchySource::Command(_)) {
                let mut rows = report.rows.clone();
                rows.extend_from_slice(&report.thread_hosts);
                snapshot.matched(&rows)
            } else {
                snapshot
            };
            report.edges = snapshot.edges;
            report.hierarchy_notice = snapshot.notice;
        },
        Err(reason) => {
            report.record_failure(reason);
        },
    }
    let written = serde_json::to_string(&report)
        .map_err(io::Error::from)
        .and_then(|json| writeln!(io::stdout().lock(), "{json}"));
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{BINARY_NAME}: probe: {error}");
            ExitCode::FAILURE
        },
    }
}

/// The rows of a report read back from `output`, or why there are none:
/// output that is not a report, or a report from another version.
pub(super) fn parse_report(output: &[u8]) -> Result<(Vec<AgentRow>, HierarchyEdges), String> {
    parse_answer(output).map(|(rows, hierarchy)| {
        let edges = match hierarchy {
            HierarchyAnswer::Answered(edges) | HierarchyAnswer::Unmatched(edges) => edges,
            HierarchyAnswer::Failed(_) => HierarchyEdges::default(),
        };
        (rows, edges)
    })
}

/// Decode hierarchy failure separately from an authoritative empty answer.
pub(super) fn parse_answer(output: &[u8]) -> Result<(Vec<AgentRow>, HierarchyAnswer), String> {
    let header: SchemaHeader =
        serde_json::from_slice(output).map_err(|_| UNREADABLE_PROBE_REASON.to_string())?;
    if header.schema != PROBE_SCHEMA {
        return Err(format!(
            "probe version {}, expected {PROBE_SCHEMA}",
            header.schema
        ));
    }
    serde_json::from_slice::<ProbeReport>(output)
        .map(ProbeReport::into_answer)
        .map_err(|_| UNREADABLE_PROBE_REASON.to_string())
}

#[cfg(test)]
#[path = "probe/hierarchy_transport_tests.rs"]
mod hierarchy_transport_tests;

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::census::Account;
    use crate::census::Agent;
    use crate::census::AgentActivity;
    use crate::census::ChildKind;
    use crate::census::ChildRow;
    use crate::census::Machine;
    use crate::census::MachineState;
    use crate::census::ServiceTier;
    use crate::census::Timer;
    use crate::census::worker_dispatch::WorkerDispatch;
    use crate::census::worker_dispatch::WorkerTask;

    /// One row of an agent's cell.
    fn child(depth: u8, kind: ChildKind, pid: Option<u32>, name: &str, started: u64) -> ChildRow {
        ChildRow {
            worker_task: WorkerTask::Unrecorded,
            thread_id: crate::census::CodexThreadId::Unrecorded,
            dispatch: WorkerDispatch::Plain,
            activity: AgentActivity::Idle,
            depth,
            kind,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            pid,
            name: name.to_string(),
            started,
            timer: None,
        }
    }

    /// A report of one Codex row, one Claude Code row running one of
    /// each kind of child and a timer, and a session that Claude Code row
    /// opened.
    fn report() -> ProbeReport {
        ProbeReport {
            hierarchy_failed: false,
            hierarchy_notice: HierarchyNotice::Clear,
            schema:           PROBE_SCHEMA,
            machine:          "mac".to_string(),
            thread_hosts:     Vec::new(),
            edges:            HierarchyEdges::default(),
            rows:             vec![
                AgentRow {
                    agent:           Agent::Codex,
                    session_id:      SessionIdentity::Unrecorded,
                    service_tier:    ServiceTier::Unrecorded,
                    account:         Account::Unrecorded,
                    name:            "ChatGPT".to_string(),
                    activity:        AgentActivity::Unreported,
                    started:         1_790_000_000,
                    process_started: 1_790_000_000,
                    pid:             76_130,
                    desktop:         None,
                    directory:       "/".to_string(),
                    branch:          None,
                    launched_by:     None,
                    children:        Vec::new(),
                },
                AgentRow {
                    agent:           Agent::Claude,
                    session_id:      SessionIdentity::Unrecorded,
                    service_tier:    ServiceTier::Unrecorded,
                    account:         Account::Unrecorded,
                    name:            "natemccoy-30".to_string(),
                    activity:        AgentActivity::from(Some("idle")),
                    started:         1_790_000_100,
                    process_started: 1_790_000_100,
                    pid:             80_020,
                    desktop:         None,
                    directory:       "~".to_string(),
                    branch:          Some("main".to_string()),
                    launched_by:     None,
                    children:        vec![
                        child(
                            0,
                            ChildKind::Shell,
                            Some(80_100),
                            "Run the mesh",
                            1_790_000_110,
                        ),
                        child(
                            1,
                            ChildKind::UnderShell(Agent::Codex),
                            Some(80_200),
                            "app-server",
                            1_790_000_111,
                        ),
                        ChildRow {
                            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                            service_tier: ServiceTier::Fast,
                            account: Account::Unrecorded,
                            ..child(2, ChildKind::Thread, None, "phase 1", 1_790_000_112)
                        },
                        child(0, ChildKind::Subagent, None, "Review", 1_790_000_120),
                        ChildRow {
                            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                            timer: Some(Timer {
                                started:  1_790_000_125,
                                deadline: 1_790_000_425,
                            }),
                            ..child(
                                0,
                                ChildKind::Shell,
                                Some(80_300),
                                "Re-arm the progress timer",
                                1_790_000_125,
                            )
                        },
                        child(
                            0,
                            ChildKind::Session(Agent::Claude),
                            Some(81_020),
                            "worker",
                            1_790_000_130,
                        ),
                    ],
                },
                worker(),
            ],
        }
    }

    /// The separate session launched by the report's Claude row.
    fn worker() -> AgentRow {
        AgentRow {
            agent:           Agent::Claude,
            session_id:      SessionIdentity::Unrecorded,
            service_tier:    ServiceTier::Unrecorded,
            account:         Account::Unrecorded,
            name:            "worker".to_string(),
            activity:        AgentActivity::from(Some("busy")),
            started:         1_790_000_130,
            process_started: 1_790_000_130,
            pid:             81_020,
            desktop:         None,
            directory:       "~".to_string(),
            branch:          None,
            launched_by:     Some(80_020),
            children:        Vec::new(),
        }
    }

    /// The printed form is the one the brief fixes, field for field,
    /// and it reads back to the rows it was printed from.
    #[test]
    fn a_report_prints_as_the_known_json_and_reads_back() {
        let json = serde_json::to_string(&report()).expect("a report should serialize");

        assert_eq!(
            json,
            concat!(
                r#"{"schema":3,"machine":"mac","rows":["#,
                r#"{"agent":"codex","session_id":null,"service_tier":"unrecorded","account":null,"name":"ChatGPT","activity":"unreported","started":1790000000,"process_started":1790000000,"pid":76130,"desktop":null,"directory":"/","branch":null,"launched_by":null,"children":[]},"#,
                r#"{"agent":"claude","session_id":null,"service_tier":"unrecorded","account":null,"name":"natemccoy-30","activity":"idle","started":1790000100,"process_started":1790000100,"pid":80020,"desktop":null,"directory":"~","branch":"main","launched_by":null,"children":["#,
                r#"{"depth":0,"kind":"shell","service_tier":"unrecorded","pid":80100,"name":"Run the mesh","started":1790000110},"#,
                r#"{"depth":1,"kind":{"under_shell":"codex"},"service_tier":"unrecorded","pid":80200,"name":"app-server","started":1790000111},"#,
                r#"{"depth":2,"kind":"thread","service_tier":"fast","pid":null,"name":"phase 1","started":1790000112},"#,
                r#"{"depth":0,"kind":"subagent","service_tier":"unrecorded","pid":null,"name":"Review","started":1790000120},"#,
                r#"{"depth":0,"kind":"shell","service_tier":"unrecorded","pid":80300,"name":"Re-arm the progress timer","started":1790000125,"timer":{"started":1790000125,"deadline":1790000425}},"#,
                r#"{"depth":0,"kind":{"session":"claude"},"service_tier":"unrecorded","pid":81020,"name":"worker","started":1790000130}]},"#,
                r#"{"agent":"claude","session_id":null,"service_tier":"unrecorded","account":null,"name":"worker","activity":{"active":"thinking"},"started":1790000130,"process_started":1790000130,"pid":81020,"desktop":null,"directory":"~","branch":null,"launched_by":80020,"children":[]}],"edges":[],"hierarchy_notice":"clear"}"#,
            )
        );
        assert_eq!(
            parse_report(format!("{json}\n").as_bytes()),
            Ok((report().rows, report().edges))
        );
    }

    /// Older rows omit desktop, branch and process birth; absent birth
    /// uses the session start, while desktop and branch remain optional.
    #[test]
    fn a_row_without_a_desktop_or_branch_reads_back_with_none() {
        let output = concat!(
            r#"{"schema":3,"machine":"natedev","rows":["#,
            r#"{"agent":"claude","name":"enh/handler","activity":{"active":"thinking"},"started":1790000000,"pid":428044,"directory":"~/rust/handler","launched_by":null,"children":[]},"#,
            r#"{"agent":"claude","name":"berth-fix","status":"idle","started":1790000100,"pid":2165974,"desktop":"berth_fix","directory":"~/rust/berth","branch":"fix/berth","launched_by":null,"children":[]}]}"#,
        );

        let read: Vec<(Option<String>, Option<String>, u64)> = parse_report(output.as_bytes())
            .expect("a report should parse")
            .0
            .into_iter()
            .map(|row| (row.desktop, row.branch, row.process_started))
            .collect();
        assert_eq!(
            read,
            [
                (None, None, 1_790_000_000),
                (
                    Some("berth_fix".to_string()),
                    Some("fix/berth".to_string()),
                    1_790_000_100
                ),
            ]
        );
    }

    /// Stable session identities and management links survive an optional schema-3 extension.
    #[test]
    fn hierarchy_and_session_ids_round_trip_while_old_probes_remain_readable() {
        let mut report = report();
        report.rows[1].children[2].thread_id =
            super::super::CodexThreadId::Recorded("worker-thread".to_string());
        report.rows[1].session_id = SessionIdentity::Claude("manager".to_string());
        report.rows[2].session_id = SessionIdentity::Claude("member".to_string());
        report.edges = HierarchyEdges(vec![super::super::HierarchyEdge {
            manager: "manager".to_string(),
            member:  "member".to_string(),
            role:    super::super::HierarchyRole::Review,
        }]);
        let output = serde_json::to_vec(&report).expect("the extended report should serialize");
        assert_eq!(
            parse_report(&output),
            Ok((report.rows.clone(), report.edges.clone()))
        );
        let mut old = serde_json::to_value(&report).expect("the report should serialize");
        old.as_object_mut()
            .expect("report is an object")
            .remove("edges");
        for row in old["rows"].as_array_mut().expect("rows is an array") {
            row.as_object_mut()
                .expect("a row is an object")
                .remove("session_id");
            for child in row["children"]
                .as_array_mut()
                .expect("children is an array")
            {
                child
                    .as_object_mut()
                    .expect("a child is an object")
                    .remove("thread_id");
            }
        }
        let (rows, edges) =
            parse_report(&serde_json::to_vec(&old).expect("old report should serialize"))
                .expect("an old probe should parse");
        assert_eq!(edges, HierarchyEdges::default());
        assert!(
            rows.iter()
                .flat_map(|row| &row.children)
                .all(|child| child.thread_id == super::super::CodexThreadId::Unrecorded)
        );
        assert!(
            rows.iter()
                .all(|row| row.session_id == SessionIdentity::Unrecorded)
        );
    }

    /// A missing extension means success, while a failed discovery remains distinct from empty.
    #[test]
    fn old_probe_hierarchy_is_answered_and_failed_probe_keeps_fresh_rows() {
        let mut report = report();
        let old = serde_json::to_vec(&report).expect("serialize old-compatible answer");
        assert!(!String::from_utf8_lossy(&old).contains("hierarchy_failed"));
        assert_eq!(
            parse_answer(&old),
            Ok((
                report.rows.clone(),
                HierarchyAnswer::Answered(HierarchyEdges::default())
            ))
        );
        report.rows[0].name = "fresh row".to_string();
        let mut legacy = serde_json::to_value(&report).expect("serialize failed discovery");
        legacy["hierarchy_failed"] = serde_json::json!(true);
        legacy
            .as_object_mut()
            .expect("probe object")
            .remove("hierarchy_notice");
        let bytes = serde_json::to_vec(&legacy).expect("serialize legacy failure");
        assert_eq!(
            parse_answer(&bytes),
            Ok((
                report.rows,
                HierarchyAnswer::Failed(HierarchyError::ExitedUnsuccessfully)
            ))
        );
    }

    /// Another version's report is named by its version, whatever else
    /// it carries.
    #[test]
    fn another_schema_is_reported_by_version() {
        let output = br#"{"schema":1,"machine":"mac","rows":[]}"#;

        assert_eq!(
            parse_report(output),
            Err(format!("probe version 1, expected {PROBE_SCHEMA}"))
        );
    }

    /// Output that is not a report, and a report missing its rows, are
    /// both unreadable.
    #[test]
    fn output_that_is_not_a_report_is_unreadable() {
        let unreadable = Err(UNREADABLE_PROBE_REASON.to_string());

        assert_eq!(parse_report(b"bash: warning: setlocale\n"), unreadable);
        assert_eq!(parse_report(b""), unreadable);
        assert_eq!(parse_report(br#"{"schema":3,"machine":"mac"}"#), unreadable);
    }
    /// Old schema-3 readers see only real agents, even when a report carries workers.
    #[test]
    fn older_probe_readers_never_see_transport_hosts_as_agents() {
        #[derive(Deserialize)]
        struct OldReport {
            rows: Vec<OldRow>,
        }
        #[derive(Deserialize)]
        struct OldRow {
            agent: OldAgent,
            pid:   u32,
        }
        #[derive(Debug, Deserialize, Eq, PartialEq)]
        #[serde(rename_all = "lowercase")]
        enum OldAgent {
            Claude,
            Codex,
        }

        let (manager, worker) = crate::census::mesh_worker_fixture();
        let mut rows = manager.rows().to_vec();
        rows.extend_from_slice(worker.rows());
        for edges in [
            HierarchyEdges::default(),
            HierarchyEdges(worker.edges().to_vec()),
        ] {
            let report = ProbeReport::from_answer("mac".to_string(), rows.clone(), edges);
            let bytes = serde_json::to_vec(&report).expect("serialize mixed answer");
            let old: OldReport = serde_json::from_slice(&bytes).expect("old peer reads report");
            assert_eq!(old.rows.len(), 1);
            assert_eq!(old.rows[0].agent, OldAgent::Claude);
            assert_eq!(old.rows[0].pid, manager.rows()[0].pid);
        }
    }

    /// Separate carrier data round-trips and still places its worker under a remote manager.
    #[test]
    fn mesh_worker_hosts_round_trip_outside_visible_probe_rows() {
        let (manager, worker) = crate::census::mesh_worker_fixture();
        let original = ProbeReport::from_answer(
            "mac".to_string(),
            worker.rows().to_vec(),
            HierarchyEdges(worker.edges().to_vec()),
        );
        let bytes = serde_json::to_vec(&original).expect("serialize worker host");
        let json: Value = serde_json::from_slice(&bytes).expect("wire answer");
        assert_eq!(json["rows"].as_array().expect("visible rows").len(), 0);
        assert_eq!(json["thread_hosts"].as_array().expect("carriers").len(), 1);
        let (rows, edges) = parse_report(&bytes).expect("read worker host answer");
        assert_eq!(rows, worker.rows());
        assert_eq!(edges.0, worker.edges());
        let received = MachineState::Answered(rows, edges.into());
        let placement = crate::census::AgentPlacement::from_machines(&[
            Machine {
                name:  "natedev",
                state: &manager,
            },
            Machine {
                name:  "mac",
                state: &received,
            },
        ]);
        assert_eq!(placement.rows.len(), 1);
        assert_eq!(placement.threads.len(), 1);
        assert_eq!(
            placement.threads[0].owner_row.children[0].name,
            "mesh worker"
        );
    }
}
