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

    /// Exercise the built binary using actual shim publications and a reconstructed PTY screen.
    const READER_SCENARIO_SCRIPT: &str = r#"import errno
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import termios
import time

root = Path(sys.argv[1]).resolve()
binary, source, scenario = sys.argv[2:]
home = root / 'home'
work = home / ('repair-group-' + root.name)
capture = root / 'capture'
pids = capture / 'state/pids'
bin_directory = root / 'bin'
for directory in (work, pids, bin_directory, root / 'config/cargo-tile',
                  home / 'Library/Application Support/cargo-tile', root / 'rustup/toolchains'):
    directory.mkdir(parents=True)
configuration = '[capture]\nauto_install = false\n[tiles]\ninitial_rows = 100\n'
if scenario == 'excluded':
    configuration += '[commands]\nexcluded = ["clippy"]\n'
elif scenario == 'exec-excluded':
    configuration += '[commands]\nexcluded = ["run"]\n'
for directory in (root / 'config/cargo-tile', home / 'Library/Application Support/cargo-tile'):
    (directory / 'config.toml').write_text(configuration)
shutil.copyfile(source, bin_directory / 'cargo')
shutil.copyfile(shutil.which('sh'), bin_directory / 'cargo-tile-real')
(bin_directory / 'cargo-tile-real').chmod(0o755)
(work / 'build').write_text('''printf '%s\\0' "$LC_ALL" "$TZ" "$LANG" "$HOME" > "$OBSERVED/environment"
printf '%s' "$$" > "$OBSERVED/cargo-pid"
printf '%s' "${CARGOTILE_NESTED-}" > "$OBSERVED/enclosing-pid"
printf 'Blocking waiting for file lock on build directory\\n' >&2
if [ -n "${NESTED_WORK-}" ]; then
    for command in check test; do
        (
            cd "$NESTED_WORK" || exit 93
            OBSERVED="$OBSERVED/$command" NESTED_WORK= sh "$CARGO" "$command" "$NESTED_MARKER-$command"
        ) &
    done
fi
remaining=1000
while [ ! -f "$OBSERVED/release" ] && [ "$remaining" -gt 0 ]; do
    if [ -f "$OBSERVED/pulse" ]; then
        printf 'writer remains captured after reader scan\\n' >&2
        rm "$OBSERVED/pulse"
    fi
    sleep 0.02
    remaining=$((remaining - 1))
done
[ "$remaining" -gt 0 ] || exit 92
wait
exit 37
''')
shutil.copyfile(work / 'build', work / 'clippy')
shutil.copyfile(work / 'build', work / 'application')
(work / 'run').write_text('exec sh "$APPLICATION"\n')

environment = dict(os.environ)
locales = subprocess.run(['locale', '-a'], check=True, capture_output=True, text=True).stdout.split()
writer_locale = next((name for name in locales if name not in ('C', 'POSIX')
                      and not name.lower().startswith('c.')), 'POSIX')
for key in ('CARGOTILE_NESTED', 'CARGO_TILE_FRAME_LOG', 'CARGO_TERM_PROGRESS_WHEN',
            'CARGO_TERM_PROGRESS_WIDTH', 'ITERM_SESSION_ID', 'NESTED_WORK', 'NESTED_MARKER'):
    environment.pop(key, None)
environment.update(HOME=str(home), XDG_CONFIG_HOME=str(root / 'config'),
                   XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'),
                   RUSTUP_HOME=str(root / 'rustup'), CARGO_TILE_ROOT=str(capture),
                   LC_ALL=writer_locale, LANG=writer_locale,
                   TZ='EST5EDT,M3.2.0,M11.1.0', TERM='xterm-256color')
writers = []
reader = None
terminal = None
transcript = bytearray()

def wait_for(predicate, description):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(description)

def start_writer(name, writer_home, command='build', nested_directory=None):
    name += '-' + root.name
    observations = root / name
    observations.mkdir()
    child_environment = dict(environment, HOME=str(writer_home), OBSERVED=str(observations))
    if command == 'run':
        child_environment['APPLICATION'] = str(work / 'application')
    if nested_directory is not None:
        nested_directory.mkdir()
        for nested_command in ('check', 'test'):
            (observations / nested_command).mkdir()
            shutil.copyfile(work / 'build', nested_directory / nested_command)
        child_environment.update(NESTED_WORK=str(nested_directory),
                                 NESTED_MARKER='probe-nested-' + root.name)
    with (observations / 'output').open('wb') as output:
        child = subprocess.Popen(['sh', str(bin_directory / 'cargo'), command, name],
                                 cwd=work, env=child_environment, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=output, start_new_session=True)
    writers.append((child, observations))
    wait_for(lambda: (observations / 'cargo-pid').exists(), 'cargo does not start')
    wait_for(lambda: any(pids.glob(str(child.pid) + '.*')), 'shim does not publish')
    registration = next(pids.glob(str(child.pid) + '.*'))
    fields = registration.read_bytes().split(b'\0')
    log = capture / os.fsdecode(fields[4])
    wait_for(lambda: log.exists() and b'Blocking waiting' in log.read_bytes(),
             'writer does not capture progress')
    inherited = (observations / 'environment').read_bytes().split(b'\0')
    assert inherited == [writer_locale.encode(), environment['TZ'].encode(),
                         writer_locale.encode(), os.fsencode(writer_home), b'']
    if nested_directory is not None:
        for nested_command in ('check', 'test'):
            nested = observations / nested_command
            wait_for(lambda: (nested / 'enclosing-pid').exists()
                     and (nested / 'enclosing-pid').read_text() == str(child.pid),
                     'nested command does not inherit the enclosing capture')
            nested_pid = (nested / 'cargo-pid').read_text()
            assert not list(pids.glob(nested_pid + '.*')), 'nested command publishes a capture'
        if command == 'run':
            application_pid = (observations / 'cargo-pid').read_text()
            application = subprocess.run(['ps', '-p', application_pid, '-o', 'comm=', '-o', 'args='],
                                         check=True, capture_output=True, text=True).stdout.strip()
            assert 'cargo' not in Path(application.split()[0]).name, application
            assert str(work / 'application') in application, application
            for nested_command in ('check', 'test'):
                descendant = (observations / nested_command / 'cargo-pid').read_text()
                ancestry = []
                while descendant != str(child.pid):
                    assert descendant not in ancestry and descendant != '1', ancestry
                    ancestry.append(descendant)
                    descendant = subprocess.run(['ps', '-p', descendant, '-o', 'ppid='], check=True,
                                                capture_output=True, text=True).stdout.strip()
                assert application_pid in ancestry, ancestry
    return child, observations, registration, fields, log

def end_writer(writer):
    child, observations = writer[:2]
    for nested_command in ('check', 'test'):
        if (observations / nested_command).is_dir():
            (observations / nested_command / 'release').touch()
    (observations / 'release').touch()
    assert child.wait(timeout=5) == 37, (observations / 'output').read_text()

def read_terminal(duration):
    deadline = time.monotonic() + duration
    while time.monotonic() < deadline:
        if select.select([terminal], [], [], min(0.05, max(0, deadline - time.monotonic())))[0]:
            try:
                data = os.read(terminal, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not data:
                break
            transcript.extend(data)

def screen():
    # Ratatui positions each changed run with CSI row;column H. Reconstruct cells
    # so repeated refreshes cannot manufacture extra headings or stale progress.
    cells = [[' '] * 300 for _ in range(100)]
    row = column = 0
    tokens = re.split(r'(\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\))',
                      transcript.decode('utf-8', 'replace'))
    for token in tokens:
        if token.startswith('\x1b['):
            command = token[-1]
            parameters = token[2:-1]
            if command in 'Hf':
                position = parameters.split(';')
                row = int(position[0] or 1) - 1
                column = int(position[1] or 1) - 1 if len(position) > 1 else 0
            elif command == 'J' and parameters in ('2', '3'):
                cells = [[' '] * 300 for _ in range(100)]
            elif command == 'K':
                if 0 <= row < len(cells):
                    cells[row][column:] = [' '] * (300 - column)
            elif command == 'C':
                column += int(parameters or 1)
            elif command == 'G':
                column = int(parameters or 1) - 1
            continue
        if token.startswith('\x1b]'):
            continue
        for character in token:
            if character == '\r':
                column = 0
            elif character == '\n':
                row += 1
            elif character >= ' ':
                if 0 <= row < 100 and 0 <= column < 300:
                    cells[row][column] = character
                column += 1
    return '\n'.join(''.join(line).rstrip() for line in cells)

def command_panes(rendered):
    lines = rendered.splitlines()
    for beginning, line in enumerate(lines):
        if 'parent' not in line or 'command' not in line:
            continue
        commands = []
        for line in lines[beginning + 1:]:
            if re.match(r'^\s*[└├╰╞╘].*[─━═]{3}', line):
                break
            commands.append(line)
        yield commands

def fixture_pane(rendered, markers):
    matches = [commands for commands in command_panes(rendered)
               if all(any(marker in line for line in commands) for marker in markers)]
    assert len(matches) == 1, 'fixture must occupy one command pane\n' + rendered
    return matches[0]

try:
    first = start_writer('probe-first', home)
    retained = []
    removed = []
    if scenario in ('grouping', 'grouping-earlier-pane'):
        second = start_writer('probe-second', root / 'custom-home')
    elif scenario in ('nested', 'excluded', 'exec-nested', 'exec-excluded'):
        nested_directory = home / ('nested-directory-' + root.name)
        enclosing_command = {'nested': 'build', 'excluded': 'clippy',
                             'exec-nested': 'run', 'exec-excluded': 'run'}[scenario]
        enclosing = start_writer('probe-enclosing', home,
                                 enclosing_command,
                                 nested_directory)
        retained.extend((enclosing[2], enclosing[4]))
        if scenario in ('excluded', 'exec-excluded'):
            # A removed sibling proves cleanup ran while the excluded capture
            # and its still-running nested commands retained their artifacts.
            ended = start_writer('probe-ended', home)
            contents = ended[2].read_bytes()
            end_writer(ended)
            ended[2].write_bytes(contents)
            ended[4].write_bytes(b'Blocking waiting for file lock on build directory\n')
            removed.extend((ended[2], ended[4]))
    elif scenario == 'staging':
        child, observations, registration, fields, log = first
        contents = registration.read_bytes()
        ended_staging = Path(str(registration) + '.tmp')
        end_writer(first)
        assert not registration.exists(), 'writer cleanup should finish first'
        ended_staging.write_bytes(contents)
        log.write_bytes(b'Blocking waiting for file lock on build directory\n')
        removed.extend((ended_staging, log))
        # A complete record with unavailable identity and an incomplete record
        # both remain; neither may prevent the ended sibling from being swept.
        for suffix, birth in (('unknown', b''), ('malformed', b'bad')):
            generation = fields[1] + b'-' + suffix.encode()
            name = pids / (str(child.pid) + '.' + generation.decode() + '.tmp')
            sibling = list(fields)
            sibling[1], sibling[3] = generation, birth
            sibling[4] = b'run-' + generation + b'-' + str(child.pid).encode() + b'.log'
            sibling_log = capture / os.fsdecode(sibling[4])
            name.write_bytes(b'\0'.join(sibling) if suffix == 'unknown' else b'incomplete\0')
            sibling_log.write_bytes(b'preserve unknown writer\n')
            retained.extend((name, sibling_log))
        live = start_writer('probe-live', home)
        live_staging = Path(str(live[2]) + '.tmp')
        live[2].rename(live_staging)
        retained.extend((live_staging, live[4]))
    elif scenario == 'forged':
        # A matching record under a different live pid has no kernel identity
        # authority. This exercises the external record boundary of the identity check.
        time.sleep(1.1)  # Darwin exposes only whole seconds in the wire protocol.
        live = start_writer('probe-victim', home)
        forged = list(first[3])
        forged[1] += b'-forged'
        forged[4] = b'run-' + forged[1] + b'-' + str(live[0].pid).encode() + b'.log'
        forged_name = pids / (str(live[0].pid) + '.' + forged[1].decode())
        forged_log = capture / os.fsdecode(forged[4])
        forged_name.write_bytes(b'\0'.join(forged))
        forged_log.write_bytes(b'Blocking waiting for file lock on build directory\n')
        removed.extend((forged_name, forged_log))
        retained.extend((first[2], first[4], live[2], live[4]))
    elif scenario in ('ambiguous-generation', 'unverifiable-generation'):
        # Duplicate one live writer's wire identity: this recreates competing
        # generations without depending on the kernel to reuse a pid in one second.
        competing = list(first[3])
        competing[1] = b'0000-competing-generation'
        assert competing[1] < first[3][1], 'conflicting generation must sort first'
        competing[4] = b'run-' + competing[1] + b'-' + str(first[0].pid).encode() + b'.log'
        competing_directory = home / ('competing-directory-' + root.name)
        competing_directory.mkdir()
        competing[5] = os.fsencode(competing_directory)
        competing_marker = 'probe-competing-' + root.name
        competing[8:] = [b'test', competing_marker.encode(), b'']
        if scenario == 'unverifiable-generation':
            competing[3] = b''
        competing_name = pids / (str(first[0].pid) + '.' + competing[1].decode())
        competing_log = capture / os.fsdecode(competing[4])
        competing_name.write_bytes(b'\0'.join(competing))
        competing_log.write_bytes(b'PASS [0.010s] (7/13) competing-test\n')
        retained.extend((first[2], first[4], competing_name, competing_log))

    reader_environment = dict(environment, LC_ALL='C', LANG='POSIX', TZ='UTC-11')
    reader, terminal = pty.fork()
    if reader == 0:
        fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', 100, 300, 0, 0))
        os.chdir(root)
        os.execve(binary, [binary], reader_environment)
    def reader_has_scanned():
        read_terminal(0.1)
        rendered = screen()
        marker = live[1].name if scenario == 'staging' else first[1].name
        return marker in rendered and 'summary' in rendered
    wait_for(reader_has_scanned, 'production reader does not display the live cargo row')
    read_terminal(1)
    rendered = screen()
    assert 'summary' in rendered, rendered
    if scenario in ('locale', 'grouping', 'grouping-earlier-pane', 'forged'):
        assert first[1].name in rendered, rendered
        assert any(first[1].name in line and 'blocked' in line
                   for line in rendered.splitlines()), rendered
        assert first[2].exists() and first[4].exists(), 'live record is removed by reader'
    if scenario in ('grouping', 'grouping-earlier-pane'):
        # Count this directory only inside the command pane, which is the one
        # that lists these writers: they run under the test binary, and the
        # summary pane carries the outermost invocation of each directory
        # rather than the nested ones. Other panes can choose a different
        # display label for the same directory identity.
        if scenario == 'grouping-earlier-pane':
            # Prepend a separate pane to the actual production screen so this
            # parser regression never depends on the host's live command order.
            rendered = ('│ pid parent command\n│ unrelated-earlier-pane\n└────\n'
                        + rendered)
            assert 'unrelated-earlier-pane' in '\n'.join(next(command_panes(rendered)))
        commands = fixture_pane(rendered, (first[1].name, second[1].name))
        assert sum(work.name in line for line in commands) == 1, rendered
    if scenario in ('nested', 'excluded', 'exec-nested', 'exec-excluded'):
        markers = ['probe-nested-' + root.name + '-' + command for command in ('check', 'test')]
        def nested_rows_are_visible():
            read_terminal(0.1)
            return all(marker in screen() for marker in markers)
        wait_for(nested_rows_are_visible, 'nested commands merge or disappear from the reader')
        rendered = screen()
        commands = fixture_pane(rendered, (first[1].name, *markers))
        assert any(nested_directory.name in line for line in commands), rendered
        for marker, command in zip(markers, ('check', 'test')):
            rows = [line for line in commands if marker in line]
            assert len(rows) == 1, 'nested invocation does not retain one row\n' + rendered
            assert 'blocked' in rows[0] and 'cargo ' + command + ' ' + marker in rows[0], rendered
            nested_pid = (enclosing[1] / command / 'cargo-pid').read_text()
            assert re.match(r'^\s*│\s*' + nested_pid + r'\s', rows[0]), rendered
        if scenario == 'nested':
            assert sum(enclosing[1].name in line for line in commands) == 1, rendered
        elif scenario in ('excluded', 'exec-excluded'):
            assert not any(enclosing[1].name in line for pane in command_panes(rendered)
                           for line in pane), 'excluded command becomes a row\n' + rendered
            assert enclosing[0].poll() is None, 'excluded writer ends before cleanup assertions'
            # Observe another captured write after the reader has pruned the ended
            # sibling; a mere retained empty filename would not prove live capture.
            assert all(not path.exists() for path in removed), 'reader has not swept the sibling'
            (enclosing[1] / 'pulse').touch()
            wait_for(lambda: enclosing[4].exists()
                     and b'writer remains captured after reader scan' in enclosing[4].read_bytes(),
                     'excluded live command loses its capture after the sweep')
    if scenario == 'staging':
        assert live[1].name in rendered, rendered
        assert not any(live[1].name in line and 'blocked' in line
                       for line in rendered.splitlines()), rendered
    if scenario in ('ambiguous-generation', 'unverifiable-generation'):
        commands = fixture_pane(rendered, (first[1].name,))
        rows = [line for line in commands if first[1].name in line]
        assert len(rows) == 1, 'ambiguous generations duplicate the live row\n' + rendered
        assert 'cargo build ' + first[1].name in rows[0], rendered
        assert re.match(r'^\s*│\s*' + (first[1] / 'cargo-pid').read_text() + r'\s', rows[0]), rendered
        assert 'blocked' not in rows[0], rendered
        headings = [line for line in commands if work.name in line]
        assert len(headings) == 1 and 'testing' not in headings[0], rendered
        assert competing_marker not in rendered and competing_directory.name not in rendered, rendered
    for path in removed:
        assert not path.exists(), 'reader retains ended artifact: ' + str(path) + '\n' + rendered
    for path in retained:
        assert path.exists(), 'reader removes unknown or live artifact: ' + str(path)
    if scenario in ('ambiguous-generation', 'unverifiable-generation'):
        competing_name.unlink()
        competing_log.unlink()
        def unambiguous_progress_recovers():
            read_terminal(0.1)
            return any(first[1].name in line and 'blocked' in line
                       for line in screen().splitlines())
        wait_for(unambiguous_progress_recovers,
                 'remaining capture does not recover after generation ambiguity ends')
        assert first[0].poll() is None and first[2].exists() and first[4].exists()
        rendered = screen()
    print(rendered)
finally:
    try:
        if reader is not None and reader != 0:
            finished, status = os.waitpid(reader, os.WNOHANG)
            if not finished:
                os.write(terminal, b'q')
                read_terminal(0.3)
                finished, status = os.waitpid(reader, os.WNOHANG)
                if not finished:
                    os.kill(reader, signal.SIGTERM)
                    os.waitpid(reader, 0)
            os.close(terminal)
    finally:
        for child, observations in writers:
            for nested_command in ('check', 'test'):
                if (observations / nested_command).is_dir():
                    (observations / nested_command / 'release').touch()
            (observations / 'release').touch()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
"#;

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
        fn path(&self, relative: &str) -> PathBuf {
            self.directory
                .path()
                .canonicalize()
                .expect("physical fixture root")
                .join(relative)
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
                .args(["-c", "command -v ln; command -v date"])
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
                    // The delay separates the shim's birth from setup children even on
                    // systems exposing process start time at one-second resolution.
                    r#"umask 0066
printf '%s' "$$" > "$SHIM_TEST_OBSERVATIONS/shim-pid"
sleep 1
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
                .env("CARGO_TILE_ROOT", self.path("capture"))
                .env("SHIM_TEST_OBSERVATIONS", self.path("observations"))
                .env(
                    "SHIM_TEST_REAL_LN",
                    utilities.next().expect("system ln path"),
                )
                .env(
                    "SHIM_TEST_REAL_DATE",
                    utilities.next().expect("system date path"),
                )
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
        LC_ALL=C TZ=UTC0 "$SHIM_TEST_REAL_DATE" -j -f '%a %b %e %H:%M:%S %Y' "$started" +%s > "$observations/birth"
        sysctl -n kern.boottime > "$observations/boot"
        ;;
esac
if [ -n "${SHIM_TEST_PENDING_DELETE-}" ] && [ -f "$SHIM_TEST_PENDING_DELETE" ]; then
    old_registration=$(sed -n '1p' "$SHIM_TEST_PENDING_DELETE")
    old_log=$(sed -n '2p' "$SHIM_TEST_PENDING_DELETE")
    rm -f "$old_registration" "$old_log"
    printf 'delayed unlink attempted\n' > "$observations/delayed-delete"
fi
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
                if [ -n "${SHIM_TEST_PENDING_DELETE-}" ] && [ ! -f "$SHIM_TEST_PENDING_DELETE" ]; then
                    for registration in "$CARGO_TILE_ROOT/state/pids"/*; do
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
printf '%s' "$SHIM_TEST_GENERATION" > "$SHIM_TEST_OBSERVATIONS/calendar"
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
    printf 'previous log\n' > "$CARGO_TILE_ROOT/run-${name#*.}-${name%%.*}.log"
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
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");
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
    print(time.strftime(formatting, time.localtime(birth)))
else:
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
            "#!/bin/sh\nprintf '{ sec = 1788900000, usec = 123456 } Wed Sep 9\\n'\n",
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

    /// Drive the production binary through a PTY; Python owns every child and terminal fd.
    /// The reader receives the shim's actual records, with no copied Rust implementation.
    fn reader_regression(scenario: &str) {
        let directory = tempfile::tempdir().expect("isolate writer and reader processes");
        let output = Command::new("python3")
            .args(["-c", READER_SCENARIO_SCRIPT])
            .arg(directory.path())
            .arg(env!("CARGO_BIN_EXE_cargo-tile"))
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/cargo-capture-shim.sh"
            ))
            .arg(scenario)
            .output()
            .expect("run isolated production reader regression");
        assert!(
            output.status.success(),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    /// Two HOME values cannot split one physical directory into separate headings.
    #[test]
    fn reader_groups_one_directory_across_different_writer_homes() {
        reader_regression("grouping");
    }

    /// Select the fixture's command pane even when an unrelated pane precedes it.
    #[test]
    fn reader_grouping_finds_fixture_markers_after_an_unrelated_pane() {
        reader_regression("grouping-earlier-pane");
    }

    /// One enclosing capture supplies progress without replacing nested commands or pids.
    #[test]
    fn reader_keeps_nested_invocations_distinct_with_one_enclosing_capture() {
        reader_regression("nested");
    }

    /// An exec replaces cargo with an application without making its cargo children direct owners.
    #[test]
    fn reader_keeps_application_spawned_cargo_invocations_distinct() {
        reader_regression("exec-nested");
    }

    /// Excluding the captured run cannot hide cargo children launched by its application.
    #[test]
    fn reader_keeps_application_spawned_cargo_when_run_is_excluded() {
        reader_regression("exec-excluded");
    }

    /// Same-birth publications cannot select an old generation by its filename order.
    #[test]
    fn reader_rejects_competing_generations_until_one_publication_remains() {
        reader_regression("ambiguous-generation");
    }

    /// Missing birth evidence cannot prove a competing publication belongs to another lifetime.
    #[test]
    fn reader_rejects_an_unverifiable_competing_generation() {
        reader_regression("unverifiable-generation");
    }

    /// Excluded writers remain live for cleanup and nested capture membership.
    #[test]
    fn reader_excludes_a_live_command_without_sweeping_its_capture() {
        reader_regression("excluded");
    }

    /// Another live pid cannot adopt the identity copied from a published record.
    #[test]
    fn reader_rejects_another_live_processes_registration_identity() {
        reader_regression("forged");
    }

    /// A fresh kernel absence permits staging cleanup while unknown and live records stay.
    #[test]
    fn reader_removes_ended_staging_and_preserves_unknown_and_live_staging() {
        reader_regression("staging");
    }

    /// Production parsing and kernel verification accept the actual writer's bytes.
    #[test]
    fn reader_accepts_live_writer_under_different_locale_and_timezone() {
        reader_regression("locale");
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
        let generation = std::str::from_utf8(fields[1]).expect("ASCII generation");
        assert!(generation.starts_with("20260909-204000-"));
        let log = format!("run-{generation}-{pid}.log");
        let count = arguments.len().to_string();
        let directory = fixture.path("home/work tree\twith\nlines");
        let home = fixture.path("home");
        assert_eq!(
            &fields[..8],
            [
                b"cargo-tile-v2".as_slice(),
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
        let joined = InstalledShim::new();
        let separated = InstalledShim::new();
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
        let first = InstalledShim::new();
        let second = InstalledShim::new();
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
        let fixture = InstalledShim::new();
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
        assert!(fields[3].iter().all(u8::is_ascii_digit), "birth is decimal");
        assert!(!fields[2].is_empty(), "this host exposes its boot identity");
    }

    /// Calendar time and a reused shell pid cannot reproduce a publication name.
    #[test]
    fn same_pid_and_calendar_second_receive_different_generations() {
        let fixture = InstalledShim::new();
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
        let fixture = InstalledShim::new();
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
            let fixture = InstalledShim::new();
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
                let observed =
                    fs::read(fixture.path(&format!("observations/{utility}-conversion")))
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

    /// The two occurrences of 01:30 at DST fallback must publish different epoch seconds.
    #[test]
    fn darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback() {
        let births = ["1793511000", "1793514600"];
        let mut wall_times = Vec::new();
        let mut published = Vec::new();
        for birth in births {
            let fixture = InstalledShim::new();
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
        let fixture = InstalledShim::new();
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
        let fixture = InstalledShim::new();
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
        let fixture = InstalledShim::new();
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

    /// Publishing the extended layout must preserve both older v2 layouts and malformed neighbors.
    #[test]
    fn phase_two_records_and_malformed_neighbors_survive_publication_and_cleanup() {
        let fixture = InstalledShim::new();
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
            fs::write(fixture.path("capture").join(log), b"older output\n")
                .expect("seed prior log");
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
                    fs::read(fixture.path(directory).join(&name))
                        .expect("neighbor remains unchanged"),
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
}
