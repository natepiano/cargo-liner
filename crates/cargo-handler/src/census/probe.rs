//! The probe: this machine's agents as one line of JSON, which
//! `cargo-handler probe` prints and a machine running the summary reads
//! back over ssh.
//!
//! ```json
//! {"schema":1,"machine":"natedev","rows":[{"agent":"claude","name":"enh/handler","status":"busy","started":1790000000,"pid":428044,"directory":"~/rust/handler"}]}
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
pub(crate) struct ProbeReport {
    /// [`PROBE_SCHEMA`] of the build that printed it.
    pub(crate) schema:  u32,
    /// The printing machine's short host name.
    pub(crate) machine: String,
    /// Its top-level agents, oldest first.
    pub(crate) rows:    Vec<AgentRow>,
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
pub(crate) fn parse_report(output: &[u8]) -> Result<Vec<AgentRow>, String> {
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

    /// A report of one Claude Code and one Codex row.
    fn report() -> ProbeReport {
        ProbeReport {
            schema:  PROBE_SCHEMA,
            machine: "mac".to_string(),
            rows:    vec![
                AgentRow {
                    agent:     Agent::Codex,
                    name:      "ChatGPT".to_string(),
                    status:    None,
                    started:   1_790_000_000,
                    pid:       76_130,
                    directory: "/".to_string(),
                },
                AgentRow {
                    agent:     Agent::Claude,
                    name:      "natemccoy-30".to_string(),
                    status:    Some("idle".to_string()),
                    started:   1_790_000_100,
                    pid:       80_020,
                    directory: "~".to_string(),
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
            r#"{"schema":1,"machine":"mac","rows":[{"agent":"codex","name":"ChatGPT","status":null,"started":1790000000,"pid":76130,"directory":"/"},{"agent":"claude","name":"natemccoy-30","status":"idle","started":1790000100,"pid":80020,"directory":"~"}]}"#
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
        let output = br#"{"schema":2,"machine":"mac","agents":[]}"#;

        assert_eq!(
            parse_report(output),
            Err(format!("probe version 2, expected {PROBE_SCHEMA}"))
        );
    }

    /// Output that is not a report, and a report missing its rows, are
    /// both unreadable.
    #[test]
    fn output_that_is_not_a_report_is_unreadable() {
        let unreadable = Err(UNREADABLE_PROBE_REASON.to_string());

        assert_eq!(parse_report(b"bash: warning: setlocale\n"), unreadable);
        assert_eq!(parse_report(b""), unreadable);
        assert_eq!(parse_report(br#"{"schema":1,"machine":"mac"}"#), unreadable);
    }
}
