//! Directory ACL grants leave descriptor-based file ownership as the cleanup boundary.

#![cfg(target_os = "macos")]

#[path = "../src/app.rs"]
mod app;
#[path = "../src/attract/mod.rs"]
mod attract;
#[path = "../src/birth_stamp/mod.rs"]
mod birth_stamp;
#[path = "../src/capture.rs"]
mod capture;
#[path = "../src/census/mod.rs"]
mod census;
#[path = "../src/cli.rs"]
mod cli;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/constants.rs"]
mod constants;
#[path = "../src/favorites/mod.rs"]
mod favorites;
#[path = "../src/favorites_overlay/mod.rs"]
mod favorites_overlay;
#[path = "../src/globals.rs"]
mod globals;
#[path = "../src/hook.rs"]
mod hook;
#[path = "../src/interaction.rs"]
mod interaction;
#[path = "../src/iterm2.rs"]
mod iterm2;
#[path = "../src/keymap.rs"]
mod keymap;
#[path = "../src/navigation.rs"]
mod navigation;
#[path = "../src/probe.rs"]
mod probe;
#[path = "../src/progress/mod.rs"]
mod progress;
#[path = "../src/random.rs"]
mod random;
#[path = "../src/registration.rs"]
mod registration;
#[path = "../src/render.rs"]
mod render;
#[path = "../src/root_scan/mod.rs"]
mod root_scan;
#[path = "../src/roster.rs"]
mod roster;
#[path = "../src/sccache.rs"]
mod sccache;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/terminal.rs"]
mod terminal;
#[path = "../src/theme/mod.rs"]
mod theme;
#[path = "../src/tiles.rs"]
mod tiles;
#[path = "../src/wrap.rs"]
mod wrap;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;

    use rustix::fs::Mode;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use crate::constants::CAPTURE_ACL_TEST_DIRECTORIES;
    use crate::constants::CAPTURE_ACL_TEST_DIRECTORY_MODE;
    use crate::constants::CAPTURE_ACL_TEST_FILE_MODE;
    use crate::constants::CAPTURE_ACL_TEST_WRITE_PERMISSIONS;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::constants::PERMISSION_BITS;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootOwner;
    use crate::root_scan::RootScan;
    use crate::root_scan::SweepBudget;
    use crate::root_scan::SweepDisposition;

    /// One owner, one removable pair, and three directories with known permissions.
    struct CaptureRoot {
        directory: TempDir,
    }

    impl CaptureRoot {
        fn new() -> Self {
            let directory = tempdir().expect("temporary capture root");
            fs::create_dir_all(directory.path().join(CAPTURE_LIVE_RUNS_DIR))
                .expect("registration directories");
            let root = Self { directory };
            for relative in CAPTURE_ACL_TEST_DIRECTORIES {
                let path = root.path(relative);
                chmod(&path, &["-N"]);
                fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(CAPTURE_ACL_TEST_DIRECTORY_MODE),
                )
                .expect("owner-only mode bits");
            }
            fs::write(root.log(), "retained log").expect("fixture log");
            fs::write(root.registration(), "sampled registration").expect("fixture registration");
            for path in [root.log(), root.registration()] {
                fs::set_permissions(path, fs::Permissions::from_mode(CAPTURE_ACL_TEST_FILE_MODE))
                    .expect("owner-only file mode bits");
            }
            root
        }

        fn path(&self, relative: &str) -> PathBuf {
            if relative.is_empty() {
                self.directory.path().to_owned()
            } else {
                self.directory.path().join(relative)
            }
        }

        fn log(&self) -> PathBuf { self.path("log") }

        fn registration(&self) -> PathBuf { self.path(CAPTURE_LIVE_RUNS_DIR).join("record") }

        fn scan(&self) -> RootScan {
            RootScan::open(self.directory.path(), &mut RootHistory::default())
                .expect("open capture directories")
        }

        fn assert_owner(&self, scan: &RootScan) {
            let owner = fs::metadata(self.directory.path())
                .expect("fixture root metadata")
                .uid();
            assert_eq!(scan.owner(), RootOwner::Uid(owner));
        }

        fn assert_retained(&self) {
            assert_eq!(fs::read(self.log()).expect("retained log"), b"retained log");
            assert_eq!(
                fs::read(self.registration()).expect("retained registration"),
                b"sampled registration"
            );
        }

        fn assert_sweeps(&self, scan: &RootScan) {
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            let counts = scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(counts.removed_files(), 2);
            assert_eq!(counts.skipped_pair_attempts(), 0);
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            self.assert_removed();
        }

        fn assert_removed(&self) {
            assert!(!self.log().try_exists().expect("log existence"));
            assert!(
                !self
                    .registration()
                    .try_exists()
                    .expect("registration existence")
            );
        }

        fn assert_skips(&self, scan: &RootScan) {
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            let counts = scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(counts.removed_files(), 0);
            assert!(
                counts.skipped_pair_attempts() > 0,
                "unproved candidates are counted"
            );
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            self.assert_retained();
        }
    }

    #[test]
    fn every_non_owner_write_permission_on_root_allows_owned_file_cleanup() {
        assert_write_permissions_sweep("");
    }

    #[test]
    fn every_non_owner_write_permission_on_state_allows_owned_file_cleanup() {
        assert_write_permissions_sweep(CAPTURE_STATE_DIR);
    }

    #[test]
    fn every_non_owner_write_permission_on_pids_allows_owned_file_cleanup() {
        assert_write_permissions_sweep(CAPTURE_LIVE_RUNS_DIR);
    }

    #[test]
    fn non_owner_write_entry_after_read_only_entry_allows_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            chmod(&path, &["+a#", "0", "group:everyone allow list"]);
            chmod(&path, &["+a#", "1", "group:everyone allow add_file"]);
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn non_owner_write_added_after_open_allows_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            grant_write(&path, "add_file");
            root.assert_sweeps(&scan);
        }
    }

    #[test]
    fn non_owner_write_added_by_first_sweep_callback_allows_both_unlinks() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                if callbacks == 1 {
                    grant_write(&path, "add_file");
                }
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert!(callbacks >= 2, "identity is rechecked before each unlink");
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            root.assert_removed();
        }
    }

    #[test]
    fn non_owner_write_added_before_registration_unlink_allows_both_unlinks() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut grants_after_log_removal = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                if !root
                    .log()
                    .try_exists()
                    .expect("log existence during callback")
                {
                    grants_after_log_removal += 1;
                    grant_write(&path, "add_file");
                }
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert!(callbacks >= 2);
            assert!(grants_after_log_removal > 0);
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            root.assert_removed();
            root.assert_owner(&scan);
        }
    }

    #[test]
    fn directories_without_acls_still_sweep() {
        let root = CaptureRoot::new();
        root.assert_sweeps(&root.scan());
    }

    #[test]
    fn directories_with_explicitly_emptied_acls_still_sweep() {
        let root = CaptureRoot::new();
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let path = root.path(relative);
            grant_write(&path, "add_file");
            // Removing the sole entry writes a zero-entry ACL; -N removes the ACL itself.
            chmod(&path, &["-a#", "0"]);
        }
        root.assert_sweeps(&root.scan());
    }

    #[test]
    fn read_only_non_owner_acl_on_each_directory_still_sweeps() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            chmod(
                &root.path(relative),
                &[
                    "+a",
                    "group:everyone allow list,search,readattr,readextattr,readsecurity",
                ],
            );
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn read_only_non_owner_acl_added_after_open_still_sweeps() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            chmod(
                &root.path(relative),
                &["+a", "group:everyone allow list,search"],
            );
            root.assert_sweeps(&scan);
        }
    }

    #[test]
    fn owner_write_acl_on_each_directory_still_sweeps() {
        let output = Command::new("/usr/bin/id")
            .arg("-un")
            .output()
            .expect("effective owner name");
        assert!(output.status.success(), "{output:?}");
        let owner = String::from_utf8(output.stdout).expect("owner name is UTF-8");
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let entry = format!(
                "user:{}:allow:{}",
                owner.trim(),
                CAPTURE_ACL_TEST_WRITE_PERMISSIONS.join(",")
            );
            chmod(&root.path(relative), &["+a", &entry]);
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn non_owner_deny_entry_is_not_a_write_grant() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            chmod(
                &root.path(relative),
                &["+a", "group:everyone deny add_file"],
            );
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn foreign_registration_in_acl_writable_directory_preserves_pair() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            grant_write(&root.path(relative), "add_file");
            let mut scan = root.scan();
            // Change only the uid comparison; file metadata and directory ownership stay real.
            scan.mark_file_foreign_for_test(&root.registration())
                .expect("inject foreign registration owner");
            root.assert_skips(&scan);
        }
    }

    #[test]
    fn foreign_log_in_acl_writable_directory_preserves_pair() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            grant_write(&root.path(relative), "add_file");
            let mut scan = root.scan();
            // Change only the uid comparison; file metadata and directory ownership stay real.
            scan.mark_file_foreign_for_test(&root.log())
                .expect("inject foreign log owner");
            root.assert_skips(&scan);
        }
    }

    #[test]
    fn writable_directory_mode_bits_allow_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            for write in [Mode::WGRP, Mode::WOTH] {
                let root = CaptureRoot::new();
                fs::set_permissions(
                    root.path(relative),
                    fs::Permissions::from_mode(
                        CAPTURE_ACL_TEST_DIRECTORY_MODE | u32::from(write.bits()),
                    ),
                )
                .expect("directory permits non-owner writes");
                root.assert_sweeps(&root.scan());
            }
        }
    }

    #[test]
    fn writable_file_mode_bits_preserve_pair_in_acl_writable_directory() {
        for write in [Mode::WGRP, Mode::WOTH] {
            for candidate in [CaptureRoot::log, CaptureRoot::registration] {
                let root = CaptureRoot::new();
                grant_write(&root.path(""), "add_file");
                fs::set_permissions(
                    candidate(&root),
                    fs::Permissions::from_mode(
                        CAPTURE_ACL_TEST_FILE_MODE | u32::from(write.bits()),
                    ),
                )
                .expect("candidate permits non-owner writes");
                root.assert_skips(&root.scan());
            }
        }
    }

    #[test]
    fn multiply_linked_file_preserves_pair_in_acl_writable_directory() {
        for candidate in [CaptureRoot::log, CaptureRoot::registration] {
            let root = CaptureRoot::new();
            grant_write(&root.path(""), "add_file");
            let alias = root.path("alias");
            fs::hard_link(candidate(&root), &alias).expect("second link to candidate");
            let contents = fs::read(&alias).expect("linked contents before sweep");
            root.assert_skips(&root.scan());
            assert_eq!(
                fs::read(alias).expect("linked contents after sweep"),
                contents
            );
        }
    }

    #[test]
    fn symlinked_file_preserves_target_in_acl_writable_directory() {
        for candidate in [CaptureRoot::log, CaptureRoot::registration] {
            let root = CaptureRoot::new();
            grant_write(&root.path(""), "add_file");
            let path = candidate(&root);
            let target = root.path("target");
            fs::rename(&path, &target).expect("move candidate to symlink target");
            symlink(&target, &path).expect("candidate symlink");
            let contents = fs::read(&target).expect("target contents before sweep");
            root.assert_skips(&root.scan());
            assert!(
                fs::symlink_metadata(path)
                    .expect("retained symlink")
                    .is_symlink()
            );
            assert_eq!(
                fs::read(target).expect("target contents after sweep"),
                contents
            );
        }
    }

    #[test]
    fn registration_replaced_before_its_unlink_survives_in_acl_writable_directory() {
        let root = CaptureRoot::new();
        grant_write(&root.path(CAPTURE_LIVE_RUNS_DIR), "add_file");
        let scan = root.scan();
        let mut callbacks = 0;
        let mut budget = SweepBudget::default();
        scan.sweep(&mut budget, |_| {
            callbacks += 1;
            if !root
                .log()
                .try_exists()
                .expect("log existence during callback")
            {
                fs::rename(root.registration(), root.path("held-record"))
                    .expect("retain sampled inode separately");
                fs::write(root.registration(), "replacement registration")
                    .expect("replace registration before unlink");
                fs::set_permissions(
                    root.registration(),
                    fs::Permissions::from_mode(CAPTURE_ACL_TEST_FILE_MODE),
                )
                .expect("replacement independently satisfies file mode rule");
            }
            SweepDisposition::Remove(PathBuf::from("log"))
        });
        assert!(callbacks >= 2);
        assert!(
            !root
                .log()
                .try_exists()
                .expect("log removed before replacement")
        );
        assert_eq!(
            fs::read(root.registration()).expect("replacement registration survives"),
            b"replacement registration"
        );
        assert_eq!(
            fs::read(root.path("held-record")).expect("sampled inode survives"),
            b"sampled registration"
        );
    }

    fn assert_write_permissions_sweep(relative: &str) {
        for permission in CAPTURE_ACL_TEST_WRITE_PERMISSIONS {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            grant_write(&path, permission);
            root.assert_sweeps(&root.scan());
        }
    }

    fn grant_write(path: &Path, permission: &str) {
        chmod(path, &["+a", &format!("group:everyone allow {permission}")]);
        assert_eq!(
            fs::metadata(path).expect("ACL directory metadata").mode() & PERMISSION_BITS,
            CAPTURE_ACL_TEST_DIRECTORY_MODE,
            "the ACL fixture must not rely on group or other write mode bits: {} ({permission})",
            path.display()
        );
    }

    fn chmod(path: &Path, arguments: &[&str]) {
        let output = Command::new("/bin/chmod")
            .args(arguments)
            .arg(path)
            .output()
            .expect("run native ACL fixture command");
        assert!(
            output.status.success(),
            "chmod {arguments:?} {}: {output:?}",
            path.display()
        );
    }
}
