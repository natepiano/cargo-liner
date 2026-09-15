use std::path::Path;
use std::path::PathBuf;

use crate::compiler::constants::REPORT_FILE_EXTENSION;

/// The `StoredReport` path for one compilation unit: the unit's `.rmeta` path
/// with its extension replaced by `REPORT_FILE_EXTENSION`.
///
/// Cargo names that file `deps/lib<crate>-<unit id>.rmeta`, and the unit id
/// changes with everything cargo fingerprints, the `RUSTC_WORKSPACE_WRAPPER`
/// path included. Two mend builds therefore write to different report files and
/// never overwrite each other. The driver passes the path rustc reports from
/// `filename_for_metadata`; `run_cargo_command` passes the `filenames` cargo
/// lists in each `compiler-artifact` message, so both sides name the same file.
pub(in crate::compiler) fn report_path_for_metadata(metadata_path: &Path) -> PathBuf {
    metadata_path.with_extension(REPORT_FILE_EXTENSION)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::report_path_for_metadata;

    #[test]
    fn report_path_keeps_the_unit_id_and_directory() {
        let metadata_path = Path::new("/target/debug/deps/libmember-cc3113e9bc221183.rmeta");

        assert_eq!(
            report_path_for_metadata(metadata_path),
            Path::new("/target/debug/deps/libmember-cc3113e9bc221183.mend.json")
        );
    }
}
