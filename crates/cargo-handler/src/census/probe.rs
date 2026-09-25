//! The probe: this machine's agents as one line of JSON, which
//! `cargo-handler probe` prints and a machine running the summary reads
//! back over ssh.
//!
//! ```json
//! {"schema":2,"machine":"natedev","rows":[{"agent":"claude","name":"enh/handler","status":"busy","started":1790000000,"pid":428044,"directory":"~/rust/handler","launched_by":null,"children":[{"depth":0,"kind":"shell","pid":3911067,"name":"Run the tests","started":1790000100}]}]}
//! ```

use std::io;
use std::io::Write;
use std::process::ExitCode;

use serde::Deserialize;
use serde::Serialize;

use super::AgentRow;
use super::local_machine_name;
use super::scan::LocalScanner;
use crate::constants::BINARY_NAME;
use crate::constants::PROBE_SCHEMA;
use crate::constants::UNREADABLE_PROBE_REASON;

/// What the probe prints.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProbeReport {
    /// [`PROBE_SCHEMA`] of the build that printed it.
    schema:  u32,
    /// The printing machine's short host name.
    machine: String,
    /// Its agents, oldest first, each with what it is running.
    rows:    Vec<AgentRow>,
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
    let report = ProbeReport {
        schema:  PROBE_SCHEMA,
        machine: local_machine_name(),
        rows:    LocalScanner::new().scan(),
    };
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
pub(super) fn parse_report(output: &[u8]) -> Result<Vec<AgentRow>, String> {
    let header: SchemaHeader =
        serde_json::from_slice(output).map_err(|_| UNREADABLE_PROBE_REASON.to_string())?;
    if header.schema != PROBE_SCHEMA {
        return Err(format!(
            "probe version {}, expected {PROBE_SCHEMA}",
            header.schema
        ));
    }
    serde_json::from_slice::<ProbeReport>(output)
        .map(|report| report.rows)
        .map_err(|_| UNREADABLE_PROBE_REASON.to_string())
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::census::Agent;
    use crate::census::ChildKind;
    use crate::census::ChildRow;

    /// One row of an agent's cell.
    fn child(depth: u8, kind: ChildKind, pid: Option<u32>, name: &str, started: u64) -> ChildRow {
        ChildRow {
            depth,
            kind,
            pid,
            name: name.to_string(),
            started,
        }
    }

    /// A report of one Codex row, one Claude Code row running one of
    /// each kind of child, and a session that Claude Code row opened.
    fn report() -> ProbeReport {
        ProbeReport {
            schema:  PROBE_SCHEMA,
            machine: "mac".to_string(),
            rows:    vec![
                AgentRow {
                    agent:       Agent::Codex,
                    name:        "ChatGPT".to_string(),
                    status:      None,
                    started:     1_790_000_000,
                    pid:         76_130,
                    directory:   "/".to_string(),
                    launched_by: None,
                    children:    Vec::new(),
                },
                AgentRow {
                    agent:       Agent::Claude,
                    name:        "natemccoy-30".to_string(),
                    status:      Some("idle".to_string()),
                    started:     1_790_000_100,
                    pid:         80_020,
                    directory:   "~".to_string(),
                    launched_by: None,
                    children:    vec![
                        child(
                            0,
                            ChildKind::Shell,
                            Some(80_100),
                            "Run the mesh",
                            1_790_000_110,
                        ),
                        child(
                            1,
                            ChildKind::Process(Agent::Codex),
                            Some(80_200),
                            "app-server",
                            1_790_000_111,
                        ),
                        child(2, ChildKind::Thread, None, "phase 1", 1_790_000_112),
                        child(0, ChildKind::Subagent, None, "Review", 1_790_000_120),
                        child(
                            0,
                            ChildKind::Session(Agent::Claude),
                            Some(81_020),
                            "worker",
                            1_790_000_130,
                        ),
                    ],
                },
                AgentRow {
                    agent:       Agent::Claude,
                    name:        "worker".to_string(),
                    status:      Some("busy".to_string()),
                    started:     1_790_000_130,
                    pid:         81_020,
                    directory:   "~".to_string(),
                    launched_by: Some(80_020),
                    children:    Vec::new(),
                },
            ],
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
                r#"{"schema":2,"machine":"mac","rows":["#,
                r#"{"agent":"codex","name":"ChatGPT","status":null,"started":1790000000,"pid":76130,"directory":"/","launched_by":null,"children":[]},"#,
                r#"{"agent":"claude","name":"natemccoy-30","status":"idle","started":1790000100,"pid":80020,"directory":"~","launched_by":null,"children":["#,
                r#"{"depth":0,"kind":"shell","pid":80100,"name":"Run the mesh","started":1790000110},"#,
                r#"{"depth":1,"kind":{"process":"codex"},"pid":80200,"name":"app-server","started":1790000111},"#,
                r#"{"depth":2,"kind":"thread","pid":null,"name":"phase 1","started":1790000112},"#,
                r#"{"depth":0,"kind":"subagent","pid":null,"name":"Review","started":1790000120},"#,
                r#"{"depth":0,"kind":{"session":"claude"},"pid":81020,"name":"worker","started":1790000130}]},"#,
                r#"{"agent":"claude","name":"worker","status":"busy","started":1790000130,"pid":81020,"directory":"~","launched_by":80020,"children":[]}]}"#,
            )
        );
        assert_eq!(
            parse_report(format!("{json}\n").as_bytes()),
            Ok(report().rows)
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
        assert_eq!(parse_report(br#"{"schema":2,"machine":"mac"}"#), unreadable);
    }
}
