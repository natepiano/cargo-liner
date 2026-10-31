use std::error::Error;

use super::CargoHome;
use super::RegisteredRoots;
use super::register_cargo_home_directory;
use super::replace_skip_set;
use crate::project::AbsolutePath;
use crate::watcher::skip::WatchSkipSet;
use crate::watcher::watch_test_support::RecordedFolderWatches;

#[test]
fn cargo_home_inside_target_gets_no_watch() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let cargo_home = scratch.path().join("target/cargo-home");
    std::fs::create_dir_all(&cargo_home)?;
    let mut watcher = RecordedFolderWatches::default();
    let mut registered = RegisteredRoots::default();
    register_cargo_home_directory(&mut watcher, &mut registered, &cargo_home);
    assert_eq!(watcher.watched, [] as [std::path::PathBuf; 0]);
    assert!(registered.watched.is_empty());
    Ok(())
}

#[test]
fn cargo_home_watch_is_tracked_and_pruned_when_target_becomes_known() -> Result<(), Box<dyn Error>>
{
    let scratch = tempfile::tempdir()?;
    let cargo_home = scratch
        .path()
        .canonicalize()?
        .join("build-cache/cargo-home");
    std::fs::create_dir_all(&cargo_home)?;
    let mut watcher = RecordedFolderWatches::default();
    let mut registered = RegisteredRoots::default();
    register_cargo_home_directory(&mut watcher, &mut registered, &cargo_home);
    assert!(registered.watched.contains_key(cargo_home.as_path()));
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(AbsolutePath::from(scratch.path().join("build-cache")));
    replace_skip_set(
        &mut watcher,
        &mut registered,
        skips,
        &CargoHome::Directory(cargo_home.clone()),
    );
    assert!(watcher.unwatched.contains(&cargo_home));
    assert!(!registered.watched.contains_key(cargo_home.as_path()));
    Ok(())
}

#[test]
fn skip_refresh_registers_only_the_supplied_cargo_home() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let cargo_home = scratch.path().join(".cargo");
    std::fs::create_dir_all(&cargo_home)?;
    let mut watcher = RecordedFolderWatches::default();
    let mut registered = RegisteredRoots::default();
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(AbsolutePath::from(scratch.path().join("build-cache")));

    replace_skip_set(
        &mut watcher,
        &mut registered,
        skips,
        &CargoHome::Directory(cargo_home.clone()),
    );

    assert_eq!(watcher.watched, vec![cargo_home]);
    Ok(())
}

#[cfg(unix)]
#[test]
fn cargo_home_alias_into_target_gets_no_watch() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let cargo_home = scratch.path().join("target/cargo-home");
    std::fs::create_dir_all(&cargo_home)?;
    let alias = scratch.path().join("cargo-home-link");
    std::os::unix::fs::symlink(&cargo_home, &alias)?;
    let mut watcher = RecordedFolderWatches::default();
    let mut registered = RegisteredRoots::default();
    register_cargo_home_directory(&mut watcher, &mut registered, &alias);
    assert_eq!(watcher.watched, [] as [std::path::PathBuf; 0]);
    Ok(())
}
