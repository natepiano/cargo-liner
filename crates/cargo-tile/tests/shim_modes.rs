//! Exercise capture permissions through an isolated installed toolchain.

/// Keep filesystem and environment changes confined to each child invocation.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;
    use std::process::Stdio;

    use tempfile::TempDir;

    /// Select the stream conditions that make the shim choose its capture path.
    #[derive(Clone, Copy, Debug)]
    enum CapturePath {
        /// The system `script` command supplies all three terminal streams.
        Pty,
        /// Piped output and null input select stderr mirroring through a FIFO.
        NoTerminal,
    }

    /// Keep the deployment-root decision independent of its directory mode.
    #[derive(Clone, Copy)]
    enum RootSelection {
        /// Exercise the built-in fallback after redirecting it inside the fixture.
        Default,
        /// An empty override also selects the built-in fallback.
        Empty,
        /// A deployment selects the root through `CARGO_TILE_ROOT`.
        Explicit,
    }

    /// Isolate an absent home from the test runner's own environment.
    #[derive(Clone, Copy)]
    enum HomeSelection {
        /// Give the child a home directory inside the fixture.
        Present,
        /// Preserve a caller value that cannot shorten an absolute working directory.
        Relative(&'static str),
        /// Remove `HOME` only from the child process.
        Unset,
    }

    /// Own the installed shim, cargo stand-in, and observations until assertions end.
    struct InstalledToolchain {
        /// All subprocess writes, including the default capture root, stay here.
        directory:    TempDir,
        /// The selected capture directory is independent of the real `/tmp` root.
        root:         PathBuf,
        /// Cargo saves artifact permissions before the shim's exit trap removes them.
        observations: PathBuf,
        /// The launcher sets the caller's umask without modifying the test process.
        launcher:     PathBuf,
    }

    impl InstalledToolchain {
        /// Copy the shim beside a recording cargo executable, as installation does.
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("create isolated toolchain");
            let root = directory.path().join("capture");
            let observations = directory.path().join("observations");
            let bin = directory.path().join("toolchain/bin");
            let launcher = directory.path().join("invoke-shim");
            fs::create_dir_all(&bin).expect("create toolchain bin directory");
            fs::create_dir(&observations).expect("create cargo observations directory");
            fs::create_dir(directory.path().join("home")).expect("create isolated HOME");
            let source = include_str!("../src/cargo-capture-shim.sh");
            assert!(source.contains("/tmp/cargo-tile"), "locate built-in root");
            // Only redirect the fallback pathname in the fixture copy. The root
            // selection and permission code still sees the requested override state.
            let source = source.replace("/tmp/cargo-tile", "${SHIM_TEST_DEFAULT_ROOT}");
            write_executable(&bin.join("cargo"), &source);
            write_executable(
                &bin.join("cargo-tile-real"),
                r#"#!/bin/sh
set -eu
umask > "$SHIM_TEST_OBSERVATIONS/umask"
printf '%s\000' "$@" > "$SHIM_TEST_OBSERVATIONS/arguments"
printf '%s\000' "$(pwd -P)" "${HOME-}" > "$SHIM_TEST_OBSERVATIONS/directory-fields"
printf '%s\000' "${CARGOTILE_NESTED-unset}" \
    "${CARGO_TERM_PROGRESS_WHEN-unset}" "${CARGO_TERM_PROGRESS_WIDTH-unset}" \
    > "$SHIM_TEST_OBSERVATIONS/environment"
if [ -t 0 ] && [ -t 1 ] && [ -t 2 ]; then
    printf '%s' pty > "$SHIM_TEST_OBSERVATIONS/streams"
else
    printf '%s' pipes > "$SHIM_TEST_OBSERVATIONS/streams"
fi
true > "$SHIM_TEST_OBSERVATIONS/cargo-output"
if [ -d "$SHIM_TEST_DEFAULT_ROOT" ]; then
    cp -pR "$SHIM_TEST_DEFAULT_ROOT" "$SHIM_TEST_OBSERVATIONS/root"
fi
printf '%s\n' cargo-stdout
printf '%s\n' cargo-stderr >&2
exit "$SHIM_TEST_EXIT_STATUS"
"#,
            );
            write_executable(
                &launcher,
                "#!/bin/sh\numask \"$SHIM_TEST_CALLER_UMASK\"\nexec sh \"$SHIM_TEST_SHIM\" \"$@\"\n",
            );
            Self {
                directory,
                root,
                observations,
                launcher,
            }
        }

        /// Run cargo synchronously so no subprocess outlives its temporary files.
        fn run(
            &self,
            capture_path: CapturePath,
            root_selection: RootSelection,
            home_selection: HomeSelection,
            caller_umask: u32,
            arguments: &[&str],
        ) -> Output {
            let mut command = self.command(capture_path, arguments);
            let search_path = env::var_os("PATH").expect("fixture inherits system utilities");
            let search_path = env::join_paths(
                std::iter::once(self.directory.path().join("toolchain/bin"))
                    .chain(env::split_paths(&search_path)),
            )
            .expect("prepend fixture utilities to PATH");
            command
                .current_dir(self.directory.path().join("home"))
                .env("SHIM_TEST_DEFAULT_ROOT", &self.root)
                .env("SHIM_TEST_OBSERVATIONS", &self.observations)
                .env(
                    "SHIM_TEST_SHIM",
                    self.directory.path().join("toolchain/bin/cargo"),
                )
                .env("SHIM_TEST_CALLER_UMASK", format!("{caller_umask:04o}"))
                .env("SHIM_TEST_EXIT_STATUS", "37")
                .env("PATH", search_path)
                .env("POSIXLY_CORRECT", "1")
                .env("SHELL", "/bin/sh")
                .env_remove("CARGOTILE_NESTED")
                .env_remove("CARGO_TERM_PROGRESS_WHEN")
                .env_remove("CARGO_TERM_PROGRESS_WIDTH")
                .stdin(Stdio::null());
            match root_selection {
                RootSelection::Default => command.env_remove("CARGO_TILE_ROOT"),
                RootSelection::Empty => command.env("CARGO_TILE_ROOT", ""),
                RootSelection::Explicit => command.env("CARGO_TILE_ROOT", &self.root),
            };
            match home_selection {
                HomeSelection::Present => command.env("HOME", self.directory.path().join("home")),
                HomeSelection::Relative(home) => command.env("HOME", home),
                HomeSelection::Unset => command.env_remove("HOME"),
            };
            command.output().expect("run installed shim to completion")
        }

        /// Use the host's real terminal recorder, accepting both supported interfaces.
        fn command(&self, capture_path: CapturePath, arguments: &[&str]) -> Command {
            match capture_path {
                CapturePath::NoTerminal => {
                    let mut command = Command::new("sh");
                    command.arg(&self.launcher).args(arguments);
                    command
                },
                CapturePath::Pty => {
                    let version = Command::new("script")
                        .arg("--version")
                        .output()
                        .expect("system script is required for terminal capture tests");
                    let mut command = Command::new("script");
                    if String::from_utf8_lossy(&version.stdout).contains("util-linux") {
                        let launcher = self.launcher.to_str().expect("UTF-8 fixture path");
                        let invocation = ["sh", launcher]
                            .into_iter()
                            .chain(arguments.iter().copied())
                            .map(shell_word)
                            .collect::<Vec<_>>()
                            .join(" ");
                        command.args(["-q", "-e", "-c", &invocation, "/dev/null"]);
                    } else {
                        command
                            .args(["-q", "/dev/null", "sh"])
                            .arg(&self.launcher)
                            .args(arguments);
                    }
                    command
                },
            }
        }

        /// Assert observations from cargo itself, including a file cargo creates.
        fn assert_cargo(&self, output: &Output, caller_umask: u32, arguments: &[&str]) {
            assert_eq!(
                output.status.code(),
                Some(37),
                "cargo status changed: {output:?}"
            );
            let observed = fs::read_to_string(self.observations.join("umask"))
                .expect("cargo records its inherited umask");
            assert_eq!(
                u32::from_str_radix(observed.trim(), 8).expect("numeric shell umask"),
                caller_umask,
                "capture permissions must not reach cargo"
            );
            assert_mode(
                &self.observations.join("cargo-output"),
                0o666 & !caller_umask,
            );
            let expected: Vec<u8> = arguments
                .iter()
                .flat_map(|argument| argument.bytes().chain(std::iter::once(0)))
                .collect();
            assert_eq!(
                fs::read(self.observations.join("arguments")).expect("cargo records argv"),
                expected,
                "cargo must receive every original argument verbatim"
            );
        }

        /// Inspect copies whose metadata was preserved while the registration was live.
        fn assert_capture_modes(&self, capture_path: CapturePath) {
            let snapshot = self.observations.join("root");
            assert_mode(&snapshot.join("state"), 0o750);
            assert_mode(&snapshot.join("state/pids"), 0o750);
            let registrations = entries(&snapshot.join("state/pids"));
            assert_eq!(registrations.len(), 1, "exactly one published registration");
            assert_mode(&registrations[0], 0o640);
            let registration = fs::read(&registrations[0]).expect("read extended registration");
            let terminated = registration
                .strip_suffix(&[0])
                .expect("NUL-terminated record");
            let fields: Vec<_> = terminated.split(|byte| *byte == 0).collect();
            assert_eq!(fields[0], b"cargo-tile-v2");
            let directory = fs::read(self.observations.join("directory-fields"))
                .expect("cargo records its original directory and home");
            let directory = directory
                .strip_suffix(&[0])
                .expect("terminated directory fields");
            let directory: Vec<_> = directory.split(|byte| *byte == 0).collect();
            assert_eq!(fields[5], directory[0]);
            let home = Path::new(std::str::from_utf8(directory[1]).expect("UTF-8 fixture home"));
            let expected_home = if home.is_absolute() {
                directory[1]
            } else {
                b""
            };
            assert_eq!(fields[6], expected_home);
            assert!(
                Path::new(std::str::from_utf8(fields[5]).expect("UTF-8 fixture cwd")).is_absolute()
            );
            let logs: Vec<_> = entries(&snapshot)
                .into_iter()
                .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
                .collect();
            assert_eq!(logs.len(), 1, "cargo observes its pre-created log");
            assert_mode(&logs[0], 0o640);
            let fifos: Vec<_> = entries(&snapshot.join("state"))
                .into_iter()
                .filter(|path| {
                    fs::metadata(path)
                        .expect("artifact metadata")
                        .file_type()
                        .is_fifo()
                })
                .collect();
            let streams = fs::read_to_string(self.observations.join("streams"))
                .expect("cargo records terminal streams");
            match capture_path {
                CapturePath::Pty => {
                    assert_eq!(
                        streams, "pty",
                        "cargo must run in the terminal capture path"
                    );
                    assert!(
                        fifos.is_empty(),
                        "terminal capture must not use the stderr FIFO"
                    );
                },
                CapturePath::NoTerminal => {
                    assert_eq!(streams, "pipes");
                    assert_eq!(fifos.len(), 1, "stderr capture must use a FIFO");
                    assert_mode(&fifos[0], 0o640);
                },
            }
            assert!(
                entries(&self.root.join("state/pids")).is_empty(),
                "registration cleanup"
            );
            assert_eq!(entries(&self.root.join("state")).len(), 1, "FIFO cleanup");
            assert_eq!(entries(&self.root).len(), 1, "log cleanup");
        }
    }

    /// Set fixture permissions explicitly, without changing the parent process's umask.
    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write fixture executable");
        set_mode(path, 0o755);
    }

    /// Single-quote argv words for util-linux `script` without interpreting their contents.
    fn shell_word(word: &str) -> String { format!("'{}'", word.replace('\'', "'\\''")) }

    /// Read directory entries only where the scenario requires a real capture directory.
    fn entries(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .expect("read capture directory")
            .map(|entry| entry.expect("read capture entry").path())
            .collect()
    }

    /// Compare Unix mode bits without file-type bits from `st_mode`.
    fn assert_mode(path: &Path, expected: u32) {
        let actual = fs::metadata(path)
            .expect("read observed artifact mode")
            .permissions()
            .mode();
        assert_eq!(actual & 0o7777, expected, "mode of {}", path.display());
    }

    /// Arrange an existing directory's exact permissions independently of the runner.
    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set fixture mode");
    }

    /// Exercise each caller umask with an ordinary invocation whose arguments stay unchanged.
    fn assert_umask(capture_path: CapturePath, caller_umask: u32) {
        let toolchain = InstalledToolchain::new();
        let arguments = ["run", "--", "a b", "single'quote", "tab\tand\nnewline", ""];
        let output = toolchain.run(
            capture_path,
            RootSelection::Explicit,
            HomeSelection::Present,
            caller_umask,
            &arguments,
        );
        toolchain.assert_cargo(&output, caller_umask, &arguments);
        toolchain.assert_capture_modes(capture_path);
        assert_mode(&toolchain.root, 0o750);
    }

    /// Delete orphan logs synchronously before publication, then inspect the writer's mode.
    /// This models a sweep at ln, not an earlier pid snapshot used after publication.
    fn assert_log_mode_after_sweep(capture_path: CapturePath) {
        let toolchain = InstalledToolchain::new();
        let system_commands = Command::new("sh")
            .args(["-c", "command -v ln; command -v tee"])
            .output()
            .expect("locate capture utilities before installing fixture wrappers");
        assert!(system_commands.status.success());
        let system_commands =
            String::from_utf8(system_commands.stdout).expect("system utility paths are UTF-8");
        let mut commands = system_commands.lines();
        let link = shell_word(commands.next().expect("system ln path"));
        let tee = shell_word(commands.next().expect("system tee path"));
        write_executable(
            &toolchain.directory.path().join("toolchain/bin/ln"),
            &format!(
                r#"#!/bin/sh
set -eu
[ ! -e "$2" ]
for log in "$SHIM_TEST_DEFAULT_ROOT"/run-*.log; do
    [ -f "$log" ] || continue
    rm -f "$log"
done
printf 'swept before publication\n' > "$SHIM_TEST_OBSERVATIONS/sweep"
exec {link} "$@"
"#
            ),
        );
        // Snapshot after tee drains stderr, while the shim still owns its log.
        // This also covers recreation happening after cargo's earlier snapshot.
        write_executable(
            &toolchain.directory.path().join("toolchain/bin/tee"),
            &format!(
                r#"#!/bin/sh
set -eu
{tee} "$@"
for log in "$SHIM_TEST_DEFAULT_ROOT"/run-*.log; do
    [ -f "$log" ] || continue
    cp -p "$log" "$SHIM_TEST_OBSERVATIONS/written-log"
done
"#
            ),
        );
        let output = toolchain.run(
            capture_path,
            RootSelection::Explicit,
            HomeSelection::Present,
            0o066,
            &["build"],
        );
        toolchain.assert_cargo(&output, 0o066, &["build"]);
        assert_eq!(
            fs::read(toolchain.observations.join("sweep"))
                .expect("the sweep runs before the real ln publishes"),
            b"swept before publication\n"
        );
        if matches!(capture_path, CapturePath::NoTerminal) {
            let log = toolchain.observations.join("written-log");
            assert_eq!(
                fs::read(&log).expect("tee captures cargo stderr"),
                b"cargo-stderr\n"
            );
            assert_mode(&log, 0o640);
            assert_eq!(output.stdout, b"cargo-stdout\n");
            assert_eq!(output.stderr, b"cargo-stderr\n");
        }
        toolchain.assert_capture_modes(capture_path);
    }

    /// F001: script must retain group-readable output after a sweep before publication.
    #[test]
    fn pty_log_stays_group_readable_after_publication_boundary_sweep() {
        assert_log_mode_after_sweep(CapturePath::Pty);
    }

    /// F001: tee must retain group-readable output after a sweep before publication.
    #[test]
    fn no_terminal_log_stays_group_readable_after_publication_boundary_sweep() {
        assert_log_mode_after_sweep(CapturePath::NoTerminal);
    }

    /// The additional identity fields are readable by the group before publication too.
    #[test]
    fn extended_registration_is_group_readable_before_publication() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            let toolchain = InstalledToolchain::new();
            let link = Command::new("sh")
                .args(["-c", "command -v ln"])
                .output()
                .expect("locate native publication utility");
            assert!(link.status.success());
            let link = String::from_utf8(link.stdout).expect("UTF-8 utility path");
            write_executable(
                &toolchain.directory.path().join("toolchain/bin/ln"),
                &format!(
                    r#"#!/bin/sh
set -eu
cp -p "$1" "$SHIM_TEST_OBSERVATIONS/staged-registration"
exec {} "$@"
"#,
                    shell_word(link.trim_end())
                ),
            );
            let output = toolchain.run(
                capture_path,
                RootSelection::Explicit,
                HomeSelection::Present,
                0o066,
                &["build"],
            );
            toolchain.assert_cargo(&output, 0o066, &["build"]);
            toolchain.assert_capture_modes(capture_path);
            let staged = toolchain.observations.join("staged-registration");
            assert_mode(&staged, 0o640);
            let published = entries(&toolchain.observations.join("root/state/pids"));
            assert_eq!(
                fs::read(staged).expect("complete staging record"),
                fs::read(&published[0]).expect("published record")
            );
        }
    }

    /// Without a unique generation, capture must fail before publishing a repeatable name.
    #[test]
    fn unavailable_uuid_sources_run_cargo_with_original_permissions_and_environment() {
        let toolchain = InstalledToolchain::new();
        let cat = Command::new("sh")
            .args(["-c", "command -v cat"])
            .output()
            .expect("locate native cat for unrelated fixture reads");
        assert!(cat.status.success());
        let cat = String::from_utf8(cat.stdout).expect("UTF-8 utility path");
        write_executable(
            &toolchain.directory.path().join("toolchain/bin/cat"),
            &format!(
                r#"#!/bin/sh
case "$1" in /proc/sys/kernel/random/uuid) exit 1 ;; esac
exec {} "$@"
"#,
                shell_word(cat.trim_end())
            ),
        );
        write_executable(
            &toolchain.directory.path().join("toolchain/bin/uuidgen"),
            "#!/bin/sh\nexit 1\n",
        );
        let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
        let output = toolchain.run(
            CapturePath::NoTerminal,
            RootSelection::Explicit,
            HomeSelection::Present,
            0o066,
            &arguments,
        );
        toolchain.assert_cargo(&output, 0o066, &arguments);
        assert_eq!(output.stdout, b"cargo-stdout\n");
        assert_eq!(output.stderr, b"cargo-stderr\n");
        assert_eq!(
            fs::read(toolchain.observations.join("environment")).expect("cargo capture settings"),
            b"unset\0unset\0unset\0"
        );
        assert!(
            !toolchain.root.exists(),
            "no repeatable capture artifacts are published"
        );
    }

    /// A desktop caller keeps its mask when cargo runs in a terminal.
    #[test]
    fn pty_keeps_caller_umask_0022() { assert_umask(CapturePath::Pty, 0o022); }

    /// The runner mask must not prevent group-readable terminal capture.
    #[test]
    fn pty_keeps_caller_umask_0066() { assert_umask(CapturePath::Pty, 0o066); }

    /// An owner-only caller still gets group-readable terminal capture artifacts.
    #[test]
    fn pty_keeps_caller_umask_0077() { assert_umask(CapturePath::Pty, 0o077); }

    /// Stderr capture preserves the caller mask while narrowing capture artifacts.
    #[test]
    fn no_terminal_keeps_caller_umask_0022() { assert_umask(CapturePath::NoTerminal, 0o022); }

    /// Stderr capture exposes runner artifacts to its group without changing cargo.
    #[test]
    fn no_terminal_keeps_caller_umask_0066() { assert_umask(CapturePath::NoTerminal, 0o066); }

    /// An owner-only caller keeps that policy for its own files under stderr capture.
    #[test]
    fn no_terminal_keeps_caller_umask_0077() { assert_umask(CapturePath::NoTerminal, 0o077); }

    /// Upgrading a runner hierarchy must repair directories as well as files.
    #[test]
    fn existing_0711_hierarchy_gains_group_access_without_changing_cargo_umask() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            let toolchain = InstalledToolchain::new();
            fs::create_dir_all(toolchain.root.join("state/pids")).expect("create old hierarchy");
            for path in [
                &toolchain.root,
                &toolchain.root.join("state"),
                &toolchain.root.join("state/pids"),
            ] {
                set_mode(path, 0o711);
            }
            let output = toolchain.run(
                capture_path,
                RootSelection::Explicit,
                HomeSelection::Present,
                0o066,
                &["build"],
            );
            toolchain.assert_cargo(&output, 0o066, &["build"]);
            toolchain.assert_capture_modes(capture_path);
            assert_mode(&toolchain.root, 0o711);
        }
    }

    /// The fallback root protects desktop captures from other users on either path.
    #[test]
    fn default_root_is_created_owner_only() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            for root_selection in [RootSelection::Default, RootSelection::Empty] {
                let toolchain = InstalledToolchain::new();
                let output = toolchain.run(
                    capture_path,
                    root_selection,
                    HomeSelection::Present,
                    0o022,
                    &["build"],
                );
                toolchain.assert_cargo(&output, 0o022, &["build"]);
                toolchain.assert_capture_modes(capture_path);
                assert_mode(&toolchain.root, 0o700);
                assert_mode(&toolchain.observations.join("root"), 0o700);
            }
        }
    }

    /// Root and state corrections preserve permissions of unrelated descendants.
    #[test]
    fn default_root_correction_leaves_unrelated_descendants_alone() {
        for root_selection in [RootSelection::Default, RootSelection::Empty] {
            let toolchain = InstalledToolchain::new();
            let unrelated = toolchain.root.join("state/unrelated/nested");
            fs::create_dir_all(&unrelated).expect("create unrelated directory");
            set_mode(&toolchain.root, 0o755);
            set_mode(&unrelated, 0o711);
            let sentinel = unrelated.join("sentinel");
            fs::write(&sentinel, "keep these permissions").expect("create unrelated file");
            set_mode(&sentinel, 0o600);
            let output = toolchain.run(
                CapturePath::NoTerminal,
                root_selection,
                HomeSelection::Present,
                0o066,
                &["build"],
            );
            toolchain.assert_cargo(&output, 0o066, &["build"]);
            assert_mode(&toolchain.root, 0o700);
            assert_mode(&toolchain.observations.join("root"), 0o700);
            assert_mode(&toolchain.root.join("state"), 0o750);
            assert_mode(&toolchain.root.join("state/pids"), 0o750);
            assert_mode(&unrelated, 0o711);
            assert_mode(&sentinel, 0o600);
        }
    }

    /// An explicit root retains the deployment permission policy.
    #[test]
    fn explicit_root_keeps_the_deployments_mode() {
        for root_mode in [0o700, 0o750, 0o755] {
            let toolchain = InstalledToolchain::new();
            fs::create_dir(&toolchain.root).expect("create deployment root");
            set_mode(&toolchain.root, root_mode);
            let output = toolchain.run(
                CapturePath::NoTerminal,
                RootSelection::Explicit,
                HomeSelection::Present,
                0o066,
                &["build"],
            );
            toolchain.assert_cargo(&output, 0o066, &["build"]);
            toolchain.assert_capture_modes(CapturePath::NoTerminal);
            assert_mode(&toolchain.root, root_mode);
            assert_mode(&toolchain.observations.join("root"), root_mode);
        }
    }

    /// Failing the first directory write still runs cargo without capture settings.
    #[test]
    fn unwritable_root_runs_original_cargo_and_returns_its_status() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            let toolchain = InstalledToolchain::new();
            fs::create_dir(&toolchain.root).expect("create unwritable root");
            set_mode(&toolchain.root, 0o500);
            let probe = toolchain.root.join("write-probe");
            let denied =
                fs::write(&probe, "must fail").expect_err("test requires an unprivileged uid");
            assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
            let arguments = [
                "check",
                "--quiet",
                "--message-format=json",
                "--",
                "a b",
                "",
                "-q",
            ];
            let output = toolchain.run(
                capture_path,
                RootSelection::Explicit,
                HomeSelection::Present,
                0o066,
                &arguments,
            );
            set_mode(&toolchain.root, 0o700);
            toolchain.assert_cargo(&output, 0o066, &arguments);
            assert_eq!(
                fs::read(toolchain.observations.join("environment"))
                    .expect("cargo records environment"),
                b"unset\0unset\0unset\0",
                "failed setup must not export capture settings"
            );
            assert!(
                entries(&toolchain.root).is_empty(),
                "failed setup leaves no artifacts"
            );
            if matches!(capture_path, CapturePath::NoTerminal) {
                assert_eq!(output.stdout, b"cargo-stdout\n");
                assert_eq!(output.stderr, b"cargo-stderr\n");
            }
        }
    }

    /// Writable staging must not hide a fatal log-redirection regression.
    #[test]
    fn denied_log_creation_does_not_exit_the_posix_shell_before_cargo() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            let toolchain = InstalledToolchain::new();
            fs::create_dir_all(toolchain.root.join("state/pids"))
                .expect("registration staging remains writable");
            set_mode(&toolchain.root, 0o500);
            let denied = fs::write(toolchain.root.join("write-probe"), "must fail")
                .expect_err("test requires an unprivileged uid");
            assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
            let arguments = ["check", "--quiet", "--message-format=json", "--", "a b"];
            let output = toolchain.run(
                capture_path,
                RootSelection::Explicit,
                HomeSelection::Present,
                0o077,
                &arguments,
            );
            set_mode(&toolchain.root, 0o700);
            toolchain.assert_cargo(&output, 0o077, &arguments);
            assert_eq!(
                fs::read(toolchain.observations.join("environment"))
                    .expect("cargo records environment"),
                b"unset\0unset\0unset\0",
                "log setup failure must reach uncaptured cargo"
            );
            assert!(
                entries(&toolchain.root.join("state/pids")).is_empty(),
                "staging cleanup"
            );
            assert_eq!(
                entries(&toolchain.root).len(),
                1,
                "only the pre-existing state directory remains"
            );
        }
    }

    /// FIFO failure must precede argument rewriting and capture exports.
    #[test]
    fn fifo_failure_preserves_original_json_arguments_and_capture_environment() {
        let toolchain = InstalledToolchain::new();
        write_executable(
            &toolchain.directory.path().join("toolchain/bin/mkfifo"),
            "#!/bin/sh\nexit 1\n",
        );
        let arguments = [
            "check",
            "--quiet",
            "-q",
            "--message-format=json",
            "--",
            "a b",
            "-q",
        ];
        let output = toolchain.run(
            CapturePath::NoTerminal,
            RootSelection::Explicit,
            HomeSelection::Present,
            0o066,
            &arguments,
        );
        toolchain.assert_cargo(&output, 0o066, &arguments);
        assert_eq!(
            fs::read(toolchain.observations.join("environment"))
                .expect("cargo records environment"),
            b"unset\0unset\0unset\0",
            "FIFO setup failure must precede capture exports"
        );
        assert!(
            entries(&toolchain.root.join("state/pids")).is_empty(),
            "registration cleanup"
        );
        assert_eq!(
            entries(&toolchain.root.join("state")).len(),
            1,
            "FIFO cleanup"
        );
        assert_eq!(entries(&toolchain.root).len(), 1, "log cleanup");
        assert_eq!(output.stdout, b"cargo-stdout\n");
        assert_eq!(output.stderr, b"cargo-stderr\n");
    }

    /// Relative and numeric homes cannot be interpreted as an absolute prefix or argc.
    #[test]
    fn relative_home_is_omitted_from_registration_but_preserved_for_cargo() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            for home in ["relative/home", "1"] {
                let toolchain = InstalledToolchain::new();
                let output = toolchain.run(
                    capture_path,
                    RootSelection::Explicit,
                    HomeSelection::Relative(home),
                    0o066,
                    &["build"],
                );
                toolchain.assert_cargo(&output, 0o066, &["build"]);
                toolchain.assert_capture_modes(capture_path);
                let inherited = fs::read(toolchain.observations.join("directory-fields"))
                    .expect("cargo records its unchanged HOME");
                assert_eq!(
                    inherited.split(|byte| *byte == 0).nth(1),
                    Some(home.as_bytes())
                );
                let registrations = entries(&toolchain.observations.join("root/state/pids"));
                let registration = fs::read(&registrations[0]).expect("extended registration");
                let fields: Vec<_> = registration
                    .strip_suffix(&[0])
                    .expect("terminated record")
                    .split(|byte| *byte == 0)
                    .collect();
                assert_eq!(&fields[6..], [b"".as_slice(), b"1", b"build"]);
            }
        }
    }

    /// Accounts without a home environment still reach cargo and publish capture.
    #[test]
    fn unset_home_does_not_abort_either_capture_path() {
        for capture_path in [CapturePath::Pty, CapturePath::NoTerminal] {
            for root_selection in [
                RootSelection::Default,
                RootSelection::Empty,
                RootSelection::Explicit,
            ] {
                let toolchain = InstalledToolchain::new();
                let output = toolchain.run(
                    capture_path,
                    root_selection,
                    HomeSelection::Unset,
                    0o077,
                    &["build"],
                );
                toolchain.assert_cargo(&output, 0o077, &["build"]);
                toolchain.assert_capture_modes(capture_path);
            }
        }
    }
}
