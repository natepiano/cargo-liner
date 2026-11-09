//! Shim publication wire contracts and their isolated executable fixture.

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::OnceLock;

use tempfile::TempDir;

use super::parallel_cases;
use super::python_cache;
use crate::birth_stamp;
use crate::constants::REGISTRATION_MAGIC;
use crate::hook;
use crate::hook::CaptureTestPublication;

/// Whether the fixture verifies the PID of each native birth query.
enum NativeBirthVerification {
    Unnecessary,
    Required,
}

/// Share the system utility paths across isolated shim fixtures.
struct SystemUtilities {
    ln:     PathBuf,
    date:   PathBuf,
    ps:     PathBuf,
    sysctl: PathBuf,
}

/// Own every path the copied shim and stand-in cargo can write.
struct InstalledShim {
    /// Keep the toolchain, capture root, and observations alive through assertions.
    directory:                 TempDir,
    native_birth_verification: NativeBirthVerification,
    utilities:                 &'static SystemUtilities,
}

impl InstalledShim {
    /// Install the repository script beside cargo that snapshots its live registration.
    fn new(native_birth_verification: NativeBirthVerification) -> Self {
        static UTILITIES: OnceLock<SystemUtilities> = OnceLock::new();
        let utilities = UTILITIES.get_or_init(|| {
            let utilities = Command::new("sh")
                .args([
                    "-c",
                    "command -v ln; command -v date; command -v ps; command -v sysctl || true",
                ])
                .output()
                .expect("locate system utilities before changing the child PATH");
            assert!(utilities.status.success());
            let utilities = String::from_utf8(utilities.stdout).expect("utility paths are UTF-8");
            let mut utilities = utilities.lines();
            SystemUtilities {
                ln:     PathBuf::from(utilities.next().expect("system ln path")),
                date:   PathBuf::from(utilities.next().expect("system date path")),
                ps:     PathBuf::from(utilities.next().expect("system ps path")),
                sysctl: PathBuf::from(utilities.next().unwrap_or("sysctl")),
            }
        });
        let fixture = Self {
            directory: tempfile::tempdir().expect("create isolated toolchain"),
            native_birth_verification,
            utilities,
        };
        for directory in [
            "bin",
            "tools",
            "observations",
            "home/work tree\twith\nlines",
        ] {
            fs::create_dir_all(fixture.path(directory)).expect("create fixture directory");
        }
        let source = hook::shim_source();
        let assignment = "capture_parent=/tmp/cargo-tile";
        assert_eq!(source.lines().filter(|line| *line == assignment).count(), 1);
        let parent = fixture
            .directory
            .path()
            .canonicalize()
            .expect("physical fixture root")
            .join("capture");
        let parent = parent
            .to_str()
            .expect("UTF-8 parent")
            .replace('\'', "'\\''");
        executable(
            &fixture.path("bin/cargo"),
            &source.replace(assignment, &format!("capture_parent='{parent}'")),
        );
        install_cargo(&fixture.path("bin/cargo-tile-real"));
        install_publication_observer(&fixture.path("tools/ln"));
        fixture
    }

    /// Resolve fixture paths without depending on the developer's home or capture root.
    fn path(&self, relative: &str) -> PathBuf {
        let directory = self
            .directory
            .path()
            .canonicalize()
            .expect("physical fixture root");
        Path::new(relative).strip_prefix("capture").map_or_else(
            |_| directory.join(relative),
            |suffix| {
                let uid = fs::metadata(&directory).expect("fixture owner").uid();
                let account = directory.join("capture").join(uid.to_string());
                if suffix.as_os_str().is_empty() {
                    account
                } else {
                    account.join(suffix)
                }
            },
        )
    }

    /// Let `InstalledShim` use native `tools/ln` while retaining both live log observations.
    fn repeat_in_same_shell(&self) {
        fs::write(self.path("observations/repeat"), b"").expect("repeat in one shell");
        for invocation in ["first", "second"] {
            fs::create_dir(self.path(&format!("observations/{invocation}")))
                .expect("create the repeated invocation's observations");
        }
        let publication = self.path("tools/ln");
        fs::remove_file(&publication).expect("remove the unused publication observer");
        symlink(&self.utilities.ln, publication).expect("retain native exclusive publication");
    }

    /// Run with an independently recorded shim pid and a restrictive caller umask.
    fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments)
            .output()
            .expect("execute copied shim with stand-in cargo")
    }

    /// Run while keeping this invocation's observations separate from earlier runs.
    fn run_recorded_in(&self, observations: &str, arguments: &[&str]) -> Output {
        self.command_recorded_in(observations, arguments)
            .output()
            .expect("execute copied shim with stand-in cargo")
    }

    /// Run an argument table through one publisher process with separate stream observations.
    fn run_cases(&self, cases: &[(&str, &[&str])]) -> Vec<Output> {
        let mut commands = cases
            .iter()
            .map(|(observations, arguments)| self.command_recorded_in(observations, arguments));
        let mut command = commands.next().expect("an argument table has cases");
        // Prepare every case's argument file before the publisher starts.
        commands.for_each(drop);
        let schedule: Vec<_> = cases
            .iter()
            .flat_map(|(observations, _)| {
                let mut path = self.path(observations).as_os_str().as_bytes().to_vec();
                path.push(0);
                path
            })
            .collect();
        let schedule_path = self.path("observations/cases");
        fs::write(&schedule_path, schedule).expect("write argument table schedule");
        let completed = command
            .env("SHIM_NATIVE_TEST_CASES", schedule_path)
            .output()
            .expect("execute argument table in one publisher process");
        cases
            .iter()
            .enumerate()
            .map(|(index, (observations, _))| {
                let observations = self.path(observations);
                let status = fs::read_to_string(observations.join("exit-status"))
                    .expect("record each invocation status")
                    .parse::<i32>()
                    .expect("numeric invocation status");
                Output {
                    status: if index + 1 == cases.len() {
                        completed.status
                    } else {
                        std::process::ExitStatus::from_raw(status << 8)
                    },
                    stdout: fs::read(observations.join("stdout"))
                        .expect("record each invocation stdout"),
                    stderr: fs::read(observations.join("stderr"))
                        .expect("record each invocation stderr"),
                }
            })
            .collect()
    }

    fn observe_live_log(&self) {
        fs::write(self.path("observations/log-timeout"), b"0.2")
            .expect("set the live log handshake deadline");
    }

    fn assert_live_log(&self, observations: &str) {
        let registration = self.registration_at(observations);
        let fields = registration.fields();
        let log = std::str::from_utf8(fields[4]).expect("UTF-8 log name");
        assert_eq!(
            fs::read(self.path(observations).join(log)).expect("live capture log"),
            b"registration log marker\n"
        );
    }

    /// Allow tests to change only the child environment and invocation schedule.
    fn command(&self, arguments: &[&str]) -> Command {
        self.command_recorded_in("observations", arguments)
    }

    /// Build a command whose files do not replace another invocation's observations.
    fn command_recorded_in(&self, observations: &str, arguments: &[&str]) -> Command {
        let observations = self.path(observations);
        fs::create_dir_all(&observations).expect("create invocation observations");
        let mut search_path = vec![self.path("tools")];
        search_path.extend(std::env::split_paths(
            &std::env::var_os("PATH").expect("test runner supplies PATH"),
        ));
        let argument_file = observations.join("invocation-arguments");
        let mut argument_bytes = Vec::new();
        for argument in arguments {
            argument_bytes.extend_from_slice(argument.as_bytes());
            argument_bytes.push(0);
        }
        fs::write(&argument_file, argument_bytes).expect("fixture arguments");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                r#"umask 0066
printf '%s' "$$" > "$SHIM_TEST_OBSERVATIONS/shim-pid"
exec "$@" 9>&1 >/dev/null"#,
                "native-publication-fixture",
            ])
            .arg(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "shim_registration::installed::publication_links_a_complete_tmp_file_into_place",
                "--nocapture",
            ])
            .env("SHIM_NATIVE_TEST_CALL", self.path("bin/cargo"))
            .env("SHIM_NATIVE_TEST_ARGUMENTS", argument_file)
            .current_dir(self.path("home/work tree\twith\nlines"))
            .env("HOME", self.path("home"))
            .env("SHIM_TEST_ACCOUNT_DIRECTORY", self.path("capture"))
            .env("SHIM_TEST_OBSERVATIONS", observations)
            .env(
                "SHIM_TEST_NATIVE_BIRTH_VERIFICATION",
                match self.native_birth_verification {
                    NativeBirthVerification::Unnecessary => "unnecessary",
                    NativeBirthVerification::Required => "required",
                },
            )
            .env("SHIM_TEST_REAL_DATE", &self.utilities.date)
            .env("SHIM_TEST_REAL_PS", &self.utilities.ps)
            .env("SHIM_TEST_REAL_SYSCTL", &self.utilities.sysctl)
            .env("SHIM_TEST_GENERATION", "20260909-204000")
            .env("SHIM_TEST_NATIVE_PLATFORM", std::env::consts::OS)
            .env_remove("SHIM_TEST_PENDING_DELETE")
            .env(
                "PATH",
                std::env::join_paths(search_path).expect("join child PATH"),
            )
            .env_remove("CARGOTILE_NESTED")
            .env_remove("CARGO_TERM_PROGRESS_WHEN")
            .env_remove("CARGO_TERM_PROGRESS_WIDTH")
            .stdin(Stdio::null());
        python_cache::cache_python_modules(&mut command);
        command
            .env(
                "SHIM_TEST_LOG_OBSERVER",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/shim_registration/capture_log_observer.py"
                ),
            )
            .env(
                "SHIM_TEST_LOG_TIMEOUT_FILE",
                self.path("observations/log-timeout"),
            );
        command
    }

    /// Select the sole v2 registration copied while cargo was alive.
    fn registration(&self) -> PublishedRegistration { self.registration_at("observations") }

    /// Read each invocation separately when two runs deliberately share a shell pid.
    fn registration_at(&self, observations: &str) -> PublishedRegistration {
        let entries: Vec<_> = fs::read_dir(self.path(observations).join("pids"))
            .expect("cargo snapshots the live registration directory")
            .map(|entry| entry.expect("read observed registration"))
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.contains('.') && !name.ends_with(".tmp")
            })
            .collect();
        assert_eq!(entries.len(), 1, "exactly one published v2 registration");
        PublishedRegistration {
            name:     entries[0]
                .file_name()
                .into_string()
                .expect("registration filename is ASCII"),
            contents: fs::read(entries[0].path()).expect("read registration bytes"),
        }
    }

    /// Keep collision assertions independent of the publisher's random generation.
    fn staged_registration(&self) -> PublishedRegistration {
        let publication = fs::read(self.path("observations/publication-arguments"))
            .expect("observe attempted publication");
        let arguments = nul_fields(&publication);
        let path = Path::new(std::str::from_utf8(arguments[1]).expect("publication path"));
        PublishedRegistration {
            name:     path
                .file_name()
                .expect("registration basename")
                .to_str()
                .expect("ASCII registration name")
                .to_owned(),
            contents: fs::read(self.path("observations/staged-registration"))
                .expect("observe complete staging bytes"),
        }
    }
}

/// Preserve the externally observed name and bytes for protocol assertions.
struct PublishedRegistration {
    /// Include the generation so tests catch publication under a bare pid.
    name:     String,
    /// Keep raw bytes until checking termination and argument framing.
    contents: Vec<u8>,
}

impl PublishedRegistration {
    /// Reject missing terminators without discarding empty argument fields.
    fn fields(&self) -> Vec<&[u8]> { nul_fields(&self.contents) }
}

/// A stand-in cargo saves observations before the shim's exit trap removes them.
fn install_cargo(path: &Path) { executable(path, include_str!("capture_log_observer.py")); }

/// Observe staging before forwarding publication to the real exclusive-create primitive.
fn install_publication_observer(path: &Path) {
    executable(
        path,
        r#"#!/bin/sh
set -eu
printf '%s\000' "$@" > "$SHIM_TEST_OBSERVATIONS/publication-arguments"
cp "$1" "$SHIM_TEST_OBSERVATIONS/staged-registration"
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-at-publication" ] || [ -f "$SHIM_TEST_OBSERVATIONS/occupy-name" ]; then
    printf 'previous registration\n' > "$2"
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-log" ]; then
    name=${2##*/}
    printf 'previous log\n' > "$SHIM_TEST_ACCOUNT_DIRECTORY/run-${name#*.}-${name%%.*}.log"
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-directory" ]; then
    mkdir "$2"
    printf 'existing directory\n' > "$2/keep"
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/signal-after-publication" ]; then
    "$SHIM_TEST_REAL_LN" "$@" || exit $?
    case $(cat "$SHIM_TEST_OBSERVATIONS/signal-after-publication") in
        shim) process=$(cat "$SHIM_TEST_OBSERVATIONS/shim-pid") ;;
        setup) process=$(ps -o ppid= -p "$$" | tr -d ' ') ;;
    esac
    kill -TERM "$process"
    exit 0
fi
exec "$SHIM_TEST_REAL_LN" "$@"
"#,
    );
}

/// Give each stand-in executable permissions independently of the runner's umask.
fn executable(path: &Path, source: &str) {
    fs::write(path, source).expect("write fixture executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fixture executable");
}

/// Remove only the framing terminator; a preceding empty field is still an argument.
fn nul_fields(bytes: &[u8]) -> Vec<&[u8]> {
    assert_eq!(bytes.last(), Some(&0), "every field ends in NUL");
    bytes[..bytes.len() - 1].split(|byte| *byte == 0).collect()
}

/// Require the shim to preserve cargo's status and both output streams.
fn assert_cargo_result(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(37),
        "cargo must run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"cargo stdout\n");
    assert_eq!(output.stderr, b"registration log marker\n");
}

/// Compare against the live shim's proc record independently of the writer's parser.
#[cfg(target_os = "linux")]
fn observed_birth(fixture: &InstalledShim) -> String {
    let stat = fs::read_to_string(fixture.path("observations/shim-stat"))
        .expect("cargo reads the live shim's stat");
    let (_, fields) = stat.rsplit_once(") ").expect("stat ends its comm field");
    fields
        .split_whitespace()
        .nth(19)
        .expect("stat contains field 22 after pid and comm")
        .to_owned()
}

/// Compare the registration with ps while the registered shim is still alive.
#[cfg(target_os = "macos")]
fn observed_birth(fixture: &InstalledShim) -> String {
    fs::read_to_string(fixture.path("observations/birth"))
        .expect("cargo reads the live shim's start time")
        .trim_end_matches('\n')
        .to_owned()
}

/// Tabs, newlines, quotes, Unicode, and empty words cannot change the field boundaries.
#[test]
fn versioned_fields_preserve_original_argument_bytes() {
    let fixture = InstalledShim::new(NativeBirthVerification::Required);
    let arguments = ["run", "--", "a b", "a\tb", "a\nb", "'\"\\", "雪", ""];
    assert_cargo_result(&fixture.run(&arguments));
    let registration = fixture.registration();
    let fields = registration.fields();
    let boot = fs::read_to_string(fixture.path("observations/boot")).expect("read boot identity");
    let birth = observed_birth(&fixture);
    let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
    let generation = std::str::from_utf8(fields[1]).expect("ASCII generation");
    assert!(generation.starts_with("20260909-204000-"));
    let log = format!("run-{generation}-{pid}.log");
    let count = arguments.len().to_string();
    let directory = fixture.path("home/work tree\twith\nlines");
    let home = fixture.path("home");
    assert_eq!(
        &fields[..8],
        [
            REGISTRATION_MAGIC,
            generation.as_bytes(),
            boot.trim_end_matches('\n').as_bytes(),
            birth.as_bytes(),
            log.as_bytes(),
            directory.as_os_str().as_encoded_bytes(),
            home.as_os_str().as_encoded_bytes(),
            count.as_bytes(),
        ]
    );
    assert_eq!(
        &fields[8..],
        arguments
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
    let observed = fs::read(fixture.path("observations/arguments")).expect("read cargo argv");
    assert_eq!(nul_fields(&observed), &fields[8..]);
}

/// Check the executed argv separately from the original argv published by the shim.
fn assert_executed_arguments(
    fixture: &InstalledShim,
    observations: &str,
    arguments: &[&str],
    executed: &[&str],
) {
    let output = fixture.run_recorded_in(observations, arguments);
    assert_completed_arguments(fixture, observations, arguments, executed, &output);
}

/// Keep the same argument assertions for an individually recorded table invocation.
fn assert_completed_arguments(
    fixture: &InstalledShim,
    observations: &str,
    arguments: &[&str],
    executed: &[&str],
    output: &Output,
) {
    assert_cargo_result(output);
    let observed =
        fs::read(fixture.path(&format!("{observations}/arguments"))).expect("read executed argv");
    assert_eq!(
        nul_fields(&observed),
        executed
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
    let registration = fixture.registration_at(observations);
    assert_eq!(
        &registration.fields()[8..],
        arguments
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
}

#[test]
fn short_quiet_json_removes_repeated_quiet_only_before_separator() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    assert_executed_arguments(
        &fixture,
        "observations",
        &[
            "check",
            "probe-json",
            "-q",
            "--message-format=json",
            "-q",
            "--",
            "--quiet",
            "-q",
        ],
        &[
            "check",
            "probe-json",
            "--message-format=json",
            "--",
            "--quiet",
            "-q",
        ],
    );
}

#[test]
fn separate_json_format_removes_repeated_quiet_only_before_separator() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    assert_executed_arguments(
        &fixture,
        "observations",
        &[
            "check",
            "probe-json",
            "-q",
            "--message-format",
            "json",
            "-q",
            "--",
            "--quiet",
            "-q",
        ],
        &[
            "check",
            "probe-json",
            "--message-format",
            "json",
            "--",
            "--quiet",
            "-q",
        ],
    );
}

#[test]
fn non_json_arguments_preserve_both_quiet_spellings() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fixture.observe_live_log();
    let cases: [(&str, &[&str]); 3] = [
        ("observations/plain", &["check", "probe-mismatch"]),
        ("observations/long", &["check", "probe-mismatch", "--quiet"]),
        ("observations/short", &["check", "probe-mismatch", "-q"]),
    ];
    for ((observations, arguments), output) in cases.iter().zip(fixture.run_cases(&cases)) {
        assert_completed_arguments(&fixture, observations, arguments, arguments, &output);
        fixture.assert_live_log(observations);
    }
}

#[test]
fn capture_ignores_tee_on_path_and_writes_the_live_log_while_cargo_runs() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fixture.observe_live_log();
    executable(
        &fixture.path("tools/tee"),
        r"#!/usr/bin/env python3
import sys

data = sys.stdin.buffer.read()
assert sys.argv[1] == '-a'
for name in sys.argv[2:]:
    with open(name, 'ab') as log:
        log.write(data)
sys.stdout.buffer.write(data)
",
    );
    let output = fixture.run(&["check", "probe-mismatch", "--quiet"]);
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout, b"cargo stdout\n");
    assert_eq!(output.stderr, b"registration log marker\n");
    fixture.assert_live_log("observations");
}

#[test]
fn json_arguments_preserve_both_quiet_spellings_after_separator() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let cases: [(&str, &[&str]); 3] = [
        (
            "observations/without-quiet",
            &["check", "probe-mismatch", "--message-format=json", "--"],
        ),
        (
            "observations/--quiet",
            &[
                "check",
                "probe-mismatch",
                "--quiet",
                "--message-format=json",
                "--",
                "--quiet",
            ],
        ),
        (
            "observations/-q",
            &[
                "check",
                "probe-mismatch",
                "-q",
                "--message-format=json",
                "--",
                "-q",
            ],
        ),
    ];
    let executed: [&[&str]; 3] = [
        &["check", "probe-mismatch", "--message-format=json", "--"],
        &[
            "check",
            "probe-mismatch",
            "--message-format=json",
            "--",
            "--quiet",
        ],
        &[
            "check",
            "probe-mismatch",
            "--message-format=json",
            "--",
            "-q",
        ],
    ];
    for (((observations, arguments), output), executed) in
        cases.iter().zip(fixture.run_cases(&cases)).zip(executed)
    {
        assert_completed_arguments(&fixture, observations, arguments, executed, &output);
    }
}

#[test]
fn quiet_json_rewrite_preserves_unrelated_arguments() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let cases: [(&str, &[&str]); 2] = [
        (
            "observations/without-quiet",
            &[
                "check",
                "probe-mismatch",
                "--message-format=json",
                "--release",
            ],
        ),
        (
            "observations/with-quiet",
            &[
                "check",
                "probe-mismatch",
                "--quiet",
                "--message-format=json",
                "--workspace",
            ],
        ),
    ];
    let executed: [&[&str]; 2] = [
        &[
            "check",
            "probe-mismatch",
            "--message-format=json",
            "--release",
        ],
        &[
            "check",
            "probe-mismatch",
            "--message-format=json",
            "--workspace",
        ],
    ];
    for (((observations, arguments), output), executed) in
        cases.iter().zip(fixture.run_cases(&cases)).zip(executed)
    {
        assert_completed_arguments(&fixture, observations, arguments, executed, &output);
    }
}

/// The old whitespace-joined representation could not distinguish these invocations.
#[test]
fn one_argument_with_a_space_differs_from_two_arguments() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let cases: [(&str, &[&str]); 2] = [
        ("observations/joined", &["run", "--", "a b"]),
        ("observations/separated", &["run", "--", "a", "b"]),
    ];
    let mut results = cases
        .into_iter()
        .zip(fixture.run_cases(&cases))
        .map(|((observations, _), output)| (output, fixture.registration_at(observations)));
    let (joined_output, joined) = results.next().expect("joined argument case");
    let (separated_output, separated) = results.next().expect("separated argument case");
    assert_cargo_result(&joined_output);
    assert_cargo_result(&separated_output);
    let joined_fields = joined.fields();
    let separated_fields = separated.fields();
    assert_eq!(joined_fields[7], b"3");
    assert_eq!(separated_fields[7], b"4");
    assert_eq!(&joined_fields[8..], [b"run".as_slice(), b"--", b"a b"]);
    assert_eq!(
        &separated_fields[8..],
        [b"run".as_slice(), b"--", b"a", b"b"]
    );
    assert_ne!(&joined_fields[8..], &separated_fields[8..]);
}

/// Equal home-relative spellings under separate homes retain distinct directory identity.
#[test]
fn matching_relative_directories_keep_their_absolute_paths_and_writer_homes() {
    let first = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let second = InstalledShim::new(NativeBirthVerification::Unnecessary);
    std::thread::scope(|scope| {
        let first_case = scope.spawn(|| {
            assert_cargo_result(&first.run(&["build"]));
        });
        let second_case = scope.spawn(|| {
            assert_cargo_result(&second.run(&["build"]));
        });
        first_case.join().expect("the first fixture finishes");
        second_case.join().expect("the second fixture finishes");
    });
    let first_registration = first.registration();
    let second_registration = second.registration();
    let first_fields = first_registration.fields();
    let second_fields = second_registration.fields();
    assert_ne!(first_fields[5], second_fields[5]);
    assert_ne!(first_fields[6], second_fields[6]);
    for (fixture, fields) in [(&first, first_fields), (&second, second_fields)] {
        assert_eq!(
            fields[5],
            fixture
                .path("home/work tree\twith\nlines")
                .as_os_str()
                .as_encoded_bytes()
        );
        assert_eq!(
            fields[6],
            fixture.path("home").as_os_str().as_encoded_bytes()
        );
    }
}

/// Command substitution must not strip a newline that belongs to the directory name.
#[test]
fn raw_directory_preserves_a_trailing_newline() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let directory = fixture.path("home/ends in newline\n");
    fs::create_dir(&directory).expect("create newline-terminated directory name");
    let output = fixture
        .command(&["build"])
        .current_dir(&directory)
        .output()
        .expect("run from newline-terminated directory");
    assert_cargo_result(&output);
    assert_eq!(
        fixture.registration().fields()[5],
        directory.as_os_str().as_encoded_bytes()
    );
}

/// The filename and log field identify the same live run and are removed on exit.
#[test]
fn registration_and_log_share_pid_and_generation() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
    assert!(pid.parse::<u32>().expect("shim pid is numeric") > 0);
    let generation = std::str::from_utf8(fields[1]).expect("generation is ASCII");
    assert_eq!(registration.name, format!("{pid}.{generation}"));
    assert_eq!(
        fs::read_to_string(fixture.path("observations/state-entries"))
            .expect("cargo lists the live state directory"),
        "pids\n",
        "stderr capture leaves nothing in state beside the registrations"
    );
    let log = std::str::from_utf8(fields[4]).expect("log basename is ASCII");
    assert_eq!(log, format!("run-{generation}-{pid}.log"));
    assert_eq!(
        fs::read(fixture.path("observations").join(log)).expect("snapshot actual capture log"),
        b"registration log marker\n"
    );
    assert!(!fixture.path("capture").join(log).exists());
    assert_eq!(
        fs::read_dir(fixture.path("capture/state/pids"))
            .expect("registration directory survives cleanup")
            .count(),
        0,
        "cleanup removes the registration and any temporary staging file"
    );
}

/// Publish complete bytes from an ignored same-directory staging file using ln.
#[test]
fn publication_links_a_complete_tmp_file_into_place() {
    if let Some(shim) = std::env::var_os("SHIM_NATIVE_TEST_CALL") {
        run_native_publication_fixture(Path::new(&shim));
    }

    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    assert_cargo_result(&fixture.run(&["check"]));
    let registration = fixture.registration();
    let publication = fs::read(fixture.path("observations/publication-arguments"))
        .expect("publication calls the exclusive-create ln primitive");
    let arguments = nul_fields(&publication);
    assert_eq!(arguments.len(), 2, "publication must not force replacement");
    let temporary = Path::new(std::str::from_utf8(arguments[0]).expect("temporary path"));
    let final_path = Path::new(std::str::from_utf8(arguments[1]).expect("registration path"));
    assert_eq!(
        temporary.extension().and_then(|part| part.to_str()),
        Some("tmp")
    );
    assert_eq!(temporary.parent(), final_path.parent());
    assert_eq!(
        final_path,
        fixture.path("capture/state/pids").join(&registration.name)
    );
    assert_eq!(
        fs::read(fixture.path("observations/staged-registration")).expect("observe staging bytes"),
        registration.contents,
        "the full registration exists before its final name becomes visible"
    );
    assert_eq!(
        fs::metadata(fixture.path("observations/staged-registration"))
            .expect("staging mode")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    assert!(!temporary.exists());
}

/// Reading /proc/self in setup would describe a later child instead of this shim.
#[test]
fn birth_stamp_belongs_to_the_pid_in_the_registration_filename() {
    let fixture = InstalledShim::new(NativeBirthVerification::Required);
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
    assert_eq!(
        registration.name.split_once('.').map(|pair| pair.0),
        Some(pid.as_str())
    );
    let birth = observed_birth(&fixture);
    assert!(
        !birth.is_empty(),
        "the live shim has an observable birth stamp"
    );
    assert_eq!(fields[3], birth.as_bytes());
    assert!(fields[3].iter().all(u8::is_ascii_digit), "birth is decimal");
    assert!(!fields[2].is_empty(), "this host exposes its boot identity");
}

/// A native query for another live process cannot supply the shim's birth.
#[test]
fn native_birth_verification_rejects_another_process() {
    let fixture = InstalledShim::new(NativeBirthVerification::Required);
    let output = fixture
        .command(&["build"])
        .env("SHIM_TEST_BIRTH_PID", std::process::id().to_string())
        .output()
        .expect("supply another live process to the native birth query");
    assert_cargo_result(&output);
    assert_eq!(fixture.registration().fields()[3], b"");
}

/// Calendar time and a reused shell pid cannot reproduce a publication name.
#[test]
fn same_pid_and_calendar_second_receive_different_generations() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fixture.repeat_in_same_shell();
    let output = fixture.run(&["build"]);
    assert_eq!(output.status.code(), Some(37), "{output:?}");
    assert_eq!(output.stdout, b"cargo stdout\ncargo stdout\n");
    assert_eq!(
        output.stderr,
        b"registration log marker\nregistration log marker\n"
    );
    let first = fixture.registration_at("observations/first");
    let second = fixture.registration_at("observations/second");
    assert_eq!(
        first.name.split_once('.').map(|pair| pair.0),
        second.name.split_once('.').map(|pair| pair.0)
    );
    assert_eq!(
        fs::read(fixture.path("observations/first/calendar")).expect("first clock"),
        fs::read(fixture.path("observations/second/calendar")).expect("second clock")
    );
    assert_eq!(
        &first.fields()[2..4],
        &second.fields()[2..4],
        "same boot and shell birth"
    );
    assert_ne!(
        first.fields()[1],
        second.fields()[1],
        "distinct invocation generations"
    );
    assert_ne!(first.fields()[4], second.fields()[4], "distinct log names");
    assert_ne!(first.name, second.name);
}

/// A delayed unlink of a prior registration and log cannot reach the later invocation.
#[test]
fn publication_after_final_confirmation_survives_delayed_unlinks() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fixture.repeat_in_same_shell();
    let output = fixture
        .command(&["build"])
        .env(
            "SHIM_TEST_PENDING_DELETE",
            fixture.path("observations/pending-delete"),
        )
        .output()
        .expect("run with deferred removal of first invocation names");
    assert_eq!(output.status.code(), Some(37), "{output:?}");
    let first = fixture.registration_at("observations/first");
    let second = fixture.registration_at("observations/second");
    assert_ne!(first.name, second.name);
    assert_eq!(
        fs::read(fixture.path("observations/second/delayed-delete"))
            .expect("attempt both stale unlinks while second cargo is alive"),
        b"delayed unlink attempted\n"
    );
    let log = std::str::from_utf8(second.fields()[4]).expect("second log basename");
    assert_eq!(
        fs::read(fixture.path("observations/second").join(log))
            .expect("second log survives delayed removal"),
        b"registration log marker\n"
    );
}

/// Real calendar conversion stays unambiguous without changing cargo's environment.
#[test]
fn darwin_birth_conversion_scopes_locale_and_keeps_cargo_environment() {
    parallel_cases::run(["UTC0", "UTC-11"], |timezone| {
        let fixture = InstalledShim::new(NativeBirthVerification::Required);
        fixture.observe_live_log();
        let output = fixture
            .command(&["build"])
            .env("LC_ALL", "POSIX")
            .env("LANG", "C")
            .env("TZ", timezone)
            .output()
            .expect("native birth writer");
        assert_cargo_result(&output);
        fixture.assert_live_log("observations");
        let registration = fixture.registration();
        assert_eq!(
            registration.fields()[3],
            observed_birth(&fixture).as_bytes()
        );
        let cargo = fs::read(fixture.path("observations/locale-settings")).expect("cargo locale");
        assert_eq!(
            &nul_fields(&cargo)[..3],
            [b"POSIX".as_slice(), timezone.as_bytes(), b"C"]
        );
    });
}

/// The two occurrences of 01:30 at DST fallback must publish different epoch seconds.
#[test]
fn darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback() {
    let births_and_utc_offsets = [
        (1_793_511_000_i64, -4 * 60 * 60),
        (1_793_514_600_i64, -5 * 60 * 60),
    ];
    assert_eq!(
        births_and_utc_offsets[1].0 - births_and_utc_offsets[0].0,
        60 * 60,
        "fallback occurrences are one hour apart"
    );
    assert_eq!(
        births_and_utc_offsets[0].0 + births_and_utc_offsets[0].1,
        births_and_utc_offsets[1].0 + births_and_utc_offsets[1].1,
        "both epochs convert to the same local time"
    );
    let published = parallel_cases::run(births_and_utc_offsets, |(birth, _)| {
        let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
        fixture.observe_live_log();
        let birth = birth.to_string();
        let observations = format!("observations/{birth}");
        fs::create_dir_all(fixture.path(&observations)).expect("epoch observations");
        let timezone = "EST5EDT,M3.2.0,M11.1.0";
        let output = fixture
            .command_recorded_in(&observations, &["build"])
            .env("LC_ALL", "POSIX")
            .env("LANG", "C")
            .env("TZ", timezone)
            .env("SHIM_TEST_BIRTH", &birth)
            .output()
            .expect("publish during the repeated DST hour");
        assert_cargo_result(&output);
        fixture.assert_live_log(&observations);
        let registration = fixture.registration_at(&observations);
        assert_eq!(registration.fields()[3], birth.as_bytes());
        let cargo = fs::read(fixture.path(&format!("{observations}/locale-settings")))
            .expect("cargo records its inherited timezone");
        assert_eq!(
            &nul_fields(&cargo)[..3],
            [b"POSIX".as_slice(), timezone.as_bytes(), b"C"]
        );
        registration.fields()[3].to_vec()
    });
    assert_ne!(published[0], published[1]);
}

/// Partial stdout from a failed conversion is never published as birth evidence.
#[test]
fn failed_darwin_birth_conversion_publishes_empty_identity_field() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::write(fixture.path("observations/birth-unavailable"), b"")
        .expect("fail after emitting partial conversion output");
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    assert!(!fields[2].is_empty(), "boot observation still succeeds");
    assert!(
        fields[3].is_empty(),
        "a failed birth observation remains unknown"
    );
}

/// A competing publisher can take the name after setup's preliminary existence check.
#[test]
fn occupied_registration_runs_original_cargo_without_replacing_owned_files() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::write(fixture.path("observations/occupy-at-publication"), b"")
        .expect("occupy the name immediately before the real ln attempts publication");
    let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
    assert_cargo_result(&fixture.run(&arguments));
    let observed = fs::read(fixture.path("observations/arguments")).expect("read fallback argv");
    assert_eq!(
        nul_fields(&observed),
        arguments
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
    let umask = fs::read_to_string(fixture.path("observations/umask")).expect("read cargo umask");
    assert_eq!(
        u32::from_str_radix(umask.trim(), 8).expect("octal umask"),
        0o066
    );
    let settings =
        fs::read(fixture.path("observations/capture-settings")).expect("read capture settings");
    assert_eq!(nul_fields(&settings), [b"".as_slice(), b"", b""]);
    let staged = fixture.staged_registration();
    assert_eq!(
        fs::read(fixture.path("capture/state/pids").join(&staged.name))
            .expect("existing registration survives failed publication and cleanup"),
        b"previous registration\n"
    );
    assert!(
        !fixture
            .path("capture")
            .join(std::str::from_utf8(staged.fields()[4]).expect("log basename"))
            .exists(),
        "failed publication removes the new run's log"
    );
    assert_eq!(
        fs::read_dir(fixture.path("capture/state/pids"))
            .expect("read remaining registrations")
            .count(),
        1
    );
}

/// A collision that also has a log must not truncate or delete that prior run's output.
#[test]
fn occupied_registration_also_preserves_its_existing_log() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::write(fixture.path("observations/occupy-name"), b"").expect("request a name collision");
    fs::write(fixture.path("observations/occupy-log"), b"").expect("request an existing log");
    assert_cargo_result(&fixture.run(&["build"]));
    let staged = fixture.staged_registration();
    assert_eq!(
        fs::read(fixture.path("capture/state/pids").join(&staged.name))
            .expect("existing registration survives log collision"),
        b"previous registration\n"
    );
    assert_eq!(
        fs::read(
            fixture
                .path("capture")
                .join(std::str::from_utf8(staged.fields()[4]).expect("log basename"))
        )
        .expect("existing log survives failed setup and cleanup"),
        b"previous log\n"
    );
    let settings =
        fs::read(fixture.path("observations/capture-settings")).expect("read capture settings");
    assert_eq!(nul_fields(&settings), [b"".as_slice(), b"", b""]);
}

/// POSIX ln treats an existing directory as a destination, which must not publish a run.
#[test]
fn occupied_directory_is_a_setup_failure_and_keeps_its_contents() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::write(fixture.path("observations/occupy-directory"), b"")
        .expect("request a directory at the publication name");
    assert_cargo_result(&fixture.run(&["check", "--quiet", "--message-format=json"]));
    let settings =
        fs::read(fixture.path("observations/capture-settings")).expect("read capture settings");
    assert_eq!(nul_fields(&settings), [b"".as_slice(), b"", b""]);
    let staged = fixture.staged_registration();
    let occupied = fixture.path("capture/state/pids").join(&staged.name);
    assert_eq!(
        fs::read(occupied.join("keep")).expect("preserve the occupied directory's contents"),
        b"existing directory\n"
    );
    assert_eq!(
        fs::read_dir(occupied)
            .expect("read occupied directory")
            .count(),
        1,
        "publication must not add its staging file inside the occupied directory"
    );
}

/// TERM can arrive after ln succeeds but before setup records ownership of its name.
#[test]
fn term_after_publication_cleans_artifacts_without_starting_cargo() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    for target in ["shim", "setup", "mirror"] {
        let observations = format!("observations/{target}");
        fs::create_dir_all(fixture.path(&observations)).expect("create signal observations");
        fs::write(
            fixture.path(&format!(
                "{observations}/{}",
                if target == "mirror" {
                    "signal-after-readiness"
                } else {
                    "signal-after-publication"
                }
            )),
            target,
        )
        .expect("request TERM before ln returns to the setup shell");
        let output = fixture.run_recorded_in(&observations, &["build"]);
        assert_eq!(
            output.status.code(),
            Some(143),
            "TERM delivered to {target}"
        );
        assert_eq!(output.stdout, [] as [u8; 0]);
        assert_eq!(output.stderr, [] as [u8; 0]);
        assert!(
            fixture
                .path(&format!("{observations}/staged-registration"))
                .exists()
        );
        assert!(
            !fixture.path(&format!("{observations}/arguments")).exists(),
            "cargo must not start after TERM"
        );
        assert_eq!(
            fs::read_dir(fixture.path("capture/state/pids"))
                .expect("read publication directory")
                .count(),
            0,
            "TERM removes both the registration and staging file"
        );
        assert_eq!(
            fs::read_dir(fixture.path("capture"))
                .expect("read capture root")
                .count(),
            1,
            "only the state directory remains"
        );
        assert_eq!(
            fs::read_dir(fixture.path("capture/state"))
                .expect("read state directory")
                .count(),
            1,
            "only the pids directory remains"
        );
    }
}

/// Each shim sweeps dead registrations, staging, and logs only in its account.
#[test]
fn dead_pid_captures_are_reaped_before_publication_without_touching_another_account() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let mut exited = Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("create known pid");
    let pid = exited.id();
    assert!(exited.wait().expect("reap fixture child").success());
    assert!(
        !Command::new("sh")
            .args(["-c", "kill -0 \"$1\" 2>/dev/null", "sh", &pid.to_string()])
            .status()
            .expect("verify dead pid")
            .success()
    );
    let account = fixture.path("capture");
    let other = account.parent().expect("shared parent").join("4294967294");
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    for directory in [&account, &other] {
        fs::create_dir_all(directory.join("state/pids")).expect("create account hierarchy");
        let mut paths = Vec::new();
        for generation in ["old", "staging-only"] {
            let name = format!("{pid}.{generation}");
            let log = format!("run-{generation}-{pid}.log");
            let bytes = format!(
                "cargo-tile-v2\0{generation}\0old-boot\0birth\0{log}\0/work\0/home\x001\0build\0"
            );
            if generation == "old" {
                let registration = directory.join("state/pids").join(&name);
                fs::write(&registration, &bytes).expect("seed dead registration");
                paths.push(registration);
            }
            let staging = directory.join("state/pids").join(format!("{name}.tmp"));
            fs::write(&staging, &bytes).expect("seed dead staging");
            paths.push(staging);
            let log = directory.join(log);
            fs::write(&log, b"old progress").expect("seed dead log");
            paths.push(log);
        }
        if directory == &account {
            removed = paths;
        } else {
            retained = paths;
        }
    }
    assert_cargo_result(&fixture.run(&["build"]));
    assert!(
        !fixture.registration().contents.is_empty(),
        "fresh cargo still registers"
    );
    for path in removed {
        assert!(!path.exists(), "dead artifact survives: {}", path.display());
    }
    for path in retained {
        assert!(path.exists(), "another account loses {}", path.display());
    }
}

/// A shell seeds its predecessor and execs the shim without changing pid.
#[test]
fn same_pid_predecessor_is_reaped_before_the_new_invocation_registers() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::write(fixture.path("observations/seed-predecessor"), b"")
        .expect("seed stale captures from the invoking shell");
    assert_cargo_result(&fixture.run(&["build"]));
    let pid =
        fs::read_to_string(fixture.path("observations/shim-pid")).expect("same shell and shim pid");
    let fresh = fixture.registration();
    assert!(fresh.name.starts_with(&format!("{pid}.")));
    for generation in ["predecessor", "staging-only"] {
        for relative in [
            format!("state/pids/{pid}.{generation}"),
            format!("state/pids/{pid}.{generation}.tmp"),
            format!("run-{generation}-{pid}.log"),
        ] {
            assert!(
                !fixture.path("capture").join(&relative).exists(),
                "predecessor artifact survives: {relative}"
            );
        }
    }
    assert_eq!(
        fs::read_to_string(fixture.path("observations/state-entries"))
            .expect("cargo lists the live state directory"),
        "pids\n"
    );
}

/// Publishing v2 alongside an older shim must leave that shim's registration intact.
#[test]
fn legacy_registration_and_log_survive_a_new_shim_run() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::create_dir_all(fixture.path("capture/state/pids")).expect("create mixed-format directory");
    let old_pid = std::process::id();
    let old_registration = fixture.path(&format!("capture/state/pids/{old_pid}"));
    let old_log = fixture.path(&format!("capture/run-20260909-203959-{old_pid}.log"));
    fs::write(&old_registration, b"~/legacy\tcargo build\n").expect("seed old registration");
    fs::write(&old_log, b"old live log\n").expect("seed old live log");
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    assert!(registration.name.contains('.'));
    assert_eq!(
        fs::read(fixture.path(&format!("observations/pids/{old_pid}")))
            .expect("old registration remains visible during new run"),
        b"~/legacy\tcargo build\n"
    );
    assert_eq!(
        fs::read(old_registration).expect("old registration survives cleanup"),
        b"~/legacy\tcargo build\n"
    );
    assert_eq!(
        fs::read(old_log).expect("old log survives cleanup"),
        b"old live log\n"
    );
}

/// Publishing the extended layout must preserve both older v2 layouts and malformed neighbors.
#[test]
fn phase_two_records_and_malformed_neighbors_survive_publication_and_cleanup() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fs::create_dir_all(fixture.path("capture/state/pids")).expect("mixed-format directory");
    let pid = std::process::id();
    let records: &[(&str, &[u8])] = &[
            ("phase-two", b"cargo-tile-v2\0phase-two\0boot\x00123\0old.log\0~/work\x001\0build\0"),
            ("unknown", b"cargo-tile-v2\0unknown\0\0\0unknown.log\0~/work\x001\0build\0"),
            ("darwin-text", b"cargo-tile-v2\0darwin-text\0boot\0Wed Sep  9 12:34:56 2026\0darwin.log\0~/work\x001\0build\0"),
            ("malformed", b"cargo-tile-v2\0partial"),
        ];
    for (generation, bytes) in records {
        fs::write(
            fixture.path(&format!("capture/state/pids/{pid}.{generation}")),
            bytes,
        )
        .expect("seed prior registration");
    }
    for log in ["old.log", "unknown.log", "darwin.log"] {
        fs::write(fixture.path("capture").join(log), b"older output\n").expect("seed prior log");
    }
    assert_cargo_result(&fixture.run(&["build"]));
    let staged = fixture.staged_registration();
    assert_eq!(
        fs::read(fixture.path("observations/pids").join(staged.name))
            .expect("new registration is visible alongside old records"),
        staged.contents
    );
    for (generation, bytes) in records {
        let name = format!("{pid}.{generation}");
        for directory in ["capture/state/pids", "observations/pids"] {
            assert_eq!(
                fs::read(fixture.path(directory).join(&name)).expect("neighbor remains unchanged"),
                *bytes
            );
        }
    }
    for log in ["old.log", "unknown.log", "darwin.log"] {
        assert_eq!(
            fs::read(fixture.path("capture").join(log)).expect("prior log survives"),
            b"older output\n"
        );
    }
}

/// Shell utility failures must not remove native capture identity.
#[test]
fn native_birth_needs_no_date_or_ps_conversion() {
    let fixture = InstalledShim::new(NativeBirthVerification::Required);
    for utility in ["date", "ps", "uname", "sysctl"] {
        executable(
            &fixture.path(&format!("tools/{utility}")),
            "#!/bin/sh\nprintf 'partial conversion output'\nexit 1\n",
        );
    }
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    assert_ne!(fields[2], b"");
    assert_ne!(fields[3], b"");
    assert!(fields[3].iter().all(u8::is_ascii_digit));
    assert_eq!(fields[3], observed_birth(&fixture).as_bytes());
    fixture.assert_live_log("observations");
}

fn seed_native_predecessor(account: &Path) {
    fs::create_dir_all(account.join("state/pids")).expect("predecessor directory");
    for generation in ["predecessor", "staging-only"] {
        let pid = std::process::id();
        let registration = account
            .join("state/pids")
            .join(format!("{pid}.{generation}"));
        let staging = registration.with_extension(format!("{generation}.tmp"));
        fs::write(
            &staging,
            b"cargo-tile-v2\0old\0old-boot\0birth\0old.log\0/work\0/home\x001\0build\0",
        )
        .expect("old staging");
        if generation == "predecessor" {
            fs::copy(&staging, registration).expect("old publication");
        }
        fs::write(
            account.join(format!("run-{generation}-{pid}.log")),
            b"old progress\n",
        )
        .expect("old log");
    }
}

fn native_publication_boundary(
    account: &Path,
    observed: &Path,
    generation: &str,
    staging: &Path,
    registration: &Path,
) -> std::io::Result<()> {
    let mut paths = Vec::new();
    for path in [staging, registration] {
        paths.extend_from_slice(path.as_os_str().as_bytes());
        paths.push(0);
    }
    fs::write(observed.join("publication-arguments"), paths)?;
    fs::copy(staging, observed.join("staged-registration"))?;
    fs::set_permissions(
        observed.join("staged-registration"),
        fs::metadata(staging)?.permissions(),
    )?;
    if observed.join("sweep-before-publication").exists() {
        assert!(!registration.exists(), "sweep precedes publication");
        for entry in fs::read_dir(account)? {
            let entry = entry?;
            if entry.file_name().as_bytes().starts_with(b"run-")
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "log")
            {
                fs::remove_file(entry.path())?;
            }
        }
        fs::write(observed.join("sweep"), b"swept before publication\n")?;
    }
    if observed.join("occupy-at-publication").exists() || observed.join("occupy-name").exists() {
        fs::write(registration, b"previous registration\n")?;
    }
    if observed.join("occupy-log").exists() {
        fs::write(
            account.join(format!("run-{generation}-{}.log", std::process::id())),
            b"previous log\n",
        )?;
    }
    if observed.join("occupy-directory").exists() {
        fs::create_dir(registration)?;
        fs::write(registration.join("keep"), b"existing directory\n")?;
    }
    Ok(())
}

/// Preserve pid verification while supplying the native publication fixture's wire fields.
fn native_fixture_identity(observations: &Path) -> (String, String) {
    let birth_pid = std::env::var("SHIM_TEST_BIRTH_PID").map_or_else(
        |_| std::process::id(),
        |pid| pid.parse::<u32>().expect("native birth query pid"),
    );
    let (boot, birth) =
        hook::capture_identity_fields(std::process::id(), birth_stamp::observe(birth_pid));
    let birth = if observations.join("birth-unavailable").exists() {
        String::new()
    } else {
        std::env::var("SHIM_TEST_BIRTH").unwrap_or(birth)
    };
    (boot, birth)
}

/// A table call records its actual streams while a normal call inherits the caller streams.
enum FixtureOutput {
    Inherited,
    Recorded,
}

/// One publication call retains its own arguments and observations inside a shared process.
struct NativePublicationInvocation {
    observations: PathBuf,
    arguments:    Vec<OsString>,
    output:       FixtureOutput,
}

impl NativePublicationInvocation {
    fn redirect(&self) {
        if matches!(self.output, FixtureOutput::Recorded) {
            let stdout = fs::File::create(self.observations.join("stdout")).expect("case stdout");
            let stderr = fs::File::create(self.observations.join("stderr")).expect("case stderr");
            rustix::stdio::dup2_stdout(&stdout).expect("select case stdout");
            rustix::stdio::dup2_stderr(&stderr).expect("select case stderr");
        }
    }

    fn finish(&self, status: i32) {
        if matches!(self.output, FixtureOutput::Recorded) {
            fs::write(self.observations.join("exit-status"), status.to_string())
                .expect("record case exit status");
        }
    }
}

fn native_fixture_arguments(path: &Path) -> Vec<OsString> {
    let bytes = fs::read(path).expect("fixture arguments");
    bytes
        .strip_suffix(&[0])
        .unwrap_or(&bytes)
        .split(|byte| *byte == 0)
        .map(|word| OsString::from_vec(word.to_vec()))
        .collect()
}

fn native_fixture_invocations(observations: &Path) -> Vec<NativePublicationInvocation> {
    if let Some(schedule) = std::env::var_os("SHIM_NATIVE_TEST_CASES") {
        let bytes = fs::read(schedule).expect("argument table schedule");
        return bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                let observations = PathBuf::from(std::ffi::OsStr::from_bytes(path));
                let arguments =
                    native_fixture_arguments(&observations.join("invocation-arguments"));
                NativePublicationInvocation {
                    observations,
                    arguments,
                    output: FixtureOutput::Recorded,
                }
            })
            .collect();
    }
    let argument_file = std::env::var_os("SHIM_NATIVE_TEST_ARGUMENTS").expect("argument file");
    let arguments = native_fixture_arguments(Path::new(&argument_file));
    let runs = if observations.join("repeat").exists() {
        vec![observations.join("first"), observations.join("second")]
    } else {
        vec![observations.to_owned()]
    };
    runs.into_iter()
        .map(|observations| NativePublicationInvocation {
            observations,
            arguments: arguments.clone(),
            output: FixtureOutput::Inherited,
        })
        .collect()
}

fn run_native_publication_fixture(shim: &Path) -> ! {
    let stdout = fs::OpenOptions::new()
        .write(true)
        .open("/dev/fd/9")
        .expect("original stdout");
    rustix::stdio::dup2_stdout(&stdout)
        .expect("restore cargo stdout after the test harness header");
    drop(stdout);
    let account =
        PathBuf::from(std::env::var_os("SHIM_TEST_ACCOUNT_DIRECTORY").expect("account root"));
    let parent = account.parent().expect("shared parent").to_owned();
    let observations =
        PathBuf::from(std::env::var_os("SHIM_TEST_OBSERVATIONS").expect("observations"));
    let (boot, birth) = native_fixture_identity(&observations);
    // A fixed calendar second exercises the UUID rather than wall-clock separation.
    let calendar = "20260909-204000";
    if observations.join("seed-predecessor").exists() {
        seed_native_predecessor(&account);
    }
    let mut status = 127;
    for invocation in native_fixture_invocations(&observations) {
        let observed = invocation.observations.as_path();
        invocation.redirect();
        fs::create_dir_all(observed).expect("invocation observations");
        fs::write(observed.join("shim-pid"), std::process::id().to_string()).expect("shim pid");
        fs::write(observed.join("calendar"), calendar).expect("calendar input");
        let generation = hook::capture_generation_at(
            chrono::DateTime::parse_from_rfc3339("2026-09-09T20:40:00Z")
                .expect("calendar input")
                .to_utc(),
        );
        let environment = [("SHIM_TEST_OBSERVATIONS", observed.as_os_str())];
        let publication = CaptureTestPublication {
            boot:            &boot,
            birth:           &birth,
            generation:      &generation,
            environment:     &environment,
            after_readiness: if observed.join("signal-after-readiness").exists() {
                || signal_hook::low_level::raise(signal_hook::consts::SIGTERM)
            } else {
                || Ok(())
            },
        };
        let uid = if observations.join("foreign-account").exists() {
            rustix::process::getuid().as_raw().wrapping_add(1)
        } else {
            rustix::process::getuid().as_raw()
        };
        status = publication
            .run_for_account(
                shim,
                parent.clone(),
                invocation.arguments.clone(),
                uid,
                |staging, registration| {
                    native_publication_boundary(
                        &account,
                        observed,
                        &generation,
                        staging,
                        registration,
                    )
                },
                || {
                    if observed.join("signal-after-publication").exists() {
                        signal_hook::low_level::raise(signal_hook::consts::SIGTERM)?;
                    }
                    Ok(())
                },
            )
            .expect("native fixture call");
        invocation.finish(status);
        if status != 37 {
            break;
        }
    }
    std::process::exit(status);
}

#[test]
fn a_publication_boundary_sweep_keeps_registration_and_log_readable() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    fixture.observe_live_log();
    fs::create_dir_all(fixture.path("capture")).expect("account directory");
    let orphan = fixture.path("capture/run-orphan.log");
    fs::write(&orphan, b"orphan output").expect("orphan log");
    fs::write(fixture.path("observations/sweep-before-publication"), b"")
        .expect("sweep at publication boundary");
    assert_cargo_result(&fixture.run(&["build"]));
    assert_eq!(
        fs::read(fixture.path("observations/sweep")).expect("publication boundary sweep"),
        b"swept before publication\n"
    );
    assert!(!orphan.exists());
    let staged = fixture.path("observations/staged-registration");
    assert_eq!(
        fs::metadata(&staged)
            .expect("staging mode")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    let registration = fixture.registration();
    assert_eq!(
        fs::read(staged).expect("complete staging record"),
        registration.contents
    );
    let published = fixture.path("observations/pids").join(&registration.name);
    assert_eq!(
        fs::metadata(published)
            .expect("published mode")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    let log = std::str::from_utf8(registration.fields()[4])
        .expect("log name")
        .to_owned();
    let captured = fixture.path("observations").join(log);
    assert_eq!(
        fs::read(&captured).expect("captured cargo bytes"),
        b"registration log marker\n"
    );
    assert_eq!(
        fs::metadata(captured)
            .expect("log mode")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn foreign_owned_account_is_refused_without_changing_cargo() {
    let fixture = InstalledShim::new(NativeBirthVerification::Unnecessary);
    let uid = rustix::process::getuid().as_raw().wrapping_add(1);
    let foreign = fixture
        .path("capture")
        .parent()
        .expect("capture parent")
        .join(uid.to_string());
    fs::create_dir_all(&foreign).expect("foreign account fixture");
    assert_ne!(
        fs::metadata(&foreign).expect("actual account owner").uid(),
        uid
    );
    fs::write(foreign.join("keep"), b"untouched").expect("foreign sentinel");
    fs::write(fixture.path("observations/foreign-account"), b"").expect("supplied account");
    let arguments = ["check", "--quiet", "--message-format=json", "--", "a b"];
    let output = fixture.run(&arguments);
    assert_cargo_result(&output);
    assert_eq!(
        fs::read_dir(&foreign).expect("foreign directory").count(),
        1
    );
    assert_eq!(
        fs::read(foreign.join("keep")).expect("foreign sentinel"),
        b"untouched"
    );
    assert_eq!(
        fs::read(fixture.path("observations/capture-settings")).expect("cargo capture settings"),
        b"\0\0\0"
    );
    assert_eq!(
        fs::read(fixture.path("observations/environment")).expect("cargo environment"),
        b"unset\0unset\0unset\0"
    );
    assert_eq!(
        nul_fields(&fs::read(fixture.path("observations/arguments")).expect("cargo arguments")),
        arguments.map(str::as_bytes)
    );
    assert_eq!(
        u32::from_str_radix(
            fs::read_to_string(fixture.path("observations/umask"))
                .expect("cargo umask")
                .trim(),
            8
        )
        .expect("octal cargo umask"),
        0o066
    );
}
