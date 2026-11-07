//! Use `setup_reader.py` to publish reader copies without sharing the build inode.

use std::fs;
use std::io;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// Use the copy that `setup_reader.py` loads outside the build directory for Foundation.
pub(super) fn stage(source: &Path) -> io::Result<PathBuf> {
    // `setup_reader.py setup` verifies and loads the path in `NEXTEST_ENV` before these tests
    // start.
    if let Some(reader) = std::env::var_os("CARGO_TILE_TEST_READER_EXECUTABLE") {
        return Ok(PathBuf::from(reader));
    }
    staged_path(&mut stage_command(source))
}

/// Give the isolated staging tests a private cache for `setup_reader.py stage`.
fn stage_in(source: &Path, cache: &Path) -> io::Result<PathBuf> {
    staged_path(stage_command(source).arg(cache))
}

/// Let `setup_reader.py stage` own the default cache directory.
fn stage_command(source: &Path) -> Command {
    let mut command = Command::new("python3");
    command
        .arg("-S")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shim_registration/setup_reader.py"
        ))
        .arg("stage")
        .arg(source);
    command
}

/// Read the published path from `setup_reader.py stage`.
fn staged_path(command: &mut Command) -> io::Result<PathBuf> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let path = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    Ok(PathBuf::from(path.trim_end()))
}

#[test]
fn reader_staging_reuses_equal_bytes_and_separates_changed_bytes() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source");
    let cache = directory.path().join("cache");
    fs::write(&source, b"first reader")?;
    let first = stage_in(&source, &cache)?;
    let second = stage_in(&source, &cache)?;
    assert_eq!(first, second);
    assert_ne!(fs::metadata(&source)?.ino(), fs::metadata(&first)?.ino());
    assert_eq!(fs::read(&first)?, b"first reader");
    fs::write(&source, b"other reader")?;
    let changed = stage_in(&source, &cache)?;
    assert_ne!(first, changed);
    assert_eq!(fs::read(&first)?, b"first reader");
    assert_eq!(fs::read(&changed)?, b"other reader");
    Ok(())
}

#[test]
fn reader_staging_publishes_complete_bytes_for_concurrent_readers() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source");
    let cache = directory.path().join("cache");
    let content = vec![b'R'; 8192];
    fs::write(&source, &content)?;
    let published = std::thread::scope(|scope| {
        let first = scope.spawn(|| stage_in(&source, &cache));
        let second = scope.spawn(|| stage_in(&source, &cache));
        (first.join(), second.join())
    });
    let first = published.0.expect("the first publisher finishes")?;
    let second = published.1.expect("the second publisher finishes")?;
    assert_eq!(first, second);
    assert_eq!(fs::read(&first)?, content);
    assert_eq!(fs::read_dir(&cache)?.count(), 1);
    Ok(())
}

#[test]
fn reader_staging_removes_retired_owners_and_retains_a_live_owner() -> io::Result<()> {
    run_staging_case("retired-owners")
}

#[test]
fn reader_staging_publishes_the_hashed_bytes_after_the_source_changes() -> io::Result<()> {
    run_staging_case("changed-source")
}

fn run_staging_case(case: &str) -> io::Result<()> {
    let output = Command::new("python3")
        .arg("-S")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shim_registration/reader_staging_cases.py"
        ))
        .arg(case)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}
