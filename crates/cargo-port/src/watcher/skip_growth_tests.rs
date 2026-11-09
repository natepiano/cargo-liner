use std::fs;

use super::CargoHome;
use super::RegisteredRoots;
use super::WatchSkipSet;
use super::replace_skip_set;
use crate::project::AbsolutePath;
use crate::watcher::watch_test_support::RecordedFolderWatches;

#[test]
fn growing_target_skips_do_not_rewalk_unrelated_project_folders() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = AbsolutePath::from(directory.path());
    fs::create_dir(root.join("src"))?;
    let mut registered = RegisteredRoots::from_dirs(vec![root.clone()]);
    let mut watcher = RecordedFolderWatches::default();
    // This folder's create event has not been delivered. A target update must
    // not enumerate unrelated folders to discover it.
    fs::create_dir(root.join("unrelated"))?;
    let mut skips = WatchSkipSet::default();
    for index in 0..128 {
        skips.add_target_directory(AbsolutePath::from(root.join(format!("output-{index}"))));
        replace_skip_set(
            &mut watcher,
            &mut registered,
            skips.clone(),
            &CargoHome::Unavailable,
        );
    }
    assert!(
        watcher.watched.is_empty(),
        "target-only updates must not walk project folders"
    );
    assert!(registered.covers(root.as_path()));
    Ok(())
}

#[test]
fn removing_a_target_skip_restores_its_folder_watches() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = AbsolutePath::from(directory.path());
    let output = root.join("output");
    fs::create_dir(&output)?;
    fs::create_dir(output.join("child"))?;
    let mut registered = RegisteredRoots::from_dirs(vec![root]);
    let mut watcher = RecordedFolderWatches::default();
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(AbsolutePath::from(output.clone()));
    replace_skip_set(
        &mut watcher,
        &mut registered,
        skips,
        &CargoHome::Unavailable,
    );
    assert!(watcher.unwatched.contains(&output));
    replace_skip_set(
        &mut watcher,
        &mut registered,
        WatchSkipSet::default(),
        &CargoHome::Unavailable,
    );
    assert!(watcher.watched.contains(&output));
    assert!(watcher.watched.contains(&output.join("child")));
    Ok(())
}
