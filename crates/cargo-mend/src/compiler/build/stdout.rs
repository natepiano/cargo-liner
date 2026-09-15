//! Reads cargo's `--message-format=json-render-diagnostics` stdout.
//!
//! Cargo prints one `compiler-artifact` message per unit, fresh units included,
//! and its `filenames` hold the unit's `deps/lib<crate>-<unit id>.rmeta`. Those
//! paths name exactly the `StoredReport` files this run's units own, which is
//! what `persistence::load_report` reads. Diagnostics are rendered to stderr by
//! cargo under this message format, so stdout carries only messages.

use std::ffi::OsStr;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::ChildStdout;

use serde::Deserialize;
use serde_json::from_str;

use super::BuildOutputMode;
use crate::compiler::constants::METADATA_FILE_EXTENSION;
use crate::compiler::persistence;

/// The one cargo message `collect_report_paths` acts on; every other `reason`
/// deserializes to `CargoMessage::Other`.
#[derive(Deserialize)]
#[serde(tag = "reason")]
enum CargoMessage {
    #[serde(rename = "compiler-artifact")]
    CompilerArtifact { filenames: Vec<PathBuf> },
    #[serde(other)]
    Other,
}

/// Reads cargo's stdout to its end and returns the `StoredReport` path of every
/// unit whose `compiler-artifact` message lists a `.rmeta` file.
///
/// A line that does not parse as a `CargoMessage` was written to cargo's stdout
/// by another process, such as a user's `RUSTC_WRAPPER`; it is forwarded in
/// `BuildOutputMode::Full`, the one mode that inherited cargo's stdout before
/// this reader existed.
pub(super) fn collect_report_paths(
    stdout: ChildStdout,
    output_mode: BuildOutputMode,
) -> io::Result<Vec<PathBuf>> {
    let mut report_paths = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        match from_str::<CargoMessage>(&line) {
            Ok(CargoMessage::CompilerArtifact { filenames }) => {
                report_paths.extend(
                    filenames
                        .iter()
                        .filter(|filename| {
                            filename.extension().and_then(OsStr::to_str)
                                == Some(METADATA_FILE_EXTENSION)
                        })
                        .map(|metadata_path| persistence::report_path_for_metadata(metadata_path)),
                );
            },
            Ok(CargoMessage::Other) => {},
            Err(_) => match output_mode {
                BuildOutputMode::Full => println!("{line}"),
                BuildOutputMode::Json
                | BuildOutputMode::SuppressUnusedImportWarnings
                | BuildOutputMode::Quiet => {},
            },
        }
    }
    Ok(report_paths)
}
