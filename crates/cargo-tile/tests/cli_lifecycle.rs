//! Exercise the installed binary without accessing a real rustup toolchain.

/// Each child owns an environment pointing exclusively at temporary fixture data.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use tempfile::TempDir;

    /// Preserve argument boundaries and record publication before the shim cleans up.
    const FIXTURE_CARGO: &str = r#"#!/bin/sh
set -eu
printf '%s\000' "$@" > "$LIFECYCLE_OBSERVATIONS/arguments"
printf '%s\000' "$HOME" "$RUSTUP_HOME" "$LIFECYCLE_CAPTURE_DIRECTORY" \
    > "$LIFECYCLE_OBSERVATIONS/environment"
if [ -n "${CARGOTILE_NESTED-}" ]; then
    cp -R "$LIFECYCLE_CAPTURE_DIRECTORY" "$LIFECYCLE_OBSERVATIONS/published"
fi
printf 'fixture cargo stdout\n'
printf 'fixture cargo stderr\n' >&2
exit 37
"#;

    /// This name sorts independently of whatever toolchains the host has installed.
    const TOOLCHAIN: &str = "fixture-stable";

    /// Bound subprocess waits even if a regression leaves a FIFO reader blocked.
    const CHILD_OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Keep all subprocess resources alive until synchronous commands and assertions finish.
    struct ToolchainLifecycle {
        /// The fixture owns every home, toolchain, capture, and observation directory.
        directory: TempDir,
        /// The executable path stays the same before, during, and after installation.
        cargo:     PathBuf,
    }

    impl ToolchainLifecycle {
        /// Include spaces in paths to exercise the installed shim's quoting.
        fn new() -> Self {
            let directory = tempfile::Builder::new()
                .prefix("cargo tile lifecycle ")
                .tempdir()
                .expect("create isolated lifecycle fixture");
            let cargo = directory
                .path()
                .join("rustup/toolchains")
                .join(TOOLCHAIN)
                .join("bin/cargo");
            fs::create_dir_all(cargo.parent().expect("toolchain bin path"))
                .expect("create fake toolchain bin directory");
            for path in [
                "home/.rustup/toolchains/ignored/bin",
                "observations",
                "config",
                "cargo-home",
            ] {
                fs::create_dir_all(directory.path().join(path)).expect("create fixture directory");
            }
            write_executable(&cargo);
            write_executable(
                &directory
                    .path()
                    .join("home/.rustup/toolchains/ignored/bin/cargo"),
            );
            Self { directory, cargo }
        }

        /// Clear inherited capture and cargo settings without changing the test runner itself.
        fn command(&self, executable: &Path) -> Command {
            let mut command = Command::new(executable);
            command
                .env_clear()
                .env(
                    "PATH",
                    env::var_os("PATH").expect("locate system shell utilities"),
                )
                .env("HOME", self.path("home"))
                .env("RUSTUP_HOME", self.path("rustup"))
                .env("CARGO_HOME", self.path("cargo-home"))
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("LIFECYCLE_CAPTURE_DIRECTORY", self.capture_directory())
                .env("LIFECYCLE_OBSERVATIONS", self.path("observations"))
                .current_dir(self.path("home"))
                .stdin(Stdio::null());
            command
        }

        /// Invoke the Cargo-provided executable path, never a binary found on PATH.
        fn cli(&self, arguments: &[&str]) -> String {
            let output = self
                .command(Path::new(env!("CARGO_BIN_EXE_cargo-tile")))
                .args(arguments)
                .output()
                .expect("run fixture-only cargo-tile command");
            assert!(output.status.success(), "{arguments:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{arguments:?}: {output:?}");
            String::from_utf8(output.stdout).expect("CLI output is UTF-8")
        }

        /// Keep fixture paths visibly separate from the process environment.
        fn path(&self, relative: &str) -> PathBuf { self.directory.path().join(relative) }

        /// Resolve the account layout without an application environment override.
        fn capture_directory(&self) -> PathBuf {
            let uid = fs::metadata(self.directory.path())
                .expect("fixture owner")
                .uid();
            self.path("capture").join(uid.to_string())
        }

        /// The fallback home toolchain must remain untouched when `RUSTUP_HOME` is set.
        fn assert_fallback_untouched(&self) {
            let bin = self.path("home/.rustup/toolchains/ignored/bin");
            assert_eq!(
                fs::read(bin.join("cargo")).expect("read fallback cargo"),
                FIXTURE_CARGO.as_bytes()
            );
            assert_eq!(
                fs::read_dir(bin)
                    .expect("inspect fallback toolchain")
                    .count(),
                1
            );
        }
    }

    /// A distinctive original mode detects replacement of cargo during restoration.
    fn write_executable(path: &Path) {
        fs::write(path, FIXTURE_CARGO).expect("write fake cargo executable");
        fs::set_permissions(path, fs::Permissions::from_mode(0o751))
            .expect("make fake cargo executable");
    }

    /// A rename preserves file identity, contents, and permissions at the new pathname.
    fn assert_original_cargo(path: &Path, original: &fs::Metadata) {
        let metadata = fs::metadata(path).expect("inspect original cargo at its current path");
        assert_eq!(
            fs::read(path).expect("read fake cargo"),
            FIXTURE_CARGO.as_bytes()
        );
        assert_eq!(metadata.dev(), original.dev());
        assert_eq!(metadata.ino(), original.ino());
        assert_eq!(metadata.permissions().mode(), original.permissions().mode());
    }

    /// Capture may not change cargo's failure status or either output stream.
    fn assert_cargo_output(output: &Output) {
        assert_eq!(output.status.code(), Some(37), "{output:?}");
        assert_eq!(output.stdout, b"fixture cargo stdout\n");
        assert_eq!(output.stderr, b"fixture cargo stderr\n");
    }

    /// Observe publication from inside cargo, then check cleanup after its shim exits.
    fn assert_shim_execution(fixture: &ToolchainLifecycle) {
        let source = fs::read_to_string(&fixture.cargo).expect("read installed shim");
        let assignment = "capture_parent=/tmp/cargo-tile";
        assert_eq!(source.lines().filter(|line| *line == assignment).count(), 1);
        let parent = fixture.path("capture");
        let parent = parent
            .to_str()
            .expect("UTF-8 parent")
            .replace('\'', "'\\''");
        fs::write(
            &fixture.cargo,
            source.replace(assignment, &format!("capture_parent='{parent}'")),
        )
        .expect("isolate the installed shim capture parent");
        let arguments = ["build", "--", "argument with spaces", "", "'quoted'", "雪"];
        let output = fixture
            .command(&fixture.cargo)
            .args(arguments)
            .output()
            .expect("execute the installed shim directly");
        assert_cargo_output(&output);
        let expected: Vec<u8> = arguments
            .iter()
            .flat_map(|argument| argument.bytes().chain(std::iter::once(0)))
            .collect();
        assert_eq!(
            fs::read(fixture.path("observations/arguments")).expect("read forwarded arguments"),
            expected
        );
        let expected = [
            fixture.path("home"),
            fixture.path("rustup"),
            fixture.capture_directory(),
        ];
        let expected: Vec<u8> = expected
            .iter()
            .flat_map(|path| {
                path.as_os_str()
                    .as_encoded_bytes()
                    .iter()
                    .copied()
                    .chain(std::iter::once(0))
            })
            .collect();
        assert_eq!(
            fs::read(fixture.path("observations/environment")).expect("read fixture environment"),
            expected
        );
        assert_eq!(
            fs::read_dir(fixture.path("observations/published/state/pids"))
                .expect("cargo observed a published registration")
                .count(),
            1
        );
        assert_eq!(
            fs::read_dir(fixture.capture_directory().join("state/pids"))
                .expect("inspect registrations after shim exit")
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(fixture.capture_directory())
                .expect("inspect capture root after shim exit")
                .count(),
            1,
            "only the state directory remains"
        );
    }

    /// CLI installation must publish a working shim and uninstall must restore the moved file.
    #[test]
    fn install_status_execution_and_uninstall_restore_the_original_cargo() {
        let fixture = ToolchainLifecycle::new();
        let original = fs::metadata(&fixture.cargo).expect("inspect original fake cargo");
        let saved = fixture.cargo.with_file_name("cargo-tile-real");
        assert_eq!(
            fixture.cli(&["status"]),
            format!("{TOOLCHAIN}: not installed\n")
        );
        assert!(!saved.exists());

        let installed = fixture.cli(&["install"]);
        assert!(
            installed
                .lines()
                .any(|line| line == format!("{TOOLCHAIN}: capture shim installed"))
        );
        assert_original_cargo(&saved, &original);
        assert_eq!(
            fs::read(&fixture.cargo).expect("read installed shim"),
            include_bytes!("../src/cargo-capture-shim.sh")
        );
        assert_eq!(
            fixture.cli(&["status"]),
            format!("{TOOLCHAIN}: capturing\n")
        );
        fixture.assert_fallback_untouched();
        assert_shim_execution(&fixture);

        assert_eq!(
            fixture.cli(&["tile", "uninstall"]),
            format!("{TOOLCHAIN}: capture shim removed\n")
        );
        assert_original_cargo(&fixture.cargo, &original);
        assert!(!saved.exists());
        assert_eq!(
            fixture.cli(&["tile", "status"]),
            format!("{TOOLCHAIN}: not installed\n")
        );
        assert_eq!(
            fs::read_dir(fixture.cargo.parent().expect("toolchain bin path"))
                .expect("inspect toolchain after uninstall")
                .count(),
            1,
            "no staging or lock file remains"
        );
        let output = fixture
            .command(&fixture.cargo)
            .arg("build")
            .output()
            .expect("execute restored original cargo");
        assert_cargo_output(&output);
        fixture.assert_fallback_untouched();
    }

    #[test]
    fn install_prints_a_completed_toolchain_before_a_later_lock_finishes() {
        assert_local_report_before_completion("install", "capture shim installed");
    }

    #[test]
    fn uninstall_prints_a_completed_toolchain_before_a_later_lock_finishes() {
        assert_local_report_before_completion("uninstall", "capture shim removed");
    }

    #[test]
    fn status_prints_a_completed_toolchain_before_a_later_inspection_finishes() {
        assert_local_report_before_completion("status", "not installed");
    }

    /// The stdout pipe must receive a row while the process still has unfinished work.
    fn assert_local_report_before_completion(operation: &str, description: &str) {
        let fixture = ToolchainLifecycle::new();
        let original = fs::metadata(&fixture.cargo).expect("original cargo identity");
        if operation == "uninstall" {
            fixture.cli(&["install"]);
        }
        let waiting = fixture.path("rustup/toolchains/z-waiting/bin");
        fs::create_dir_all(&waiting).expect("later toolchain directory");
        if operation == "status" {
            let output = Command::new("mkfifo")
                .arg(waiting.join("cargo"))
                .output()
                .expect("create blocked status inspection");
            assert!(output.status.success(), "{output:?}");
        } else {
            write_executable(&waiting.join("cargo"));
            fs::write(waiting.join("cargo-tile-shim.lock"), b"another writer")
                .expect("hold the later toolchain lock");
        }
        let mut command = fixture.command(Path::new(env!("CARGO_BIN_EXE_cargo-tile")));
        command.arg(operation);
        let output = terminate_after_first_row(&mut command);
        assert_eq!(output.status.signal(), Some(9), "{output:?}");
        assert_eq!(
            output.stdout,
            format!("{TOOLCHAIN}: {description}\n").as_bytes(),
            "emit only the completed toolchain before termination: {output:?}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        if operation == "install" {
            assert_original_cargo(&fixture.cargo.with_file_name("cargo-tile-real"), &original);
            assert_eq!(
                fs::read(&fixture.cargo).expect("completed installation"),
                include_bytes!("../src/cargo-capture-shim.sh")
            );
        } else {
            assert_original_cargo(&fixture.cargo, &original);
            assert!(!fixture.cargo.with_file_name("cargo-tile-real").exists());
        }
        assert!(!waiting.join("cargo-tile-real").exists());
        if operation != "status" {
            assert_eq!(
                fs::read(waiting.join("cargo")).expect("locked cargo remains untouched"),
                FIXTURE_CARGO.as_bytes()
            );
            assert_eq!(
                fs::read(waiting.join("cargo-tile-shim.lock")).expect("other writer's lock"),
                b"another writer"
            );
        }
        fixture.assert_fallback_untouched();
    }

    /// Kill and reap on both success and timeout, then join the owner of the stdout pipe.
    fn terminate_after_first_row(command: &mut Command) -> Output {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start CLI with unfinished later work");
        let stdout = child.stdout.take().expect("child stdout pipe");
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut bytes = Vec::new();
            let first = reader.read_until(b'\n', &mut bytes);
            sender.send(first).expect("report first-row readiness");
            reader.read_to_end(&mut bytes).expect("drain child stdout");
            bytes
        });
        let first = receiver.recv_timeout(CHILD_OUTPUT_TIMEOUT);
        child
            .kill()
            .expect("terminate CLI before later operation completes");
        let mut output = child.wait_with_output().expect("reap terminated CLI");
        output.stdout = reader.join().expect("join stdout reader");
        assert!(
            matches!(first, Ok(Ok(length)) if length > 0),
            "completed row never arrived: {first:?}; {output:?}"
        );
        output
    }

    /// A transient orphan owned by another writer must reach lock acquisition first.
    #[test]
    fn uninstall_waits_on_a_locked_temporary_orphan_before_classifying_it() {
        let fixture = ToolchainLifecycle::new();
        let original = fs::metadata(&fixture.cargo).expect("original cargo identity");
        fixture.cli(&["install"]);
        let saved = fixture.cargo.with_file_name("cargo-tile-real");
        let pending = fixture.cargo.with_file_name("pending-real-cargo");
        let lock = fixture.cargo.with_file_name("cargo-tile-shim.lock");
        fs::write(&lock, b"concurrent installer").expect("other installer holds lock");
        fs::rename(&saved, &pending).expect("other installer temporarily moves saved cargo");
        let output = fixture
            .command(Path::new(env!("CARGO_BIN_EXE_cargo-tile")))
            .arg("uninstall")
            .output()
            .expect("uninstall during concurrent installation");
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("cargo-tile-shim.lock"),
            "wait for the writer before inspecting its temporary state: {output:?}"
        );
        assert!(!stderr.contains("real cargo is missing"), "{output:?}");
        assert_original_cargo(&pending, &original);
        assert_eq!(
            fs::read(&lock).expect("other installer's lock survives"),
            b"concurrent installer"
        );
        fs::rename(&pending, &saved).expect("other installer restores saved cargo");
        fs::remove_file(&lock).expect("other installer releases lock");
        assert_eq!(
            fixture.cli(&["uninstall"]),
            format!("{TOOLCHAIN}: capture shim removed\n")
        );
        assert_original_cargo(&fixture.cargo, &original);
        assert!(!saved.exists());
    }

    /// Every administrative verb accepts the option and reaches its privilege check.
    #[test]
    fn all_accounts_install_refuses_non_root_with_sudo_instruction() {
        assert_admin_permission_refusal("install");
    }

    #[test]
    fn all_accounts_uninstall_refuses_non_root_with_sudo_instruction() {
        assert_admin_permission_refusal("uninstall");
    }

    #[test]
    fn all_accounts_status_refuses_non_root_with_sudo_instruction() {
        assert_admin_permission_refusal("status");
    }

    /// An unprivileged caller receives one actionable line before account enumeration.
    fn assert_admin_permission_refusal(operation: &str) {
        let fixture = ToolchainLifecycle::new();
        assert_ne!(
            fs::metadata(fixture.directory.path())
                .expect("fixture owner")
                .uid(),
            0,
            "this acceptance suite runs under one unprivileged uid"
        );
        let original = fs::read(&fixture.cargo).expect("original cargo");
        let output = fixture
            .command(Path::new(env!("CARGO_BIN_EXE_cargo-tile")))
            .args([operation, "--all-accounts"])
            .output()
            .expect("refuse non-root administrative command");
        assert!(!output.status.success(), "{output:?}");
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(message.lines().count(), 1, "{message}");
        assert!(message.contains("sudo"), "{message}");
        assert!(
            message.contains(&format!("{operation} --all-accounts")),
            "permission refusal names the requested operation: {message}"
        );
        assert!(!message.contains("unexpected argument"), "{message}");
        assert_eq!(
            fs::read(&fixture.cargo).expect("cargo after refusal"),
            original
        );
        assert!(!fixture.cargo.with_file_name("cargo-tile-real").exists());
        assert_eq!(
            fs::read_dir(fixture.cargo.parent().expect("toolchain bin"))
                .expect("inspect refused toolchain")
                .count(),
            1,
            "permission refusal creates no lock or staging file"
        );
        fixture.assert_fallback_untouched();
    }
}
