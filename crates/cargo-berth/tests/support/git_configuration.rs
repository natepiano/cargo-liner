//! Filesystem-only configuration setup for repository fixtures.

use std::fs;
use std::io;
use std::path::Path;

/// Append Git configuration to a main checkout without spawning Git.
pub(crate) fn append_git_configuration(root: &Path, configuration: &str) -> io::Result<()> {
    let path = root.join(".git/config");
    let mut contents = fs::read_to_string(&path)?;
    contents.push_str(configuration);
    fs::write(path, contents)
}
