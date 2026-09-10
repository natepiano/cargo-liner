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
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;
    use std::process::Stdio;

    use tempfile::TempDir;

    /// Preserve argument boundaries and record publication before the shim cleans up.
    const FIXTURE_CARGO: &str = r#"#!/bin/sh
set -eu
printf '%s\000' "$@" > "$LIFECYCLE_OBSERVATIONS/arguments"
printf '%s\000' "$HOME" "$RUSTUP_HOME" "$CARGO_TILE_ROOT" \
    > "$LIFECYCLE_OBSERVATIONS/environment"
if [ -n "${CARGOTILE_NESTED-}" ]; then
    cp -R "$CARGO_TILE_ROOT" "$LIFECYCLE_OBSERVATIONS/published"
fi
printf 'fixture cargo stdout\n'
printf 'fixture cargo stderr\n' >&2
exit 37
"#;

    /// This name sorts independently of whatever toolchains the host has installed.
    const TOOLCHAIN: &str = "fixture-stable";

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
                .env("CARGO_TILE_ROOT", self.path("capture"))
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
            fixture.path("capture"),
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
            fs::read_dir(fixture.path("capture/state/pids"))
                .expect("inspect registrations after shim exit")
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(fixture.path("capture"))
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
}
