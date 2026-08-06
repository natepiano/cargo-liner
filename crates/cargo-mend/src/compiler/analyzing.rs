//! Which targets are running mend's analysis right now.
//!
//! mend analyzes inside each `rustc` process cargo spawns, so the parent process
//! that draws the progress line cannot see what those children are doing. Cargo
//! is no help either: it prints one `Checking <package>` line however many
//! targets the package holds, so a package with a library and thirty examples
//! produces one line and then minutes of silence.
//!
//! Each analyzing wrapper therefore leaves a file named after its process id in
//! a directory under `MEND_FINDINGS_DIR`, holding the crate name it is working
//! on, and removes it when the analysis ends. The parent lists that directory
//! once per spinner frame. A file rather than a line on stderr because cargo may
//! hold a unit's `rustc` stderr until the unit finishes, which would deliver the
//! announcement after the work it announces.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process;

use super::constants::ANALYZING_DIR_NAME;

/// The directory holding one file per in-flight analysis.
fn analyzing_dir(findings_dir: &Path) -> PathBuf {
    findings_dir.join(ANALYZING_DIR_NAME)
}

/// Announces one target's analysis for as long as it is alive.
///
/// The marker is removed on drop, which covers the compiler bailing out partway
/// through as well as the ordinary return. A stale marker left by a killed
/// process only costs a wrong name on the progress line, never a wrong result.
pub(super) struct AnalyzingMarker {
    path: Option<PathBuf>,
}

impl AnalyzingMarker {
    /// Writes the marker, or yields an inert value if the directory or file
    /// cannot be created. Progress display is never worth failing a run over.
    pub(super) fn new(findings_dir: &Path, crate_name: &str) -> Self {
        let directory = analyzing_dir(findings_dir);
        if fs::create_dir_all(&directory).is_err() {
            return Self { path: None };
        }
        let path = directory.join(process::id().to_string());
        if fs::write(&path, crate_name).is_err() {
            return Self { path: None };
        }
        Self { path: Some(path) }
    }
}

impl Drop for AnalyzingMarker {
    fn drop(&mut self) {
        if let Some(path) = self.path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

/// The crate names currently being analyzed, sorted so the progress line does
/// not reorder itself between frames while the same set is in flight.
pub(super) fn targets_in_flight(findings_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(analyzing_dir(findings_dir)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter(|crate_name| !crate_name.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}
