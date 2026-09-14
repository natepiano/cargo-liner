//! Shim publication wire contracts and their isolated executable fixture.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use tempfile::TempDir;

use crate::constants::REGISTRATION_MAGIC;

/// Whether native birth assertions need to distinguish the shim from setup children.
enum BirthTimeSeparation {
    Unnecessary,
    Required,
}

/// Own every path the copied shim and stand-in cargo can write.
struct InstalledShim {
    /// Keep the toolchain, capture root, and observations alive through assertions.
    directory:             TempDir,
    birth_time_separation: BirthTimeSeparation,
}

impl InstalledShim {
    /// Install the repository script beside cargo that snapshots its live registration.
    fn new(birth_time_separation: BirthTimeSeparation) -> Self {
        let fixture = Self {
            directory: tempfile::tempdir().expect("create isolated toolchain"),
            birth_time_separation,
        };
        for directory in [
            "bin",
            "tools",
            "observations",
            "home/work tree\twith\nlines",
        ] {
            fs::create_dir_all(fixture.path(directory)).expect("create fixture directory");
        }
        let source = include_str!("../cargo-capture-shim.sh");
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
        install_date(&fixture.path("tools/date"));
        install_fifo_removal_observer(&fixture.path("tools/rm"));
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

    /// Run with an independently recorded shim pid and a restrictive caller umask.
    fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments)
            .output()
            .expect("execute copied shim with stand-in cargo")
    }

    /// Allow tests to change only the child environment and invocation schedule.
    fn command(&self, arguments: &[&str]) -> Command {
        let utilities = Command::new("sh")
            .args(["-c", "command -v ln; command -v date; command -v rm"])
            .output()
            .expect("locate system utilities before changing the child PATH");
        assert!(utilities.status.success());
        let utilities = String::from_utf8(utilities.stdout).expect("utility paths are UTF-8");
        let mut utilities = utilities.lines();
        let mut search_path = vec![self.path("tools")];
        search_path.extend(std::env::split_paths(
            &std::env::var_os("PATH").expect("test runner supplies PATH"),
        ));
        let mut command = Command::new("sh");
        command
                .args([
                    "-c",
                // Native birth comparisons need distinct shim and setup-child births,
                // including macOS ps lstart's one-second resolution.
                    r#"umask 0066
printf '%s' "$$" > "$SHIM_TEST_OBSERVATIONS/shim-pid"
if [ -f "$SHIM_TEST_OBSERVATIONS/seed-predecessor" ]; then
    mkdir -p "$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids"
    for generation in predecessor staging-only; do
        publication=$$.$generation
        log=run-$generation-$$.log
        staging="$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids/$publication.tmp"
        printf '%s\000' cargo-tile-v2 "$generation" old-boot birth "$log" /work /home 1 build > "$staging"
        if [ "$generation" = predecessor ]; then
            cp "$staging" "$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids/$publication"
        fi
        printf 'old progress\n' > "$SHIM_TEST_ACCOUNT_DIRECTORY/$log"
        mkfifo "$SHIM_TEST_ACCOUNT_DIRECTORY/state/stderr-$publication"
    done
fi
if [ "$SHIM_TEST_BIRTH_TIME_SEPARATION" = required ]; then
    sleep 1
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/repeat" ]; then
    for invocation in first second; do
        mkdir "$SHIM_TEST_OBSERVATIONS/$invocation"
        printf '%s' "$$" > "$SHIM_TEST_OBSERVATIONS/$invocation/shim-pid"
        (
            SHIM_TEST_OBSERVATIONS=$SHIM_TEST_OBSERVATIONS/$invocation
            export SHIM_TEST_OBSERVATIONS
            . "$0" "$@"
        )
        result=$?
        [ "$result" -eq 37 ] || exit "$result"
    done
    exit 37
fi
exec sh "$0" "$@""#,
                ])
                .arg(self.path("bin/cargo"))
                .args(arguments)
                .current_dir(self.path("home/work tree\twith\nlines"))
                .env("HOME", self.path("home"))
                .env("SHIM_TEST_ACCOUNT_DIRECTORY", self.path("capture"))
            .env("SHIM_TEST_OBSERVATIONS", self.path("observations"))
            .env(
                "SHIM_TEST_BIRTH_TIME_SEPARATION",
                match self.birth_time_separation {
                    BirthTimeSeparation::Unnecessary => "unnecessary",
                    BirthTimeSeparation::Required => "required",
                },
            )
                .env(
                    "SHIM_TEST_REAL_LN",
                    utilities.next().expect("system ln path"),
                )
                .env(
                    "SHIM_TEST_REAL_DATE",
                    utilities.next().expect("system date path"),
                )
                .env("SHIM_TEST_REAL_RM", utilities.next().expect("system rm path"))
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
fn install_cargo(path: &Path) {
    executable(
        path,
        r#"#!/bin/sh
set -eu
observations=$SHIM_TEST_OBSERVATIONS
printf '%s\000' "$@" > "$observations/arguments"
umask > "$observations/umask"
printf '%s\000' "${CARGOTILE_NESTED-}" "${CARGO_TERM_PROGRESS_WHEN-}" "${CARGO_TERM_PROGRESS_WIDTH-}" > "$observations/capture-settings"
printf '%s\000' "${LC_ALL-}" "${TZ-}" "${LANG-}" "${HOME-}" > "$observations/locale-settings"
shim_pid=$(cat "$observations/shim-pid")
case $SHIM_TEST_NATIVE_PLATFORM in
    linux)
        cat "/proc/$shim_pid/stat" > "$observations/shim-stat"
        cat /proc/sys/kernel/random/boot_id > "$observations/boot"
        ;;
    macos)
        started=$(LC_ALL=C TZ=UTC0 ps -o lstart= -p "$shim_pid")
        # BSD ps pads lstart to its column width; date warns about the suffix.
        started=$(printf '%s\n' "$started" | sed 's/[[:space:]]*$//')
        LC_ALL=C TZ=UTC0 "$SHIM_TEST_REAL_DATE" -j -f '%a %b %e %H:%M:%S %Y' "$started" +%s > "$observations/birth"
        sysctl -n kern.bootsessionuuid > "$observations/boot"
        ;;
esac
if [ -n "${SHIM_TEST_PENDING_DELETE-}" ] && [ -f "$SHIM_TEST_PENDING_DELETE" ]; then
    old_registration=$(sed -n '1p' "$SHIM_TEST_PENDING_DELETE")
    old_log=$(sed -n '2p' "$SHIM_TEST_PENDING_DELETE")
    rm -f "$old_registration" "$old_log"
    printf 'delayed unlink attempted\n' > "$observations/delayed-delete"
fi
if [ -d "$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids" ]; then
    cp -R "$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids" "$observations/pids"
fi
for fifo in "$SHIM_TEST_ACCOUNT_DIRECTORY"/state/stderr-*; do
    if [ -p "$fifo" ]; then printf '%s\n' "${fifo##*/}" >> "$observations/fifos"; fi
done
printf 'cargo stdout\n'
printf 'registration log marker\n' >&2
if [ -n "${CARGOTILE_NESTED-}" ]; then
    remaining=200
    while [ "$remaining" -gt 0 ]; do
        for log in "$SHIM_TEST_ACCOUNT_DIRECTORY"/run-*.log; do
            if [ -f "$log" ] && grep -q 'registration log marker' "$log"; then
                cp "$log" "$observations/${log##*/}"
                if [ -n "${SHIM_TEST_PENDING_DELETE-}" ] && [ ! -f "$SHIM_TEST_PENDING_DELETE" ]; then
                    for registration in "$SHIM_TEST_ACCOUNT_DIRECTORY/state/pids"/*; do
                        case $registration in *.tmp) continue ;; esac
                        printf '%s\n' "$registration" "$log" > "$SHIM_TEST_PENDING_DELETE"
                    done
                fi
                exit 37
            fi
        done
        remaining=$((remaining - 1))
        sleep 0.01
    done
    printf 'capture never wrote the cargo stderr marker\n' >&2
    exit 98
fi
exit 37
"#,
    );
}

/// Freeze calendar time while leaving native birth-stamp conversion available.
fn install_date(path: &Path) {
    executable(
        path,
        r#"#!/bin/sh
set -eu
if [ "$1" != +%Y%m%d-%H%M%S ]; then
    if [ -f "$SHIM_TEST_OBSERVATIONS/darwin-conversion" ]; then
        printf '%s\000' "${LC_ALL-}" "${TZ-}" "$@" > "$SHIM_TEST_OBSERVATIONS/date-conversion"
        [ "${LC_ALL-}" = C ] || exit 91
        if [ -f "$SHIM_TEST_OBSERVATIONS/gnu-date" ]; then
            # GNU coreutils date: no -j, and -d parses the same text.
            if [ "$1" = -j ]; then
                printf '%s\000' "$@" > "$SHIM_TEST_OBSERVATIONS/date-rejected"
                printf "date: invalid option -- 'j'\n" >&2
                exit 1
            fi
            [ "$#" -eq 4 ] && [ "$1" = -u ] && [ "$2" = -d ] && [ "$4" = +%s ] || exit 94
            exec python3 "$SHIM_TEST_OBSERVATIONS/darwin-time.py" date "$3"
        fi
        [ "$#" -eq 5 ] && [ "$1" = -j ] && [ "$2" = -f ] || exit 92
        [ "$3" = '%a %b %e %H:%M:%S %Y' ] && [ "$5" = +%s ] || exit 93
        if [ -f "$SHIM_TEST_OBSERVATIONS/fail-conversion" ]; then
            printf 'partial conversion output'
            exit 1
        fi
        exec python3 "$SHIM_TEST_OBSERVATIONS/darwin-time.py" date "$4"
    fi
    exec "$SHIM_TEST_REAL_DATE" "$@"
fi
printf '%s\n' "$SHIM_TEST_GENERATION"
printf '%s' "$SHIM_TEST_GENERATION" > "$SHIM_TEST_OBSERVATIONS/calendar"
"#,
    );
}

/// Seed the full FIFO name immediately before the shim's real removal attempt.
fn install_fifo_removal_observer(path: &Path) {
    executable(
        path,
        r#"#!/bin/sh
set -eu
for candidate in "$@"; do
    case $candidate in
        "$SHIM_TEST_ACCOUNT_DIRECTORY"/state/stderr-*)
            if [ -f "$SHIM_TEST_OBSERVATIONS/leave-stale-fifo" ]; then
                "$SHIM_TEST_REAL_RM" -f "$SHIM_TEST_OBSERVATIONS/leave-stale-fifo"
                mkfifo "$candidate"
                [ -p "$candidate" ]
                printf '%s' "$candidate" > "$SHIM_TEST_OBSERVATIONS/stale-fifo"
            fi
            if [ -f "$SHIM_TEST_OBSERVATIONS/block-fifo-removal" ]; then
                "$SHIM_TEST_REAL_RM" -f "$SHIM_TEST_OBSERVATIONS/block-fifo-removal"
                mkdir "$candidate"
                printf 'keep this directory\n' > "$candidate/keep"
                printf '%s' "$candidate" > "$SHIM_TEST_OBSERVATIONS/blocked-fifo"
            fi
            ;;
    esac
done
exec "$SHIM_TEST_REAL_RM" "$@"
"#,
    );
}

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

/// Exercise Darwin's conversion contract on either supported test host.
fn install_darwin_observers(fixture: &InstalledShim) {
    fs::write(fixture.path("observations/darwin-conversion"), b"")
        .expect("select controlled Darwin observations");
    fs::write(
        fixture.path("observations/darwin-time.py"),
        r"import locale
import os
import sys
import time

locale.setlocale(locale.LC_ALL, '')
time.tzset()
formatting = '%a %b %d %H:%M:%S %Y'
if sys.argv[1] == 'ps':
    birth = int(os.environ.get('SHIM_TEST_BIRTH', '1788957296'))
    started = time.strftime(formatting, time.localtime(birth))
    if os.environ.get('SHIM_TEST_PS_PADDING'):
        started += '    '
    print(started)
else:
    assert sys.argv[2] == sys.argv[2].rstrip(), 'date receives ps column padding'
    print(int(time.mktime(time.strptime(sys.argv[2], formatting))))
",
    )
    .expect("install timezone-sensitive Darwin conversion adapters");
    executable(
        &fixture.path("tools/uname"),
        "#!/bin/sh\nprintf 'Darwin\\n'\n",
    );
    executable(
        &fixture.path("tools/sysctl"),
        r#"#!/bin/sh
set -eu
[ "$#" -eq 2 ] && [ "$1" = -n ] && [ "$2" = kern.bootsessionuuid ] || exit 91
printf '12345678-1234-1234-1234-123456789abc\n'
"#,
    );
    executable(
        &fixture.path("tools/ps"),
        r#"#!/bin/sh
set -eu
printf '%s\000' "${LC_ALL-}" "${TZ-}" "$@" > "$SHIM_TEST_OBSERVATIONS/ps-conversion"
[ "${LC_ALL-}" = C ] || exit 91
[ "$1" = -o ] && [ "$2" = lstart= ] && [ "$3" = -p ] || exit 92
exec python3 "$SHIM_TEST_OBSERVATIONS/darwin-time.py" ps
"#,
    );
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Required);
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

/// The old whitespace-joined representation could not distinguish these invocations.
#[test]
fn one_argument_with_a_space_differs_from_two_arguments() {
    let joined = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    let separated = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    assert_cargo_result(&joined.run(&["run", "--", "a b"]));
    assert_cargo_result(&separated.run(&["run", "--", "a", "b"]));
    let joined = joined.registration();
    let separated = separated.registration();
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
    let first = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    let second = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    assert_cargo_result(&first.run(&["build"]));
    assert_cargo_result(&second.run(&["build"]));
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
fn registration_log_and_live_fifo_share_pid_and_generation() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
    assert!(pid.parse::<u32>().expect("shim pid is numeric") > 0);
    let generation = std::str::from_utf8(fields[1]).expect("generation is ASCII");
    assert_eq!(registration.name, format!("{pid}.{generation}"));
    let fifo_name = format!("stderr-{}", registration.name);
    assert_eq!(
        fs::read_to_string(fixture.path("observations/fifos"))
            .expect("cargo observes the live FIFO"),
        format!("{fifo_name}\n")
    );
    assert!(!fixture.path("capture/state").join(fifo_name).exists());
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

/// The full invocation FIFO is removed before mkfifo, even when it already exists.
#[test]
fn stale_invocation_fifo_still_publishes_registration_and_captures_stderr() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    fs::write(fixture.path("observations/leave-stale-fifo"), b"")
        .expect("seed the FIFO at the generation boundary before capture setup");
    let arguments = ["run", "--", "a b", ""];
    assert_cargo_result(&fixture.run(&arguments));
    let pid = fs::read_to_string(fixture.path("observations/shim-pid"))
        .expect("read independently recorded shim pid");
    let registration = fixture.registration();
    let fifo = fixture.path(&format!("capture/state/stderr-{}", registration.name));
    assert_eq!(
        fs::read_to_string(fixture.path("observations/stale-fifo"))
            .expect("the boundary fixture creates a real FIFO"),
        fifo.to_str().expect("fixture FIFO path is UTF-8")
    );
    let registration = fixture.registration();
    let fields = registration.fields();
    let generation = std::str::from_utf8(fields[1]).expect("ASCII generation");
    assert_eq!(registration.name, format!("{pid}.{generation}"));
    let log = std::str::from_utf8(fields[4]).expect("registration names its capture log");
    assert_eq!(
        fs::read(fixture.path("observations").join(log))
            .expect("published run actually captures cargo stderr"),
        b"registration log marker\n"
    );
    let observed = fs::read(fixture.path("observations/arguments"))
        .expect("cargo records its original arguments");
    assert_eq!(
        nul_fields(&observed),
        arguments
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
    let umask = fs::read_to_string(fixture.path("observations/umask"))
        .expect("cargo records its inherited umask");
    assert_eq!(
        u32::from_str_radix(umask.trim(), 8).expect("octal umask"),
        0o066
    );
    assert!(!fifo.exists(), "cleanup removes the replacement FIFO");
    assert!(!fixture.path("capture").join(log).exists(), "log cleanup");
    assert_eq!(
        fs::read_dir(fixture.path("capture/state/pids"))
            .expect("read registration directory after cleanup")
            .count(),
        0,
        "registration and staging cleanup"
    );
}

/// A directory at the FIFO path makes real rm fail and must leave cargo unchanged.
#[test]
fn fifo_removal_failure_preserves_original_cargo_and_unowned_directory() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    fs::write(fixture.path("observations/block-fifo-removal"), b"")
        .expect("leave a directory that rm -f cannot remove at the FIFO path");
    let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
    assert_cargo_result(&fixture.run(&arguments));
    let observed = fs::read(fixture.path("observations/arguments"))
        .expect("cargo records original arguments after removal failure");
    assert_eq!(
        nul_fields(&observed),
        arguments
            .iter()
            .map(|word| word.as_bytes())
            .collect::<Vec<_>>()
    );
    let umask = fs::read_to_string(fixture.path("observations/umask"))
        .expect("cargo records its inherited umask");
    assert_eq!(
        u32::from_str_radix(umask.trim(), 8).expect("octal umask"),
        0o066
    );
    assert_eq!(
        fs::read(fixture.path("observations/capture-settings"))
            .expect("cargo records capture exports"),
        b"\0\0\0",
        "removal failure reaches fallback before capture exports"
    );
    let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
    let directory = PathBuf::from(
        fs::read_to_string(fixture.path("observations/blocked-fifo"))
            .expect("observe blocked invocation FIFO"),
    );
    let name = directory
        .file_name()
        .expect("FIFO basename")
        .to_str()
        .expect("ASCII name");
    assert!(
        name.starts_with(&format!("stderr-{pid}.20260909-204000-")),
        "{name}"
    );
    assert_eq!(
        fs::read(directory.join("keep")).expect("preserve the unowned directory sentinel"),
        b"keep this directory\n"
    );
    assert_eq!(
        fs::read_dir(directory)
            .expect("read FIFO-path directory")
            .count(),
        1
    );
    for registrations in ["observations/pids", "capture/state/pids"] {
        assert_eq!(
            fs::read_dir(fixture.path(registrations))
                .expect("inspect registrations during cargo and after cleanup")
                .count(),
            0,
            "removal failure leaves neither publication nor staging"
        );
    }
    assert_eq!(
        fs::read_dir(fixture.path("capture"))
            .expect("read capture root")
            .count(),
        1,
        "failed setup leaves only the state directory"
    );
}

/// Publish complete bytes from an ignored same-directory staging file using ln.
#[test]
fn publication_links_a_complete_tmp_file_into_place() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
    assert!(!temporary.exists());
}

/// Reading /proc/self in setup would describe a later child instead of this shim.
#[test]
fn birth_stamp_belongs_to_the_pid_in_the_registration_filename() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Required);
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

/// Calendar time and a reused shell pid cannot reproduce a publication name.
#[test]
fn same_pid_and_calendar_second_receive_different_generations() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Required);
    fs::write(fixture.path("observations/repeat"), b"").expect("repeat in one shell");
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    fs::write(fixture.path("observations/repeat"), b"").expect("repeat in one shell");
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
    for timezone in ["UTC0", "UTC-11"] {
        let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
        install_darwin_observers(&fixture);
        let output = fixture
            .command(&["build"])
            .env("LC_ALL", "POSIX")
            .env("LANG", "C")
            .env("TZ", timezone)
            .output()
            .expect("run controlled Darwin writer");
        assert_cargo_result(&output);
        assert_eq!(fixture.registration().fields()[3], b"1788957296");
        for utility in ["date", "ps"] {
            let observed = fs::read(fixture.path(&format!("observations/{utility}-conversion")))
                .expect("observe conversion environment");
            assert_eq!(&nul_fields(&observed)[..2], [b"C".as_slice(), b"UTC0"]);
        }
        let cargo = fs::read(fixture.path("observations/locale-settings"))
            .expect("cargo records unmodified environment");
        assert_eq!(
            &nul_fields(&cargo)[..3],
            [b"POSIX".as_slice(), timezone.as_bytes(), b"C"]
        );
    }
}

/// BSD ps column padding must not reach date or contaminate cargo's capture log.
#[test]
fn darwin_birth_conversion_removes_ps_column_padding() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    install_darwin_observers(&fixture);
    let output = fixture
        .command(&["build"])
        .env("SHIM_TEST_PS_PADDING", "1")
        .output()
        .expect("run with padded Darwin ps output");
    assert_cargo_result(&output);
    let registration = fixture.registration();
    let fields = registration.fields();
    assert_eq!(fields[2], b"12345678-1234-1234-1234-123456789abc");
    assert_eq!(fields[3], b"1788957296");
    let converted = fs::read(fixture.path("observations/date-conversion"))
        .expect("observe the exact date conversion argument");
    let arguments = nul_fields(&converted);
    assert_eq!(arguments[5], b"Wed Sep 09 12:34:56 2026");
    let log = std::str::from_utf8(fields[4]).expect("registered log basename");
    assert_eq!(
        fs::read(fixture.path("observations").join(log)).expect("captured cargo bytes"),
        b"registration log marker\n"
    );
}

/// GNU coreutils ahead of /bin on PATH must not leave the birth field empty.
#[test]
fn darwin_birth_conversion_falls_back_to_gnu_date() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    install_darwin_observers(&fixture);
    fs::write(fixture.path("observations/gnu-date"), b"")
        .expect("select a GNU coreutils date on the search path");
    assert_cargo_result(&fixture.run(&["build"]));
    let registration = fixture.registration();
    let fields = registration.fields();
    assert_eq!(fields[2], b"12345678-1234-1234-1234-123456789abc");
    assert_eq!(fields[3], b"1788957296");
    let rejected = fs::read(fixture.path("observations/date-rejected"))
        .expect("the BSD form is attempted first");
    assert_eq!(nul_fields(&rejected)[0], b"-j");
    let converted = fs::read(fixture.path("observations/date-conversion"))
        .expect("observe the GNU conversion arguments");
    assert_eq!(
        nul_fields(&converted),
        [
            b"C".as_slice(),
            b"UTC0",
            b"-u",
            b"-d",
            b"Wed Sep 09 12:34:56 2026",
            b"+%s"
        ]
    );
}

/// The two occurrences of 01:30 at DST fallback must publish different epoch seconds.
#[test]
fn darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback() {
    let births = ["1793511000", "1793514600"];
    let mut wall_times = Vec::new();
    let mut published = Vec::new();
    for birth in births {
        let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
        install_darwin_observers(&fixture);
        let timezone = "EST5EDT,M3.2.0,M11.1.0";
        let ambiguous = Command::new("python3")
            .arg(fixture.path("observations/darwin-time.py"))
            .arg("ps")
            .env("LC_ALL", "C")
            .env("TZ", timezone)
            .env("SHIM_TEST_BIRTH", birth)
            .output()
            .expect("format the repeated local wall-clock time independently");
        assert!(ambiguous.status.success());
        wall_times.push(ambiguous.stdout);
        let output = fixture
            .command(&["build"])
            .env("LC_ALL", "POSIX")
            .env("LANG", "C")
            .env("TZ", timezone)
            .env("SHIM_TEST_BIRTH", birth)
            .output()
            .expect("publish during the repeated DST hour");
        assert_cargo_result(&output);
        let registration = fixture.registration();
        assert_eq!(registration.fields()[3], birth.as_bytes());
        published.push(registration.fields()[3].to_vec());
        let cargo = fs::read(fixture.path("observations/locale-settings"))
            .expect("cargo records its inherited timezone");
        assert_eq!(
            &nul_fields(&cargo)[..3],
            [b"POSIX".as_slice(), timezone.as_bytes(), b"C"]
        );
    }
    assert_eq!(
        wall_times[0], wall_times[1],
        "fixture crosses the repeated hour"
    );
    assert_ne!(published[0], published[1]);
}

/// Partial stdout from a failed conversion is never published as birth evidence.
#[test]
fn failed_darwin_birth_conversion_publishes_empty_identity_field() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
    install_darwin_observers(&fixture);
    fs::write(fixture.path("observations/fail-conversion"), b"")
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
    for target in ["shim", "setup"] {
        let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
        fs::write(
            fixture.path("observations/signal-after-publication"),
            target,
        )
        .expect("request TERM before ln returns to the setup shell");
        let output = fixture.run(&["build"]);
        assert_eq!(
            output.status.code(),
            Some(143),
            "TERM delivered to {target}"
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert!(fixture.path("observations/staged-registration").exists());
        assert!(
            !fixture.path("observations/arguments").exists(),
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

/// Each shim sweeps dead registrations, staging, logs, and FIFOs only in its account.
#[test]
fn dead_pid_captures_are_reaped_before_publication_without_touching_another_account() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
            let fifo = directory.join(format!("state/stderr-{name}"));
            assert!(
                Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .expect("seed stale FIFO")
                    .success()
            );
            paths.push(fifo);
        }
        if directory == &account {
            removed = paths;
        } else {
            retained = paths;
        }
    }
    let unrelated_fifo = account.join(format!("state/stderr-{pid}.unregistered-generation"));
    assert!(
        Command::new("mkfifo")
            .arg(&unrelated_fifo)
            .status()
            .expect("seed another invocation FIFO")
            .success()
    );
    retained.push(unrelated_fifo);
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
            format!("state/stderr-{pid}.{generation}"),
        ] {
            assert!(
                !fixture.path("capture").join(&relative).exists(),
                "predecessor artifact survives: {relative}"
            );
        }
    }
    assert_eq!(
        fs::read_to_string(fixture.path("observations/fifos"))
            .expect("fresh FIFO observed during cargo"),
        format!("stderr-{}\n", fresh.name)
    );
}

/// Publishing v2 alongside an older shim must leave that shim's registration intact.
#[test]
fn legacy_registration_and_log_survive_a_new_shim_run() {
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
    let fixture = InstalledShim::new(BirthTimeSeparation::Unnecessary);
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
