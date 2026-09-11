//! Native macOS ACLs must preserve the capture cleanup ownership boundary.

#![cfg(target_os = "macos")]

#[path = "../src/app.rs"]
mod app;
#[path = "../src/attract/mod.rs"]
mod attract;
#[path = "../src/birth_stamp/mod.rs"]
mod birth_stamp;
#[path = "../src/capture.rs"]
mod capture;
#[path = "../src/capture_root.rs"]
mod capture_root;
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
#[path = "../src/processes.rs"]
mod processes;
#[path = "../src/progress.rs"]
mod progress;
#[path = "../src/random.rs"]
mod random;
#[path = "../src/registration.rs"]
mod registration;
#[path = "../src/render.rs"]
mod render;
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
    use std::io;
    use std::io::ErrorKind;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use crate::capture_root::CleanupRefusal;
    use crate::capture_root::RootHistory;
    use crate::capture_root::RootOwner;
    use crate::capture_root::RootScan;
    use crate::capture_root::SweepBudget;
    use crate::capture_root::SweepDisposition;
    use crate::constants::CAPTURE_ACL_TEST_DIRECTORIES;
    use crate::constants::CAPTURE_ACL_TEST_WRITE_PERMISSIONS;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::constants::PERMISSION_BITS;
    use crate::settings::cleanup_refusal_for_test;

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
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .expect("owner-only mode bits");
            }
            fs::write(root.log(), "retained log").expect("fixture log");
            fs::write(root.registration(), "sampled registration").expect("fixture registration");
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
            assert!(
                scan.cleanup_refusals().is_empty(),
                "{:?}",
                scan.cleanup_refusals()
            );
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            assert!(!self.log().try_exists().expect("log existence"));
            assert!(
                !self
                    .registration()
                    .try_exists()
                    .expect("registration existence")
            );
        }

        fn assert_refuses(&self, scan: &RootScan, path: &Path) {
            let refusals = scan.cleanup_refusals();
            assert!(
                refusals.contains(&CleanupRefusal::AclWritableByOthers(path.to_owned())),
                "missing ACL refusal for {}: {refusals:?}",
                path.display()
            );
            assert!(
                !refusals.iter().any(|refusal| matches!(
                    refusal,
                    CleanupRefusal::Foreign(_) | CleanupRefusal::WritableByOthers(_)
                )),
                "ACL state must not replace ownership or mode observations: {refusals:?}"
            );
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
            self.assert_retained();
        }
    }

    #[test]
    fn every_non_owner_write_permission_on_root_refuses_cleanup() {
        assert_write_permissions_refused("");
    }

    #[test]
    fn every_non_owner_write_permission_on_state_refuses_cleanup() {
        assert_write_permissions_refused(CAPTURE_STATE_DIR);
    }

    #[test]
    fn every_non_owner_write_permission_on_pids_refuses_cleanup() {
        assert_write_permissions_refused(CAPTURE_LIVE_RUNS_DIR);
    }

    #[test]
    fn non_owner_write_entry_after_read_only_entry_still_refuses_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            chmod(&path, &["+a#", "0", "group:everyone allow list"]);
            chmod(&path, &["+a#", "1", "group:everyone allow add_file"]);
            root.assert_refuses(&root.scan(), &path);
        }
    }

    #[test]
    fn non_owner_write_added_after_open_refuses_cleanup_on_each_directory() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            assert!(scan.cleanup_refusals().is_empty());
            let path = root.path(relative);
            grant_write(&path, "add_file");
            root.assert_refuses(&scan, &path);
        }
    }

    #[test]
    fn non_owner_write_added_by_first_sweep_callback_prevents_both_unlinks() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                grant_write(&path, "add_file");
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(
                callbacks, 1,
                "the original cleanup gate must admit the scan"
            );
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
            root.assert_retained();
            root.assert_refuses(&scan, &path);
        }
    }

    #[test]
    fn non_owner_write_added_before_registration_unlink_preserves_registration() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                if callbacks == 2 {
                    grant_write(&path, "add_file");
                }
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(callbacks, 2);
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            assert!(!root.log().try_exists().expect("log existence"));
            assert_eq!(
                fs::read(root.registration()).expect("registration survives new ACL"),
                b"sampled registration"
            );
            let refusals = scan.cleanup_refusals();
            assert!(
                refusals.contains(&CleanupRefusal::AclWritableByOthers(path.clone())),
                "missing ACL refusal before registration unlink at {}: {refusals:?}",
                path.display()
            );
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
    fn acl_inspection_failure_names_each_directory_and_preserves_owner_and_files() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let mut scan = root.scan();
            let path = root.path(relative);
            scan.fail_acl_inspection_for_test(
                &path,
                io::Error::new(ErrorKind::PermissionDenied, "fixture ACL query failed"),
            )
            .expect("selected directory belongs to the scan");
            let refusals = scan.cleanup_refusals();
            let refusal = refusals
                .iter()
                .find(|refusal| {
                    matches!(
                        refusal,
                        CleanupRefusal::Access(failure)
                            if failure.path == path
                                && failure.failure.kind == ErrorKind::PermissionDenied
                                && failure.failure.message.contains("fixture ACL query failed")
                    )
                })
                .expect("ACL query failure retains its path and cause");
            let rendered = cleanup_refusal_for_test(refusal);
            assert!(
                rendered.contains(path.to_str().expect("fixture path is UTF-8")),
                "{rendered}"
            );
            assert!(rendered.contains("fixture ACL query failed"), "{rendered}");
            root.assert_owner(&scan);
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
            root.assert_retained();
        }
    }

    #[test]
    fn acl_refusal_rendering_names_each_directory_and_write_reason() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            grant_write(&path, "add_file");
            let scan = root.scan();
            let refusals = scan.cleanup_refusals();
            let refusal = refusals
                .iter()
                .find(|refusal| **refusal == CleanupRefusal::AclWritableByOthers(path.clone()))
                .expect("path-qualified ACL refusal");
            let rendered = cleanup_refusal_for_test(refusal);
            assert!(
                rendered.contains(path.to_str().expect("fixture path is UTF-8")),
                "{rendered}"
            );
            assert!(rendered.contains("ACL"), "{rendered}");
            assert!(rendered.contains("write"), "{rendered}");
        }
    }

    fn assert_write_permissions_refused(relative: &str) {
        for permission in CAPTURE_ACL_TEST_WRITE_PERMISSIONS {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            grant_write(&path, permission);
            root.assert_refuses(&root.scan(), &path);
        }
    }

    fn grant_write(path: &Path, permission: &str) {
        chmod(path, &["+a", &format!("group:everyone allow {permission}")]);
        assert_eq!(
            fs::metadata(path).expect("ACL directory metadata").mode() & PERMISSION_BITS,
            0o700,
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
