//! Read known loose references in repository fixtures without spawning Git.

use std::fs;
use std::path::Path;

/// Read a reference that the fixture deliberately keeps loose in the common Git directory.
pub(crate) fn read_loose_ref(root: &Path, reference: &str) -> String {
    fs::read_to_string(root.join(".git").join(reference))
        .expect("fixture's loose reference should read")
        .trim()
        .to_owned()
}
