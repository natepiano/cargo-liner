//! Exercise the installed shell shim's registration protocol without crate linkage.

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;
    use std::process::Stdio;

    use tempfile::TempDir;

    /// Own every path the copied shim and stand-in cargo can write.
    struct InstalledShim {
        /// Keep the toolchain, capture root, and observations alive through assertions.
        directory: TempDir,
    }

    impl InstalledShim {
        /// Install the repository script beside cargo that snapshots its live registration.
        fn new() -> Self {
            let fixture = Self {
                directory: tempfile::tempdir().expect("create isolated toolchain"),
            };
            for directory in [
                "bin",
                "tools",
                "observations",
                "home/work tree\twith\nlines",
            ] {
                fs::create_dir_all(fixture.path(directory)).expect("create fixture directory");
            }
            fs::copy(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/cargo-capture-shim.sh"),
                fixture.path("bin/cargo"),
            )
            .expect("copy shim into installed toolchain layout");
            install_cargo(&fixture.path("bin/cargo-tile-real"));
            install_date(&fixture.path("tools/date"));
            install_publication_observer(&fixture.path("tools/ln"));
            fixture
        }

        /// Resolve fixture paths without depending on the developer's home or capture root.
        fn path(&self, relative: &str) -> PathBuf { self.directory.path().join(relative) }

        /// Run with an independently recorded shim pid and a restrictive caller umask.
        fn run(&self, arguments: &[&str]) -> Output {
            let real_link = Command::new("sh")
                .args(["-c", "command -v ln"])
                .output()
                .expect("locate system ln before changing the child PATH");
            assert!(real_link.status.success());
            let real_link = String::from_utf8(real_link.stdout).expect("system ln path is UTF-8");
            let mut search_path = vec![self.path("tools")];
            search_path.extend(std::env::split_paths(
                &std::env::var_os("PATH").expect("test runner supplies PATH"),
            ));
            Command::new("sh")
                .args([
                    "-c",
                    // The delay separates the shim's birth from setup children even on
                    // systems exposing process start time at one-second resolution.
                    "umask 0066; printf '%s' \"$$\" > \"$SHIM_TEST_OBSERVATIONS/shim-pid\"; sleep 1; exec sh \"$@\"",
                    "shim-registration-test",
                ])
                .arg(self.path("bin/cargo"))
                .args(arguments)
                .current_dir(self.path("home/work tree\twith\nlines"))
                .env("HOME", self.path("home"))
                .env("CARGO_TILE_ROOT", self.path("capture"))
                .env("SHIM_TEST_OBSERVATIONS", self.path("observations"))
                .env("SHIM_TEST_REAL_LN", real_link.trim_end())
                .env("SHIM_TEST_GENERATION", "20260909-204000")
                .env("PATH", std::env::join_paths(search_path).expect("join child PATH"))
                .env_remove("CARGOTILE_NESTED")
                .env_remove("CARGO_TERM_PROGRESS_WHEN")
                .env_remove("CARGO_TERM_PROGRESS_WIDTH")
                .stdin(Stdio::null())
                .output()
                .expect("execute copied shim with stand-in cargo")
        }

        /// Select the sole v2 registration copied while cargo was alive.
        fn registration(&self) -> PublishedRegistration {
            let entries: Vec<_> = fs::read_dir(self.path("observations/pids"))
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
shim_pid=$(cat "$observations/shim-pid")
case $(uname -s) in
    Linux)
        cat "/proc/$shim_pid/stat" > "$observations/shim-stat"
        cat /proc/sys/kernel/random/boot_id > "$observations/boot"
        ;;
    Darwin)
        ps -o lstart= -p "$shim_pid" > "$observations/birth"
        sysctl -n kern.boottime > "$observations/boot"
        ;;
esac
if [ -d "$CARGO_TILE_ROOT/state/pids" ]; then
    cp -R "$CARGO_TILE_ROOT/state/pids" "$observations/pids"
fi
printf 'cargo stdout\n'
printf 'registration log marker\n' >&2
if [ -n "${CARGOTILE_NESTED-}" ]; then
    remaining=200
    while [ "$remaining" -gt 0 ]; do
        for log in "$CARGO_TILE_ROOT"/run-*.log; do
            if [ -f "$log" ] && grep -q 'registration log marker' "$log"; then
                cp "$log" "$observations/${log##*/}"
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

    /// Freeze the generation and optionally occupy the exact upcoming publication name.
    fn install_date(path: &Path) {
        executable(
            path,
            r#"#!/bin/sh
set -eu
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-name" ]; then
    shim_pid=$(cat "$SHIM_TEST_OBSERVATIONS/shim-pid")
    mkdir -p "$CARGO_TILE_ROOT/state/pids"
    printf 'previous registration\n' > "$CARGO_TILE_ROOT/state/pids/$shim_pid.$SHIM_TEST_GENERATION"
    if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-log" ]; then
        printf 'previous log\n' > "$CARGO_TILE_ROOT/run-$SHIM_TEST_GENERATION-$shim_pid.log"
    fi
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-directory" ]; then
    shim_pid=$(cat "$SHIM_TEST_OBSERVATIONS/shim-pid")
    mkdir -p "$CARGO_TILE_ROOT/state/pids/$shim_pid.$SHIM_TEST_GENERATION"
    printf 'existing directory\n' > "$CARGO_TILE_ROOT/state/pids/$shim_pid.$SHIM_TEST_GENERATION/keep"
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/leave-stale-fifo" ]; then
    shim_pid=$(cat "$SHIM_TEST_OBSERVATIONS/shim-pid")
    mkdir -p "$CARGO_TILE_ROOT/state"
    fifo="$CARGO_TILE_ROOT/state/stderr-$shim_pid"
    mkfifo "$fifo"
    [ -p "$fifo" ]
    printf '%s' "$fifo" > "$SHIM_TEST_OBSERVATIONS/stale-fifo"
fi
if [ -f "$SHIM_TEST_OBSERVATIONS/block-fifo-removal" ]; then
    shim_pid=$(cat "$SHIM_TEST_OBSERVATIONS/shim-pid")
    directory="$CARGO_TILE_ROOT/state/stderr-$shim_pid"
    mkdir -p "$directory"
    printf 'keep this directory\n' > "$directory/keep"
fi
printf '%s\n' "$SHIM_TEST_GENERATION"
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
if [ -f "$SHIM_TEST_OBSERVATIONS/occupy-at-publication" ]; then
    printf 'previous registration\n' > "$2"
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
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");
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
        let fixture = InstalledShim::new();
        let arguments = ["run", "--", "a b", "a\tb", "a\nb", "'\"\\", "雪", ""];
        assert_cargo_result(&fixture.run(&arguments));
        let registration = fixture.registration();
        let fields = registration.fields();
        let boot =
            fs::read_to_string(fixture.path("observations/boot")).expect("read boot identity");
        let birth = observed_birth(&fixture);
        let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
        let log = format!("run-20260909-204000-{pid}.log");
        let count = arguments.len().to_string();
        assert_eq!(
            &fields[..7],
            [
                b"cargo-tile-v2".as_slice(),
                b"20260909-204000",
                boot.trim_end_matches('\n').as_bytes(),
                birth.as_bytes(),
                log.as_bytes(),
                b"~/work tree\twith\nlines",
                count.as_bytes(),
            ]
        );
        assert_eq!(
            &fields[7..],
            arguments
                .iter()
                .map(|word| word.as_bytes())
                .collect::<Vec<_>>()
        );
        let observed = fs::read(fixture.path("observations/arguments")).expect("read cargo argv");
        assert_eq!(nul_fields(&observed), &fields[7..]);
    }

    /// The old whitespace-joined representation could not distinguish these invocations.
    #[test]
    fn one_argument_with_a_space_differs_from_two_arguments() {
        let joined = InstalledShim::new();
        let separated = InstalledShim::new();
        assert_cargo_result(&joined.run(&["run", "--", "a b"]));
        assert_cargo_result(&separated.run(&["run", "--", "a", "b"]));
        let joined = joined.registration();
        let separated = separated.registration();
        let joined_fields = joined.fields();
        let separated_fields = separated.fields();
        assert_eq!(joined_fields[6], b"3");
        assert_eq!(separated_fields[6], b"4");
        assert_eq!(&joined_fields[7..], [b"run".as_slice(), b"--", b"a b"]);
        assert_eq!(
            &separated_fields[7..],
            [b"run".as_slice(), b"--", b"a", b"b"]
        );
        assert_ne!(&joined_fields[7..], &separated_fields[7..]);
    }

    /// The filename and log field identify the same live run and are removed on exit.
    #[test]
    fn registration_filename_and_written_log_share_pid_and_generation() {
        let fixture = InstalledShim::new();
        assert_cargo_result(&fixture.run(&["build"]));
        let registration = fixture.registration();
        let fields = registration.fields();
        let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
        assert!(pid.parse::<u32>().expect("shim pid is numeric") > 0);
        let generation = std::str::from_utf8(fields[1]).expect("generation is ASCII");
        assert_eq!(registration.name, format!("{pid}.{generation}"));
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

    /// F002: a FIFO left by a dead run with this pid must not disable capture.
    #[test]
    fn stale_pid_fifo_still_publishes_registration_and_captures_stderr() {
        let fixture = InstalledShim::new();
        fs::write(fixture.path("observations/leave-stale-fifo"), b"")
            .expect("seed the FIFO at the generation boundary before capture setup");
        let arguments = ["run", "--", "a b", ""];
        assert_cargo_result(&fixture.run(&arguments));
        let pid = fs::read_to_string(fixture.path("observations/shim-pid"))
            .expect("read independently recorded shim pid");
        let fifo = fixture.path(&format!("capture/state/stderr-{pid}"));
        assert_eq!(
            fs::read_to_string(fixture.path("observations/stale-fifo"))
                .expect("the boundary fixture creates a real FIFO"),
            fifo.to_str().expect("fixture FIFO path is UTF-8")
        );
        let registration = fixture.registration();
        let fields = registration.fields();
        assert_eq!(registration.name, format!("{pid}.20260909-204000"));
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
        let fixture = InstalledShim::new();
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
        let directory = fixture.path(&format!("capture/state/stderr-{pid}"));
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
        let fixture = InstalledShim::new();
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
            fs::read(fixture.path("observations/staged-registration"))
                .expect("observe staging bytes"),
            registration.contents,
            "the full registration exists before its final name becomes visible"
        );
        assert!(!temporary.exists());
    }

    /// Reading /proc/self in setup would describe a later child instead of this shim.
    #[test]
    fn birth_stamp_belongs_to_the_pid_in_the_registration_filename() {
        let fixture = InstalledShim::new();
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
        assert!(!fields[2].is_empty(), "this host exposes its boot identity");
    }

    /// A competing publisher can take the name after setup's preliminary existence check.
    #[test]
    fn occupied_registration_runs_original_cargo_without_replacing_owned_files() {
        let fixture = InstalledShim::new();
        fs::write(fixture.path("observations/occupy-at-publication"), b"")
            .expect("occupy the name immediately before the real ln attempts publication");
        let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
        assert_cargo_result(&fixture.run(&arguments));
        let observed =
            fs::read(fixture.path("observations/arguments")).expect("read fallback argv");
        assert_eq!(
            nul_fields(&observed),
            arguments
                .iter()
                .map(|word| word.as_bytes())
                .collect::<Vec<_>>()
        );
        let umask =
            fs::read_to_string(fixture.path("observations/umask")).expect("read cargo umask");
        assert_eq!(
            u32::from_str_radix(umask.trim(), 8).expect("octal umask"),
            0o066
        );
        let settings =
            fs::read(fixture.path("observations/capture-settings")).expect("read capture settings");
        assert_eq!(nul_fields(&settings), [b"".as_slice(), b"", b""]);
        let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
        assert_eq!(
            fs::read(fixture.path(&format!("capture/state/pids/{pid}.20260909-204000")))
                .expect("existing registration survives failed publication and cleanup"),
            b"previous registration\n"
        );
        assert!(
            !fixture
                .path(&format!("capture/run-20260909-204000-{pid}.log"))
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
        let fixture = InstalledShim::new();
        fs::write(fixture.path("observations/occupy-name"), b"").expect("request a name collision");
        fs::write(fixture.path("observations/occupy-log"), b"").expect("request an existing log");
        assert_cargo_result(&fixture.run(&["build"]));
        let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
        assert_eq!(
            fs::read(fixture.path(&format!("capture/state/pids/{pid}.20260909-204000")))
                .expect("existing registration survives log collision"),
            b"previous registration\n"
        );
        assert_eq!(
            fs::read(fixture.path(&format!("capture/run-20260909-204000-{pid}.log")))
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
        let fixture = InstalledShim::new();
        fs::write(fixture.path("observations/occupy-directory"), b"")
            .expect("request a directory at the publication name");
        assert_cargo_result(&fixture.run(&["check", "--quiet", "--message-format=json"]));
        let settings =
            fs::read(fixture.path("observations/capture-settings")).expect("read capture settings");
        assert_eq!(nul_fields(&settings), [b"".as_slice(), b"", b""]);
        let pid = fs::read_to_string(fixture.path("observations/shim-pid")).expect("read shim pid");
        let occupied = fixture.path(&format!("capture/state/pids/{pid}.20260909-204000"));
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
            let fixture = InstalledShim::new();
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

    /// Publishing v2 alongside an older shim must leave that shim's registration intact.
    #[test]
    fn legacy_registration_and_log_survive_a_new_shim_run() {
        let fixture = InstalledShim::new();
        fs::create_dir_all(fixture.path("capture/state/pids"))
            .expect("create mixed-format directory");
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
}
