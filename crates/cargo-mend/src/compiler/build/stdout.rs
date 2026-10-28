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

/// The one cargo message `collect_units` acts on; every other `reason`
/// deserializes to `CargoMessage::Other`.
#[derive(Deserialize)]
#[serde(tag = "reason")]
enum CargoMessage {
    #[serde(rename = "compiler-artifact")]
    CompilerArtifact {
        package_id: String,
        target:     CargoTarget,
        filenames:  Vec<PathBuf>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct CargoTarget {
    kind:     Vec<String>,
    src_path: PathBuf,
}

/// One Cargo unit whose metadata has a corresponding mend report.
#[derive(Debug)]
pub(super) struct AnalysisUnit {
    pub package_id:    String,
    pub metadata_path: PathBuf,
    pub crate_root:    PathBuf,
}

impl AnalysisUnit {
    pub(super) fn report_path(&self) -> PathBuf {
        persistence::report_path_for_metadata(&self.metadata_path)
    }
}

/// Reads cargo's stdout and keeps the owning package and metadata of each unit,
/// including fresh units. Build scripts are outside mend's source analysis.
///
/// A line that does not parse as a `CargoMessage` was written to cargo's stdout
/// by another process, such as a user's `RUSTC_WRAPPER`; it is forwarded in
/// `BuildOutputMode::Full`, the one mode that inherited cargo's stdout before
/// this reader existed.
pub(super) fn collect_units(
    stdout: ChildStdout,
    output_mode: BuildOutputMode,
) -> io::Result<Vec<AnalysisUnit>> {
    let mut units = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        match from_str::<CargoMessage>(&line) {
            Ok(CargoMessage::CompilerArtifact {
                package_id,
                target,
                filenames,
            }) => {
                if target.kind.iter().any(|kind| kind == "custom-build") {
                    continue;
                }
                units.extend(
                    filenames
                        .iter()
                        .filter(|filename| {
                            filename.extension().and_then(OsStr::to_str)
                                == Some(METADATA_FILE_EXTENSION)
                        })
                        .map(|metadata_path| AnalysisUnit {
                            package_id:    package_id.clone(),
                            metadata_path: metadata_path.clone(),
                            crate_root:    target.src_path.clone(),
                        }),
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
    Ok(units)
}
