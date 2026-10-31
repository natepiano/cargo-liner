#![cfg(unix)]

use std::error::Error;
use std::os::unix::fs::symlink;

use notify::Event;
use notify::EventKind;
use notify::event::CreateKind;

use super::WatchSkipSet;
use crate::project::AbsolutePath;
use crate::watcher::roots;
use crate::watcher::roots::CargoHome;
use crate::watcher::roots::WatchRegistration;
use crate::watcher::watch_test_support::RecordedFolderWatches;

#[test]
fn custom_target_symlink_skips_real_tree_at_registration_and_creation() -> Result<(), Box<dyn Error>>
{
    let scratch = tempfile::tempdir()?;
    let root = AbsolutePath::from(scratch.path().canonicalize()?);
    let real_target = root.join("build-real");
    std::fs::create_dir_all(real_target.join("debug/deps"))?;
    std::fs::create_dir(root.join("src"))?;
    let alias = root.join("build-link");
    symlink(&real_target, &alias)?;
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(AbsolutePath::from(alias.clone()));
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut watcher,
        std::slice::from_ref(&root),
        skips,
        WatchRegistration::PerFolder,
    );
    assert!(failures.is_empty());
    assert!(watcher.watched.contains(&root.join("src")));
    assert!(
        watcher
            .watched
            .iter()
            .all(|path| !path.starts_with(&real_target) && !path.starts_with(&alias))
    );

    let created = real_target.join("new/nested");
    std::fs::create_dir_all(&created)?;
    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &Event::new(EventKind::Create(CreateKind::Folder)).add_path(created.clone()),
    );
    assert!(!watcher.watched.contains(&created));
    Ok(())
}

#[test]
fn late_custom_target_symlink_prunes_real_tree_watches() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let root = AbsolutePath::from(scratch.path().canonicalize()?);
    let real_target = root.join("build-real");
    std::fs::create_dir_all(real_target.join("debug"))?;
    let alias = root.join("build-link");
    symlink(&real_target, &alias)?;
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_watch_roots(&mut watcher, &[root]);
    assert!(failures.is_empty());
    assert!(watcher.watched.contains(&real_target));
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(AbsolutePath::from(alias));
    roots::replace_skip_set(
        &mut watcher,
        &mut registered,
        skips,
        &CargoHome::Unavailable,
    );
    assert!(watcher.unwatched.contains(&real_target));
    assert!(watcher.unwatched.contains(&real_target.join("debug")));
    Ok(())
}
