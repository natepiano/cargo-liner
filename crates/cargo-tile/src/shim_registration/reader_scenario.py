from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from datetime import datetime, timezone
import codecs
from collections.abc import Callable
import errno
import faulthandler
import fcntl
import os
from pathlib import Path
import pty
import pwd
import re
import select
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import termios
import threading
import time
from typing import TYPE_CHECKING, ClassVar, Iterator, TypeVar, cast

if TYPE_CHECKING:
    # A started writer is (child, observations, registration, fields, log).
    StartedWriter = tuple[subprocess.Popen[bytes], Path, Path, list[bytes], Path]
    RegistrationCarrier = tuple['ParentOwnedChild', Path, Path, list[bytes], Path]
    # A cell foreground holds its SGR color parameters, or None for the default color.
    Foreground = tuple[int, ...] | None
    Snapshot = tuple[str, list[list[Foreground]]]

root = Path(sys.argv[1]).resolve()
binary, source, scenario, observation_argument, smoothing_argument, *deadline_arguments = sys.argv[2:]
timestamps_requested = os.environ.get('CARGO_TILE_READER_TIMESTAMPS') == '1'
timestamp_origin = time.monotonic()
previous_timestamp = timestamp_origin
timestamp_lock = threading.Lock()

def record_timestamp(name: str) -> None:
    global previous_timestamp
    with timestamp_lock:
        now = time.monotonic()
        if timestamps_requested:
            print(f'reader timestamp {name}: +{now - timestamp_origin:.6f}s '
                  f'({now - previous_timestamp:.6f}s)', file=sys.stderr, flush=True)
        previous_timestamp = now

record_timestamp('python start')
# How long the CPU scenario reads the table, and how long the reader's
# CensusCadence takes a reading most of the way to a changed share, in seconds.
observation_window = float(observation_argument)
smoothing_window = float(smoothing_argument)
READER_SCENARIOS = (
    'child-source-switch', 'locale', 'root-headings', 'settings-scroll-burst',
    'cpu-cache-server', 'quiet-json-long', 'excluded',
)
SELF_CHECK_SCENARIOS = (
    '--concurrent-deadline-self-check', '--reader-end-self-check',
    '--scenario-deadline-self-check', '--terminal-frame-self-check',
)
assert scenario in READER_SCENARIOS or scenario in SELF_CHECK_SCENARIOS, scenario
assert len(deadline_arguments) <= 1, deadline_arguments
# Ratatui completes every cursorless draw with Crossterm's Hide command.
frame_end = b'\x1b[?25l'
# The reader counts only this script's writers; leave room for every asserted row.
terminal_rows = 40
terminal_columns = 240
# How long wait_for polls before it fails.
wait_seconds = 10
SCENARIO_DEADLINE_SECONDS = wait_seconds * 2
HARD_STOP_GRACE_SECONDS = wait_seconds / 2
READER_EXIT_SECONDS = wait_seconds / 2
READER_KILLED_EXIT_SECONDS = wait_seconds / 10
READER_STOP_SECONDS = wait_seconds / 2
WRITER_KILLED_EXIT_SECONDS = wait_seconds / 10
scenario_deadline_seconds = (float(deadline_arguments[0]) if deadline_arguments
                             else SCENARIO_DEADLINE_SECONDS)
home = root / 'home'
work = home / ('repair-group-' + root.name)
capture_parent = root / 'capture'
capture = capture_parent / str(os.getuid())
other_uid = 4294967294
other_capture = capture_parent / str(other_uid)
ended_pid = 2147483647
pids = capture / 'state/pids'
bin_directory = root / 'bin'
foreign_work = home / ('foreign-group-' + root.name)

T = TypeVar('T')

def required(value: T | None, description: str) -> T:
    # Fail with the description instead of a TypeError where None is first used.
    if value is None:
        raise AssertionError(description)
    return value

def copy_named_shell(path: Path) -> None:
    # Darwin's /bin/sh re-execs another shell, losing the fixture process name.
    shell = '/bin/bash' if sys.platform == 'darwin' else required(shutil.which('sh'), 'sh is not on PATH')
    _ = shutil.copyfile(shell, path)
    path.chmod(0o755)
    if sys.platform == 'darwin':
        # A relocated platform shell is killed before exec completes. Sign only
        # the owned fixture copy so it can run under its cargo/compiler name.
        _ = subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(path)],
                           check=True, capture_output=True, text=True, timeout=wait_seconds)
    record_timestamp('copied shell ' + path.name)

def link_named_shell(path: Path) -> None:
    os.link(bin_directory / 'cargo-tile-real', path)
    record_timestamp('linked shell ' + path.name)

writer_locale = 'POSIX'
environment: dict[str, str] = {}

def prepare_fixture() -> None:
    global environment, writer_locale
    bin_directory.mkdir(parents=True)
    copy_named_shell(bin_directory / 'cargo-tile-real')
    for directory in (work, pids, root / 'config/cargo-tile',
                      home / 'Library/Application Support/cargo-tile', root / 'rustup/toolchains'):
        directory.mkdir(parents=True)
    configuration = '[capture]\nauto_install = false\n'
    if scenario == 'root-headings':
        (other_capture / 'state/pids').mkdir(parents=True)
        # Account discovery is automatic; no roots configuration is written.
    configuration += '[tiles]\ninitial_rows = 100\n'
    if scenario == 'excluded':
        configuration += '[commands]\nexcluded = ["clippy"]\n'
    for directory in (root / 'config/cargo-tile', home / 'Library/Application Support/cargo-tile'):
        _ = (directory / 'config.toml').write_text(configuration)
    shim_source = Path(source).read_text()
    assignment = 'capture_parent=/tmp/cargo-tile'
    assert shim_source.splitlines().count(assignment) == 1
    _ = (bin_directory / 'cargo').write_text(
        shim_source.replace(assignment, 'capture_parent=' + shlex.quote(str(capture_parent))))
    _ = (work / 'build').write_text('''accumulate_own_cpu() {
    if [ -r "/proc/$$/stat" ]; then
        while :; do
            read -r pid comm state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime remainder < "/proc/$$/stat"
            [ "$((utime + stime))" -gt 0 ] && return
        done
    fi
    remaining=20000
    while [ "$remaining" -gt 0 ]; do remaining=$((remaining - 1)); done
}
printf '%s\\0' "$LC_ALL" "$TZ" "$LANG" "$HOME" > "$OBSERVED/environment"
printf '%s' "$$" > "$OBSERVED/cargo-pid"
printf '%s' "${CARGOTILE_NESTED-}" > "$OBSERVED/enclosing-pid"
printf '%s\\0' "$@" > "$OBSERVED/arguments"
printf 'process' > "$OBSERVED/source"
printf 'Blocking waiting for file lock on build directory\\n' >&2
if [ -n "${CPU_WORKLOAD-}" ]; then
    # Establish the invocation's own nonzero counter before its first scan.
    accumulate_own_cpu
    printf 'ready' > "$OBSERVED/cpu-ready"
    sh "$CPU_WORKLOAD"
    exit 37
fi
if [ -n "${NESTED_WORK-}" ]; then
    for command in check test; do
        (
            cd "$NESTED_WORK" || exit 93
            OBSERVED="$OBSERVED/$command" NESTED_WORK= exec sh "$CARGO" "$command" "$NESTED_MARKER-$command"
        ) &
    done
fi
remaining=1000
while [ ! -f "$OBSERVED/release" ] && [ "$remaining" -gt 0 ]; do
    if [ -f "$OBSERVED/spawn-child" ] && [ ! -f "$OBSERVED/child-started" ]; then
        sh "$OBSERVED/spawn-child" &
        printf '%s' "$!" > "$OBSERVED/child-started"
    fi
    if [ -n "${REGISTRATION_CARRIER-}" ] && [ -f "$OBSERVED/retire" ]; then
        rm "$OBSERVED/activate" "$OBSERVED/retire"
        exec sh "$REGISTRATION_CARRIER"
    fi
    if [ -f "$OBSERVED/pulse" ]; then
        printf 'writer remains captured after reader scan\\n' >&2
        rm "$OBSERVED/pulse"
    fi
    if [ -p "$OBSERVED/notification" ]; then
        IFS= read -r notification < "$OBSERVED/notification" || :
    else
        sleep 0.02
    fi
    remaining=$((remaining - 1))
done
[ "$remaining" -gt 0 ] || exit 92
wait
exit 37
''')
    _ = shutil.copyfile(work / 'build', work / 'clippy')
    _ = shutil.copyfile(work / 'build', work / 'check')

    if scenario == 'locale':
        locales = subprocess.run(['locale', '-a'], check=True, capture_output=True,
                                 text=True, timeout=wait_seconds).stdout.split()
        writer_locale = next((name for name in locales if name not in ('C', 'POSIX')
                              and not name.lower().startswith('c.')), 'POSIX')
    environment = dict(os.environ)
    for key in ('CARGOTILE_NESTED', 'CARGO_TILE_FRAME_LOG', 'CARGO_TERM_PROGRESS_WHEN',
                'CARGO_TERM_PROGRESS_WIDTH', 'ITERM_SESSION_ID', 'NESTED_WORK', 'NESTED_MARKER'):
        _ = environment.pop(key, None)
    environment.update(HOME=str(home), XDG_CONFIG_HOME=str(root / 'config'),
                       XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'),
                       RUSTUP_HOME=str(root / 'rustup'),
                       PYTHONUTF8='1',
                       LC_ALL=writer_locale, LANG=writer_locale,
                       TZ='EST5EDT,M3.2.0,M11.1.0', TERM='xterm-256color')
writers: list[tuple[subprocess.Popen[bytes], Path]] = []
parent_owned_children: list[Path] = []
detached_servers: list[subprocess.Popen[bytes]] = []
# CPU setup assigns these through globals; annotations narrow them at module scope.
cache_server = None
cache_server_process: subprocess.Popen[bytes] | None = None
reader: int | None = None
terminal: int | None = None
transcript = bytearray()
pending_waits: dict[int, tuple[str, ...]] = {}

@contextmanager
def named_wait(description: str) -> Iterator[None]:
    thread = threading.get_ident()
    previous = pending_waits.get(thread, ())
    pending_waits[thread] = (*previous, description)
    try:
        yield
    finally:
        if previous:
            pending_waits[thread] = previous
        else:
            _ = pending_waits.pop(thread, None)

def pending_wait_description() -> str:
    # Python runs signal handlers with the GIL held, so this snapshot cannot race
    # a worker's context entry or exit.
    waits = tuple(pending_waits.values())
    return next((descriptions[-1] for descriptions in waits if descriptions),
                'scenario execution')

def scenario_deadline_reached(_signal_number, _frame) -> None:
    raise TimeoutError(
        f'{scenario} exceeded {scenario_deadline_seconds:g} seconds while waiting for '
        f'{pending_wait_description()}')

def arm_scenario_deadline() -> None:
    _ = signal.signal(signal.SIGALRM, scenario_deadline_reached)
    _ = signal.setitimer(signal.ITIMER_REAL, scenario_deadline_seconds)
    faulthandler.dump_traceback_later(
        scenario_deadline_seconds + HARD_STOP_GRACE_SECONDS, exit=True)

def cancel_scenario_deadline() -> None:
    _ = signal.setitimer(signal.ITIMER_REAL, 0)
    faulthandler.cancel_dump_traceback_later()

def reader_pid() -> int:
    return required(reader, 'reader process is used before pty.fork')

def reader_terminal() -> int:
    return required(terminal, 'reader terminal is used before pty.fork')

def start_reader_process(reader_environment: dict[str, str]) -> None:
    global reader, terminal
    if scenario == 'cpu-cache-server':
        prerequisites = (
            root / ('probe-first-' + root.name) / 'cpu-ready',
            root / ('probe-unrelated-' + root.name) / 'cpu-ready',
            root / 'cpu-server/compiler-pid',
        )
        missing = [str(path) for path in prerequisites if not path.exists()]
        assert not missing, 'reader starts before CPU baselines: ' + ', '.join(missing)
    record_timestamp('reader exec')
    reader, terminal = pty.fork()
    if reader == 0:
        _ = fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
        os.chdir(root)
        if scenario == 'settings-scroll-burst':
            _ = (root / 'reader-terminal').write_text(os.ttyname(0))
        reader_environment['CARGO_TILE_TEST_READER'] = scenario
        os.execve(binary, [binary, '--exact', 'shim_registration::reader_scenarios::reader_child', '--nocapture'], reader_environment)

def wait_for(predicate: Callable[[], object], description: str,
             diagnostics: Callable[[], str] | None = None, *, pause: float = 0.02,
             deadline: float | None = None) -> None:
    # Filesystem and ps predicates pause between checks; a predicate that blocks on a
    # terminal read passes pause=0. A caller that bounds nested waits passes their deadline.
    deadline = time.monotonic() + wait_seconds if deadline is None else deadline
    with named_wait(description):
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(pause)
        details = diagnostics() if diagnostics is not None else ('\n' + screen() if transcript else '')
        raise AssertionError(description + details)

class WriterNotification:
    """Wake one source-switch fixture shell without a polling process."""

    def __init__(self, observations: Path) -> None:
        path = observations / 'notification'
        os.mkfifo(path)
        self.read_descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
        self.write_descriptor = os.open(path, os.O_WRONLY | os.O_NONBLOCK)

    def send(self) -> None:
        written = os.write(self.write_descriptor, b'\n')
        assert written == 1, written

    def close(self) -> None:
        os.close(self.write_descriptor)
        os.close(self.read_descriptor)

writer_notifications: dict[Path, WriterNotification] = {}

def start_writer(name: str, writer_home: Path, command: str = 'build',
                 nested_directory: Path | None = None, directory: Path = work,
                 arguments: tuple[str, ...] = (),
                 additional_environment: tuple[tuple[str, str], ...] = ()) -> StartedWriter:
    name += '-' + root.name
    observations = root / name
    observations.mkdir()
    if scenario == 'child-source-switch':
        writer_notifications[observations] = WriterNotification(observations)
    child_environment = dict(environment, HOME=str(writer_home), OBSERVED=str(observations))
    child_environment.update(additional_environment)
    if nested_directory is not None:
        nested_directory.mkdir()
        for nested_command in ('check', 'test'):
            (observations / nested_command).mkdir()
            _ = shutil.copyfile(work / 'build', nested_directory / nested_command)
        child_environment.update(NESTED_WORK=str(nested_directory),
                                 NESTED_MARKER='probe-nested-' + root.name)
    with (observations / 'output').open('wb') as output:
        child = subprocess.Popen(['sh', str(bin_directory / 'cargo'), command, name, *arguments],
                                 cwd=directory, env=child_environment, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=output, start_new_session=True)
    writers.append((child, observations))
    wait_for(lambda: (observations / 'output').stat().st_size > 0,
             'writer produces no output', pause=0.005)
    record_timestamp('writer first output ' + name)
    wait_for(lambda: (observations / 'cargo-pid').exists(), 'cargo does not start', pause=0.005)
    publications = capture / 'state/pids'
    wait_for(lambda: any(publications.glob(str(child.pid) + '.*')),
             'shim does not publish', pause=0.005)
    registration = next(publications.glob(str(child.pid) + '.*'))
    fields = registration.read_bytes().split(b'\0')
    log = capture / os.fsdecode(fields[4])
    wait_for(lambda: log.exists() and b'Blocking waiting' in log.read_bytes(),
             'writer does not capture progress', pause=0.005)
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
    record_timestamp('writer ' + name)
    return child, observations, registration, fields, log

def copy_publication_to_foreign_root(writer: StartedWriter) -> None:
    registration = other_capture / 'state/pids' / writer[2].name
    log = other_capture / writer[4].name
    fields = list(writer[3])
    fields[5] = os.fsencode(foreign_work)
    _ = registration.write_bytes(b'\0'.join(fields))
    _ = shutil.copyfile(writer[4], log)
    assert registration.read_bytes() != writer[2].read_bytes(), \
        'foreign publication must differ from the accepted publication'

def seed_ended_publication(template: StartedWriter) -> tuple[Path, Path]:
    pid = ended_pid
    generation = 'ended-' + root.name
    fields = list(template[3])
    fields[1] = generation.encode()
    fields[3] = b'1'
    fields[4] = f'run-{generation}-{pid}.log'.encode()
    registration = pids / f'{pid}.{generation}'
    log = capture / os.fsdecode(fields[4])
    _ = registration.write_bytes(b'\0'.join(fields))
    _ = log.write_bytes(b'Blocking waiting for file lock on build directory\n')
    return registration, log

class ParentOwnedChild:
    # The enclosing writer owns wait and process-group cleanup for this child.
    def __init__(self, pid: int) -> None:
        self.pid: int = pid

def trigger_writer(observations: Path, trigger: str) -> None:
    (observations / trigger).touch()
    if observations in writer_notifications:
        writer_notifications[observations].send()

def start_registration_carrier(template: StartedWriter) -> RegistrationCarrier:
    # The process keeps one kernel identity while alternating between cargo argv
    # and a non-cargo registration carrier.
    observations = root / ('probe-carrier-' + root.name)
    observations.mkdir()
    writer_notifications[observations] = WriterNotification(observations)
    directory = home / ('registered-directory-' + root.name)
    directory.mkdir(parents=True)
    _ = shutil.copyfile(work / 'build', directory / 'build')
    carrier_script = observations / 'carrier.sh'
    _ = carrier_script.write_text('''printf registration > "$OBSERVED/source"
while [ ! -f "$OBSERVED/release" ]; do
    if [ -f "$OBSERVED/activate" ]; then
        exec "$CARRIER_CARGO" "$CARRIER_COMMAND" "$CARRIER_NAME"
    fi
    if [ -p "$OBSERVED/notification" ]; then
        IFS= read -r notification < "$OBSERVED/notification" || :
    else
        sleep 0.02
    fi
done
''')
    child_environment = dict(environment, HOME=str(home), OBSERVED=str(observations),
                             CARRIER_CARGO=str(bin_directory / 'cargo-tile-real'),
                             CARRIER_COMMAND='build', CARRIER_NAME=observations.name,
                             REGISTRATION_CARRIER=str(carrier_script))
    parent_owned_children.append(observations)
    launch = ['env', *(key + '=' + value for key, value in child_environment.items()
                      if environment.get(key) != value),
              str(bin_directory / 'cargo-tile-real'), 'build', observations.name]
    launch_file = template[1] / 'spawn-child.tmp'
    _ = launch_file.write_text(
        'cd ' + shlex.quote(str(directory)) + '\nexec ' + shlex.join(launch) + '\n')
    _ = launch_file.rename(template[1] / 'spawn-child')
    trigger_writer(template[1], 'spawn-child')
    observed_pid = template[1] / 'child-started'
    wait_for(lambda: observed_pid.exists() and observed_pid.read_text().isdigit(),
             'readable parent does not spawn its only child', pause=0.005)
    child = ParentOwnedChild(int(observed_pid.read_text()))
    wait_for(lambda: (observations / 'source').exists(),
             'registration carrier does not start', pause=0.005)
    fields = list(template[3])
    if sys.platform == 'linux':
        stat = Path('/proc/' + str(child.pid) + '/stat').read_bytes()
        fields[3] = stat.rsplit(b') ', 1)[1].split()[19]
    else:
        started = subprocess.run(['ps', '-p', str(child.pid), '-o', 'lstart='], check=True,
                                 capture_output=True, text=True,
                                 env=dict(environment, LC_ALL='C', TZ='UTC0'),
                                 timeout=wait_seconds).stdout.strip()
        fields[3] = str(int(datetime.strptime(started, '%a %b %d %H:%M:%S %Y')
                           .replace(tzinfo=timezone.utc).timestamp())).encode()
    fields[1] += b'-carrier'
    fields[4] = b'run-' + fields[1] + b'-' + str(child.pid).encode() + b'.log'
    fields[5:8] = [os.fsencode(directory), os.fsencode(home), b'2']
    fields[8:] = [b'build', observations.name.encode(), b'']
    registration = pids / (str(child.pid) + '.' + fields[1].decode())
    log = capture / os.fsdecode(fields[4])
    _ = log.write_bytes(b'Blocking waiting for file lock on build directory\n')
    _ = registration.write_bytes(b'\0'.join(fields))
    return child, observations, registration, fields, log

def prepare_cpu_workload() -> None:
    global cache_server_process
    workload = root / 'cpu-workload.sh'
    environment['CPU_WORKLOAD'] = str(workload)

    server = root / 'cpu-server'
    server.mkdir()
    environment['CPU_SERVER'] = str(server)
    environment['CPU_TARGET'] = str(work / 'target')
    (work / 'target/debug/deps').mkdir(parents=True)
    for name in ('sccache', 'rustc'):
        link_named_shell(bin_directory / name)
    environment['RUSTC_WRAPPER'] = str(bin_directory / 'sccache')
    compiler = server / 'compile.sh'
    _ = compiler.write_text('''printf '%s' "$$" > "$CPU_SERVER/compiler-pid"
while [ ! -f "$CPU_SERVER/release" ]; do :; done
''')
    service = server / 'serve.sh'
    _ = service.write_text('''printf '%s' "$$" > "$CPU_SERVER/server-pid"
while [ ! -f "$CPU_SERVER/request" ]; do sleep 0.005; done
"$CPU_RUSTC" "$CPU_COMPILE" --crate-name cache_fixture --out-dir "$CPU_TARGET/debug/deps" &
wait
''')
    client = server / 'client.sh'
    _ = client.write_text('''printf '%s' "$$" > "$OBSERVED/client-pid"
printf 'compile' > "$CPU_SERVER/request"
while [ ! -f "$OBSERVED/release" ]; do sleep 0.005; done
''')
    _ = workload.write_text('exec "$RUSTC_WRAPPER" ' + shlex.quote(str(client)) + '\n')
    server_environment = dict(environment, CPU_RUSTC=str(bin_directory / 'rustc'),
                              CPU_COMPILE=str(compiler))
    # A new session keeps the compiler server outside every cargo process tree.
    with (server / 'output').open('wb') as output:
        server_process = subprocess.Popen(
            [str(bin_directory / 'sccache'), str(service)], env=server_environment,
            stdin=subprocess.DEVNULL, stdout=output, stderr=output, start_new_session=True)
    cache_server_process = server_process
    detached_servers.append(server_process)
    record_timestamp('cache server launched')

def finish_cpu_workload_start() -> None:
    global cache_server
    server = root / 'cpu-server'
    server_process = required(cache_server_process, 'cache server process is not started')
    wait_for(lambda: (server / 'server-pid').exists()
             and (server / 'server-pid').read_text(),
             'cache server does not start', pause=0.005)
    cache_server = int((server / 'server-pid').read_text())
    assert cache_server == server_process.pid
    assert os.getsid(cache_server) == cache_server
    record_timestamp('cache server ready')

def process_parent(pid: int) -> int:
    return int(subprocess.run(['ps', '-p', str(pid), '-o', 'ppid='], check=True,
                              capture_output=True, text=True,
                              timeout=wait_seconds).stdout.strip())

def cpu_readings_in_frame(rendered: str, writers: tuple[StartedWriter, ...]
                          ) -> list[tuple[StartedWriter, int]]:
    readings = []
    for writer in writers:
        pid = (writer[1] / 'cargo-pid').read_text()
        rows = [line for line in rendered.splitlines() if writer[1].name in line
                and re.match(r'^\s*│\s*' + pid + r'\s', line)]
        if not rows:
            continue
        values = []
        for row in rows:
            # Header and row redraws can be observed separately; read CPU from this row.
            prefix, command, _ = row.partition('cargo ')
            assert command, 'CPU fixture row has no cargo command\n' + rendered
            cells = prefix.replace('│', ' ').split()
            assert cells[0] == pid, rendered
            percentages = [cell for cell in cells if re.fullmatch(r'\d+%', cell)]
            assert len(percentages) == 1, \
                'CPU row loses its numeric reading in a completed frame\n' + rendered
            values.append(int(percentages[0][:-1]))
        assert len(set(values)) == 1, 'CPU rows disagree in a completed frame\n' + rendered
        readings.append((writer, values[0]))
    return readings

def cpu_rows_are_ready(rendered: str, writers: tuple[StartedWriter, ...]) -> bool:
    for writer in writers:
        pid = (writer[1] / 'cargo-pid').read_text()
        rows = [line for line in rendered.splitlines() if writer[1].name in line
                and re.match(r'^\s*│\s*' + pid + r'\s', line)]
        if not rows or any(len(re.findall(r'\d+%', row)) != 1 for row in rows):
            return False
    return True

def wait_for_cpu_rows(writers: tuple[StartedWriter, ...]) -> str:
    rendered = ''
    def rows_are_ready() -> bool:
        nonlocal rendered
        read_terminal(0.1)
        rendered = screen()
        return cpu_rows_are_ready(rendered, writers)
    wait_for(rows_are_ready, 'CPU rows do not finish rendering', pause=0)
    return rendered

def assert_cpu_workload(writer: StartedWriter, unrelated: StartedWriter, rendered: str) -> str:
    writers = (writer, unrelated)
    if not cpu_rows_are_ready(rendered, writers):
        rendered = wait_for_cpu_rows(writers)
    record_timestamp('CPU rows')
    started = time.monotonic()
    initial = cpu_readings_in_frame(rendered, writers)
    assert len(initial) == len(writers), rendered
    readings = [(0.0, next(cpu for observed, cpu in initial if observed[1] == writer[1]))]
    other_readings = [next(cpu for observed, cpu in initial if observed[1] == unrelated[1])]
    terminal_frames.take_completed()
    # Readings climb for one smoothing window; the sustained set fills the rest of the window.
    deadline = started + observation_window
    def observe_cpu() -> None:
        nonlocal rendered
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        read_terminal(min(0.1, remaining))
        terminal_snapshot()
        for frame in terminal_frames.take_completed():
            rendered = frame
            elapsed = time.monotonic() - started
            for observed, cpu in cpu_readings_in_frame(frame, writers):
                if observed[1] == writer[1]:
                    readings.append((elapsed, cpu))
                else:
                    other_readings.append(cpu)
    while time.monotonic() < deadline:
        observe_cpu()
    record_timestamp('CPU observation')
    sustained = [cpu for elapsed, cpu in readings if elapsed >= smoothing_window]
    assert len(sustained) >= 3 and min(sustained) >= 10, (
        'compile CPU must remain charged across reporting windows', readings)
    assert max(other_readings) < min(sustained) / 2, (readings, other_readings)
    return rendered


def read_pending() -> bool:
    # Append one read of reader output; False once the reader has closed its terminal.
    try:
        data = os.read(reader_terminal(), 65536)
    except OSError as error:
        if error.errno == errno.EIO:
            return False
        raise
    transcript.extend(data)
    return bool(data)

def read_terminal(duration: float) -> None:
    # Return once this call has read a completed frame and nothing more is pending, so a
    # predicate is checked against every frame; duration bounds the wait when none arrives.
    deadline = time.monotonic() + duration
    # Start early enough to find a frame end split across the previous read and this one.
    frame_search = max(0, len(transcript) - len(frame_end) + 1)
    while (remaining := deadline - time.monotonic()) > 0:
        framed = transcript.find(frame_end, frame_search) >= 0
        if not select.select([reader_terminal()], [], [], 0 if framed else remaining)[0]:
            return
        if not read_pending():
            return

def reader_exited_before(deadline: float, terminal_fd: int | None) -> bool:
    while True:
        finished, _status = os.waitpid(reader_pid(), os.WNOHANG)
        if finished:
            return True
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False
        pause = min(0.01, remaining)
        if terminal_fd is None:
            time.sleep(pause)
        elif select.select([terminal_fd], [], [], pause)[0] and not read_pending():
            time.sleep(pause)

def close_reader_terminal() -> None:
    global terminal
    if terminal is not None:
        os.close(terminal)
        terminal = None

def end_reader(request: bytes | signal.Signals, exit_seconds: float = READER_EXIT_SECONDS,
               killed_exit_seconds: float = READER_KILLED_EXIT_SECONDS) -> None:
    terminal_fd = reader_terminal()
    if reader_exited_before(time.monotonic(), terminal_fd):
        close_reader_terminal()
        return
    if isinstance(request, bytes):
        try:
            _ = os.write(terminal_fd, request)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
        request_name = repr(request)
    else:
        try:
            os.kill(reader_pid(), request)
        except ProcessLookupError:
            pass
        request_name = request.name
    description = 'reader to exit after ' + request_name
    with named_wait(description):
        if reader_exited_before(time.monotonic() + exit_seconds, terminal_fd):
            close_reader_terminal()
            return
    for end_signal in (signal.SIGCONT, signal.SIGKILL):
        try:
            os.kill(reader_pid(), end_signal)
        except ProcessLookupError:
            pass
    close_reader_terminal()
    description = 'reader to exit after SIGKILL'
    with named_wait(description):
        if not reader_exited_before(time.monotonic() + killed_exit_seconds, None):
            raise AssertionError(description)

class CompletedTerminalFrames:
    """Keep the mutable draw separate from the last completed screen."""

    # Match the valid prefix even when its terminator has not arrived.
    csi: ClassVar[re.Pattern[str]] = re.compile(r'\x1b\[[0-?]*[ -/]*([@-~]?)')
    osc: ClassVar[re.Pattern[str]] = re.compile(r'\x1b\][^\x07\x1b]*(\x07|\x1b\\)?')

    def __init__(self) -> None:
        self.decoder: codecs.IncrementalDecoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.pending: str = ''
        self.offset: int = 0
        self.row: int = 0
        self.column: int = 0
        self.foreground: Foreground = None
        self.cells: list[list[str]] = []
        self.colors: list[list[Foreground]] = []
        self.rows: int = 0
        self.columns: int = 0
        self.published: Snapshot = ('', [])
        self.completed: list[str] = []

    def take_completed(self) -> list[str]:
        completed = self.completed
        self.completed = []
        return completed

    def resize(self, rows: int, columns: int) -> None:
        if (rows, columns) == (self.rows, self.columns):
            return
        self.cells = [(line[:columns] + [' '] * max(0, columns - len(line)))
                      for line in self.cells[:rows]]
        self.colors = [(line[:columns] + [None] * max(0, columns - len(line)))
                       for line in self.colors[:rows]]
        self.cells.extend([[' '] * columns for _ in range(rows - len(self.cells))])
        self.colors.extend([[None] * columns for _ in range(rows - len(self.colors))])
        self.rows, self.columns = rows, columns

    def feed(self, data: bytearray) -> None:
        self.pending += self.decoder.decode(data)
        index = 0
        while index < len(self.pending):
            character = self.pending[index]
            if character == '\x1b':
                if index + 1 == len(self.pending):
                    break
                introducer = self.pending[index + 1]
                if introducer == '[':
                    match = required(self.csi.match(self.pending, index), 'CSI pattern rejects its introducer')
                    if not match[1] and match.end() == len(self.pending):
                        break
                    if match[1]:
                        token = match[0]
                        self.command(token[-1], token[2:-1])
                    # An invalid prefix ends before the byte that cancels it.
                    index = match.end()
                    continue
                if introducer == ']':
                    match = required(self.osc.match(self.pending, index), 'OSC pattern rejects its introducer')
                    # A trailing ESC may be the first byte of the OSC terminator.
                    if not match[1] and self.pending[match.end():] in ('', '\x1b'):
                        break
                    index = match.end()
                    continue
                index += 1 if introducer == '\x1b' else 2
                continue
            if character == '\r':
                self.column = 0
            elif character == '\n':
                self.row += 1
            elif character >= ' ':
                if 0 <= self.row < self.rows and 0 <= self.column < self.columns:
                    self.cells[self.row][self.column] = character
                    self.colors[self.row][self.column] = self.foreground
                self.column += 1
            index += 1
        self.pending = self.pending[index:]

    def command(self, command: str, parameters: str) -> None:
        if command == 'l' and parameters == '?25':
            self.published = ('\n'.join(''.join(line).rstrip() for line in self.cells),
                              [line.copy() for line in self.colors])
            self.completed.append(self.published[0])
        elif command in 'Hf':
            position = parameters.split(';')
            self.row = int(position[0] or 1) - 1
            self.column = int(position[1] or 1) - 1 if len(position) > 1 else 0
        elif command == 'J' and parameters in ('2', '3'):
            self.cells = [[' '] * self.columns for _ in range(self.rows)]
            self.colors = [[None] * self.columns for _ in range(self.rows)]
        elif command == 'K' and 0 <= self.row < self.rows:
            beginning = max(0, min(self.column, self.columns))
            self.cells[self.row][beginning:] = [' '] * (self.columns - beginning)
            self.colors[self.row][beginning:] = [None] * (self.columns - beginning)
        elif command == 'C':
            self.column += int(parameters or 1)
        elif command == 'G':
            self.column = int(parameters or 1) - 1
        elif command == 'm':
            attributes = [int(value or 0) for value in parameters.split(';')]
            index = 0
            while index < len(attributes):
                attribute = attributes[index]
                if attribute in (0, 39):
                    self.foreground = None
                elif 30 <= attribute <= 37 or 90 <= attribute <= 97:
                    self.foreground = (attribute,)
                elif attribute in (38, 48) and index + 1 < len(attributes):
                    count = 3 if attributes[index + 1] == 2 else 1
                    if attribute == 38:
                        self.foreground = tuple(attributes[index + 1:index + 2 + count])
                    index += 1 + count
                index += 1

terminal_frames = CompletedTerminalFrames()

def terminal_snapshot() -> Snapshot:
    terminal_frames.resize(terminal_rows, terminal_columns)
    terminal_frames.feed(transcript[terminal_frames.offset:])
    terminal_frames.offset = len(transcript)
    return terminal_frames.published

def screen() -> str:
    return terminal_snapshot()[0]

def unavailable_measurements(row: str) -> int:
    # The final runs cell can touch the pane border without trailing padding.
    # A border delimits that cell just as whitespace delimits the inner cells.
    return row.replace('│', ' ').split().count('--')

def carrier_source_in_rendered(writer: RegistrationCarrier, source: str, rendered: str) -> bool:
    lines = rendered.splitlines()
    matching = [index for index, line in enumerate(lines) if writer[1].name in line]
    if len(matching) != 1:
        return False
    child_line = matching[0]
    if not any('parent' in line and 'command' in line for line in lines[:child_line]):
        return False
    unavailable = unavailable_measurements(lines[child_line])
    expected = 4
    if source == 'registration':
        return unavailable == expected
    # A sleeping process may never earn a CPU baseline. Its observed compiler
    # absence and managed count still distinguish it from registration-only data.
    return unavailable < expected

def carrier_source_is_rendered(writer: RegistrationCarrier, source: str) -> bool:
    read_terminal(0.1)
    return carrier_source_in_rendered(writer, source, screen())

def assert_child_family(parent: StartedWriter, child: RegistrationCarrier,
                        rendered: str) -> tuple[tuple[int, ...], tuple[str, ...], tuple[str, ...]]:
    # No PTY read separates the ready screen from its matching color snapshot.
    colors = terminal_snapshot()[1]
    lines = rendered.splitlines()
    child_matches = [line for line in lines if child[1].name in line]
    assert len(child_matches) == 1, 'source change duplicates an invocation\n' + rendered
    child_row = child_matches[0]
    child_line = lines.index(child_row)
    child_header_line = next(index for index in reversed(range(child_line))
                             if 'parent' in lines[index] and 'command' in lines[index])
    command_lines = lines[child_header_line + 1:]
    rows: dict[int, str] = {}
    for writer in (parent, child):
        matching = [line for line in command_lines if writer[1].name in line]
        assert len(matching) == 1, 'source change duplicates an invocation\n' + rendered
        rows[writer[0].pid] = matching[0]
    parent_row, child_row = rows[parent[0].pid], rows[child[0].pid]
    parent_pid = (parent[1] / 'cargo-pid').read_text()
    assert re.match(r'^\s*│\s*' + parent_pid + r'\s', parent_row), rendered
    assert re.match(r'^\s*│\s*' + str(child[0].pid) + r'\s+' + parent_pid + r'\s',
                    child_row), rendered
    # The readable lead counts its only assembled child, whichever source supplies it.
    assert re.search(r'\s1\s*│\s*$', parent_row), 'parent loses its managed child\n' + rendered
    parent_line = lines.index(parent_row)
    parent_column = parent_row.index(parent_pid)
    child_column = child_row.index(str(child[0].pid))
    reference_column = child_row.index(parent_pid, child_column + len(str(child[0].pid)))
    family = colors[parent_line][parent_column]
    assert family is not None, 'terminal parser does not observe the family color'
    assert family == colors[child_line][reference_column], 'child loses its parent family color'
    assert family != colors[child_line][child_column], 'parent has plain pid color instead of a family'
    child_header = lines[child_header_line]
    starts = tuple(row[child_header.index('start'):child_header.index('dur')].strip()
                   for row in (parent_row, child_row))
    assert all(starts), 'start cells are absent\n' + rendered
    headings = tuple(line.strip() for line in command_lines
                     if work.name in line or Path(os.fsdecode(child[3][5])).name in line)
    assert len(headings) == 2, 'source change loses a directory heading\n' + rendered
    return family, starts, headings

def assert_child_source_switch(carrier: RegistrationCarrier) -> None:
    initial = assert_child_family(first, carrier, screen())
    record_timestamp('carrier process')
    for source, trigger in (('registration', 'retire'), ('process', 'activate')):
        trigger_writer(carrier[1], trigger)
        wait_for(lambda: carrier_source_is_rendered(carrier, source),
                 'reader does not observe child source ' + source, pause=0)
        observed = assert_child_family(first, carrier, screen())
        record_timestamp('carrier ' + source)
        assert observed == initial, \
            ('child source change alters family color, start, or headings: '
             + repr(initial) + ' became ' + repr(observed) + '\n' + screen())


def settings_screen() -> str:
    _ = os.write(reader_terminal(), b's')
    def settings_are_visible() -> bool:
        read_terminal(0.1)
        rendered = screen()
        return 'Capture:' in rendered and 'auto install' in rendered and 'Commands:' in rendered
    wait_for(settings_are_visible, 'settings do not open', pause=0)
    rendered = screen()
    _ = os.write(reader_terminal(), b'\x1b')
    def settings_are_closed() -> bool:
        read_terminal(0.1)
        return 'Capture:' not in screen()
    wait_for(settings_are_closed, 'settings do not close', pause=0)
    return rendered

def capture_settings_rows(rendered: str, account: str) -> list[str]:
    lines = rendered.splitlines()
    heading = next((index for index, line in enumerate(lines) if 'Capture:' in line), None)
    assert heading is not None, 'Capture heading is absent\n' + rendered
    capture_line = lines[heading]
    heading_column = capture_line.index('Capture:')
    left = capture_line.rfind('│', 0, heading_column)
    right = capture_line.find('│', heading_column)
    assert left >= 0 and right > left, 'Capture popup borders are absent\n' + rendered
    interior = [line[left + 1:right] for line in lines[heading:]]
    end = next((index for index, line in enumerate(interior) if 'Commands:' in line), None)
    assert end is not None, 'Commands heading is absent from popup\n' + rendered
    rows = interior[:end]
    assert any(account in line and 'yours · 1 capture ·' in line for line in rows), \
        'Capture section loses the account row\n' + rendered
    return rows

def assert_settings_scroll() -> None:
    global terminal_rows, terminal_columns
    terminal_rows = 10
    transcript_start = len(transcript)
    input_attributes = termios.tcgetattr(reader_terminal())
    try:
        if scenario == 'settings-scroll-burst':
            # Queue both events while the already-rendering reader is descheduled.
            os.kill(reader_pid(), signal.SIGSTOP)
            deadline = time.monotonic() + READER_STOP_SECONDS
            description = 'reader to stop after SIGSTOP'
            with named_wait(description):
                while time.monotonic() < deadline:
                    stopped, status = os.waitpid(reader_pid(), os.WNOHANG | os.WUNTRACED)
                    if stopped:
                        assert stopped == reader and os.WIFSTOPPED(status), (stopped, status)
                        break
                    time.sleep(0.005)
                else:
                    raise AssertionError('reader did not stop after SIGSTOP')
        _ = fcntl.ioctl(reader_terminal(), termios.TIOCSWINSZ,
                        struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
        written = os.write(reader_terminal(), b's')
    finally:
        if scenario == 'settings-scroll-burst':
            try:
                os.kill(reader_pid(), signal.SIGCONT)
            except ProcessLookupError:
                pass
    def popup_diagnostics() -> str:
        received = bytes(transcript[transcript_start:])
        # Inspect queued input only after timeout; successful runs never read it.
        try:
            slave = os.open((root / 'reader-terminal').read_text(),
                            os.O_RDONLY | os.O_NONBLOCK | os.O_NOCTTY)
            try:
                pending = struct.unpack('I', fcntl.ioctl(slave, termios.FIONREAD,
                                                       struct.pack('I', 0)))[0]
                unread = os.read(slave, pending) if pending else b''
            finally:
                os.close(slave)
        except OSError as error:
            unread = repr(error)
        # termios types tcgetattr as list[Any]; index 3 holds the local mode flags.
        return ('\nsettings input: ' + repr({
            'written': written,
            'canonical': bool(cast(int, input_attributes[3]) & termios.ICANON),
            'echo': bool(cast(int, input_attributes[3]) & termios.ECHO),
            'unread_input': unread,
            'received_bytes': len(received),
            'settings_in_raw_output': b'Settings' in received,
            'completed_frames': received.count(frame_end),
            'raw_tail': received[-2048:],
        }) + '\n' + screen())
    def settings_are_visible() -> bool:
        read_terminal(0.1)
        rendered = screen()
        return 'Settings' in rendered and any('▶' in line and 'mode' in line
                                             for line in rendered.splitlines())
    wait_for(settings_are_visible, 'small settings popup does not open', popup_diagnostics,
             pause=0)

def assert_excluded_command(enclosing: StartedWriter, nested_directory: Path,
                            rendered: str) -> str:
    markers = ['probe-nested-' + root.name + '-' + command for command in ('check', 'test')]
    for marker, command in zip(markers, ('check', 'test')):
        assert nested_directory.name in rendered, rendered
        rows = [line for line in rendered.splitlines() if marker in line]
        assert len(rows) == 1, 'nested invocation does not retain one row\n' + rendered
        assert 'blocked' in rows[0] and 'cargo ' + command + ' ' + marker in rows[0], rendered
        nested_pid = (enclosing[1] / command / 'cargo-pid').read_text()
        assert re.match(r'^\s*│\s*' + nested_pid + r'\s', rows[0]), rendered
    assert not any(enclosing[1].name in line for line in rendered.splitlines()), \
        'excluded command becomes a row\n' + rendered
    assert enclosing[0].poll() is None, 'excluded writer ends before cleanup assertions'
    # Observe another captured write after the reader has pruned the ended
    # sibling; a mere retained empty filename would not prove live capture.
    assert all(not path.exists() for path in removed), 'reader has not swept the sibling'
    (enclosing[1] / 'pulse').touch()
    wait_for(lambda: enclosing[4].exists()
             and b'writer remains captured after reader scan' in enclosing[4].read_bytes(),
             'excluded live command loses its capture after the sweep')
    return rendered

def assert_completed_terminal_frames() -> None:
    global terminal_rows, terminal_columns
    def split_frame(frame: bytes) -> Snapshot:
        previous = terminal_snapshot()
        for byte in frame[:-1]:
            transcript.append(byte)
            assert terminal_snapshot() == previous, 'snapshot exposes an unfinished redraw'
        transcript.append(frame[-1])
        return terminal_snapshot()

    first = ('\x1b[2J\x1b[H│ pid parent command\r\n'
             '│ earlier row\r\n│ cargo check probe-nested\r\n'
             '│ cargo test probe-child\r\n└────').encode() + frame_end
    _ = split_frame(first)
    redraw = ('\x1b[2;1H\x1b[32m│ cargo check probe-nested\x1b[0m'
              '\x1b[3;1H│ cargo test probe-child\x1b[K'
              '\x1b[4;1H└────\x1b[K\x1b[5;1H\x1b[K').encode() + frame_end
    for _ in range(3):
        rendered, colors = split_frame(redraw)
        assert rendered.count('probe-nested') == rendered.count('probe-child') == 1, rendered
        assert colors[1][0] == (32,), 'completed redraw loses foreground colors'
        assert colors[2][0] is None, 'foreground reset does not survive frame completion'

    previous = terminal_snapshot()
    transcript.extend(b'\x1b[2J\x1b[')
    assert terminal_snapshot() == previous, 'unfinished clear discards the completed screen'
    terminal_rows, terminal_columns = 12, 80
    assert terminal_snapshot() == previous, 'resize publishes an incomplete frame'
    redraw = ('H\x1b[38;2;12;34;56m│ resized-é\x1b[39m'
              '\x1b[2;1H\x1b[38;5;123mindexed\x1b[48;2;4;5;6m background'
              '\x1b[0m reset').encode() + frame_end
    rendered, colors = split_frame(redraw)
    assert 'resized-é' in rendered and 'probe-nested' not in rendered, rendered
    assert len(colors) == terminal_rows and len(colors[0]) == terminal_columns
    assert colors[0][0] == (2, 12, 34, 56), 'RGB foreground does not survive the resize'
    assert colors[1][0] == colors[1][8] == (5, 123), 'background SGR changes foreground'
    assert colors[1][18] is None, 'SGR reset retains a foreground'
    for _ in range(3):
        rendered, colors = split_frame(b'\x1b[3;1Hcargo check probe-nested' + frame_end)
        assert rendered.count('probe-nested') == 1 and 'resized-é' in rendered
        assert colors[0][0] == (2, 12, 34, 56), 'redraw loses unchanged cell colors'
    terminal_rows, terminal_columns = 6, 40
    rendered, colors = split_frame(b'\x1b[4;1Hcargo check probe-nested' + frame_end)
    assert len(colors) == 6 and len(colors[0]) == 40, 'shrinking keeps stale geometry'
    assert rendered.count('probe-nested') == 2, 'snapshot removes a real duplicate'
    offset = terminal_frames.offset
    assert terminal_snapshot() == (rendered, colors) and terminal_frames.offset == offset

    for index, abandoned in enumerate((b'\x1b[', b'\x1b[?25', b'\x1b]unfinished title', b'\x1b')):
        marker = f'recovered {index}'
        redraw = b'\x1b[2J\x1b[H' + marker.encode() + frame_end
        rendered, colors = split_frame(abandoned + redraw)
        assert rendered.rstrip() == marker, ('abandoned escape stalls redraw', abandoned, rendered)
        # Cancellation must also work when both escapes arrive in the same read.
        transcript.extend(abandoned + b'\x1b[2J\x1b[Hwhole read ' + marker.encode() + frame_end)
        assert terminal_snapshot()[0].rstrip() == 'whole read ' + marker

    for terminator in (b'\x07', b'\x1b\\'):
        rendered, colors = split_frame(b'\x1b[2J\x1b[H\x1b]hidden title' + terminator
                                       + b'visible text' + frame_end)
        assert rendered.rstrip() == 'visible text', 'split OSC leaks its contents into the frame'

    rendered, colors = split_frame(b'\x1b[2J\x1b[Hdiscard\x1b[\rkept\x1b[K' + frame_end)
    assert rendered.rstrip() == 'kept', 'invalid CSI consumes its cancelling carriage return'
    rendered, colors = split_frame('\x1b[2J\x1b[H\x1b[é visible'.encode() + frame_end)
    assert rendered.rstrip() == 'é visible', 'invalid CSI consumes its cancelling UTF-8 character'

def assert_reader_end_bounds_output_writer() -> None:
    global reader, terminal
    reader, terminal = pty.fork()
    if reader == 0:
        _ = signal.setitimer(signal.ITIMER_REAL, 0)
        _ = signal.signal(signal.SIGTERM, signal.SIG_IGN)
        _ = os.write(1, b'reader ready\n')
        while True:
            _ = os.write(1, b'pending reader output\n')
    def reader_is_ready() -> bool:
        read_terminal(0.01)
        return b'reader ready' in transcript
    wait_for(reader_is_ready, 'reader end stand-in to become ready', pause=0,
             deadline=time.monotonic() + 0.2)
    end_reader(signal.SIGTERM, exit_seconds=0.02, killed_exit_seconds=0.2)

def assert_concurrent_deadline_names_pending_wait() -> None:
    # The deadline is armed below, once only the second wait is pending.
    cancel_scenario_deadline()
    first_started = threading.Event()
    second_started = threading.Event()
    release_second = threading.Event()
    def finish_first_wait() -> None:
        with named_wait('finished concurrent predicate'):
            first_started.set()
            assert second_started.wait(0.2), 'second concurrent wait does not start'
    def hold_second_wait() -> None:
        assert first_started.wait(0.2), 'first concurrent wait does not start'
        with named_wait('concurrent pending predicate'):
            second_started.set()
            release_second.wait()
    executor = ThreadPoolExecutor(max_workers=2)
    try:
        first = executor.submit(finish_first_wait)
        second = executor.submit(hold_second_wait)
        first.result(timeout=0.2)
        arm_scenario_deadline()
        second.result()
    finally:
        release_second.set()
        executor.shutdown()

arm_scenario_deadline()

if scenario == '--terminal-frame-self-check':
    assert_completed_terminal_frames()
    record_timestamp('assertions')
    record_timestamp('cleanup')
    cancel_scenario_deadline()
    sys.exit(0)

if scenario == '--reader-end-self-check':
    assert_reader_end_bounds_output_writer()
    record_timestamp('assertions')
    record_timestamp('cleanup')
    cancel_scenario_deadline()
    sys.exit(0)

if scenario == '--concurrent-deadline-self-check':
    try:
        assert_concurrent_deadline_names_pending_wait()
    finally:
        record_timestamp('cleanup')

if scenario == '--scenario-deadline-self-check':
    try:
        wait_for(lambda: False, 'deadline self-check predicate')
    finally:
        record_timestamp('cleanup')

try:
    prepare_fixture()
    if scenario == 'cpu-cache-server':
        prepare_cpu_workload()
    arguments: tuple[str, ...] = ()
    first_arguments = ('--target-dir', str(work / 'target')) if scenario == 'cpu-cache-server' else ()
    first_name = 'probe-first'
    first_command = 'build'
    nested_directory: Path | None = None
    json_format = ('--message-format=json',)
    if scenario == 'quiet-json-long':
        quiet = '--quiet'
        arguments = (quiet, *json_format, quiet, '--', '--quiet', '-q')
        first_name = 'probe-json'
        first_command = 'check'
        first_arguments = arguments
    elif scenario == 'excluded':
        first_name = 'probe-enclosing'
        first_command = 'clippy'
        nested_directory = home / ('nested-directory-' + root.name)
    retained: list[Path] = []
    removed: list[Path] = []
    # Scenario setup assigns these; the same scenario reads them again after the reader scans.
    unrelated: StartedWriter | None = None
    carrier: RegistrationCarrier | None = None
    reader_environment = dict(environment, LC_ALL='C', LANG='POSIX', TZ='UTC-11')
    # Family assertions observe ANSI foregrounds even when the outer test runner is uncolored.
    _ = reader_environment.pop('NO_COLOR', None)
    if scenario == 'cpu-cache-server':
        idle = root / 'idle-workload.sh'
        _ = idle.write_text('while [ ! -f "$OBSERVED/release" ]; do sleep 0.005; done\n')
        unrelated_directory = home / ('unrelated-cpu-' + root.name)
        unrelated_directory.mkdir()
        _ = shutil.copyfile(work / 'build', unrelated_directory / 'build')
        other_target = unrelated_directory / 'target'
        with ThreadPoolExecutor(max_workers=2) as executor:
            first_start = executor.submit(
                start_writer, first_name, home, first_command, nested_directory,
                arguments=first_arguments)
            unrelated_start = executor.submit(
                start_writer, 'probe-unrelated', home, directory=unrelated_directory,
                command='build', arguments=('--target-dir', str(other_target)),
                additional_environment=(('CPU_WORKLOAD', str(idle)),))
            finish_cpu_workload_start()
            first = first_start.result()
            unrelated = unrelated_start.result()
    else:
        first = start_writer(first_name, home, first_command, nested_directory,
                             arguments=first_arguments)

    quiet_writer = first if scenario == 'quiet-json-long' else None
    enclosing = first if scenario == 'excluded' else None
    if scenario == 'cpu-cache-server':
        wait_for(lambda: (first[1] / 'cpu-ready').exists(),
                 'invocation baseline is not established', pause=0.005)
        cache_server = required(cache_server, 'prepare_cpu_workload does not assign cache_server')
        assert process_parent(cache_server) == os.getpid()
        wait_for(lambda: (root / 'cpu-server/compiler-pid').exists()
                 and (root / 'cpu-server/compiler-pid').read_text(),
                 'server does not start rustc', pause=0.005)
        compiler_pid = int((root / 'cpu-server/compiler-pid').read_text())
        assert process_parent(compiler_pid) == cache_server
        client_pid = int((first[1] / 'client-pid').read_text())
        assert process_parent(client_pid) == int((first[1] / 'cargo-pid').read_text())
        unrelated_ready = required(unrelated, 'CPU setup does not assign unrelated')[1] / 'cpu-ready'
        wait_for(lambda: unrelated_ready.exists(),
                 'second invocation baseline is not established', pause=0.005)
    if scenario == 'quiet-json-long':
        quiet_writer = required(quiet_writer, 'quiet-json-long setup does not assign quiet_writer')
        observed = (quiet_writer[1] / 'arguments').read_bytes().split(b'\0')
        assert observed == [quiet_writer[1].name.encode(), *(value.encode() for value in json_format),
                            b'--', b'--quiet', b'-q', b''], observed
        retained.extend((quiet_writer[2], quiet_writer[4]))
    if scenario == 'child-source-switch':
        start_reader_process(reader_environment)
        carrier = start_registration_carrier(first)
        retained.extend((carrier[2], carrier[4]))
    if scenario == 'root-headings':
        copy_publication_to_foreign_root(first)
    if scenario == 'excluded':
        enclosing = required(enclosing, 'excluded setup does not assign enclosing')
        retained.extend((enclosing[2], enclosing[4]))
        # A removed sibling proves cleanup ran while the excluded capture
        # and its still-running nested commands retained their artifacts.
        removed.extend(seed_ended_publication(enclosing))

    if reader is None:
        start_reader_process(reader_environment)

    def reader_has_scanned():
        read_terminal(0.1)
        rendered = screen()
        marker = ('probe-nested-' + root.name + '-check'
                  if scenario == 'excluded' else first[1].name)
        marker_is_ready = marker in rendered
        if scenario == 'excluded':
            marker_is_ready = all('probe-nested-' + root.name + '-' + command in rendered
                                  for command in ('check', 'test'))
        elif scenario == 'child-source-switch':
            marker_is_ready = carrier_source_in_rendered(
                required(carrier, 'child-source-switch setup does not assign carrier'),
                'process', rendered)
        return marker_is_ready and 'summary' in rendered
    wait_for(reader_has_scanned, 'production reader does not display the live cargo row', pause=0)
    record_timestamp('first frame')
    rendered = screen()
    assert 'summary' in rendered, rendered
    if scenario == 'cpu-cache-server':
        rendered = assert_cpu_workload(
            first, required(unrelated, 'cpu-cache-server setup does not assign unrelated'), rendered)
    if scenario == 'settings-scroll-burst':
        assert_settings_scroll()
    if scenario == 'quiet-json-long':
        quiet_writer = required(quiet_writer, 'quiet-json-long setup does not assign quiet_writer')
        cargo_pid = (quiet_writer[1] / 'cargo-pid').read_text()
        rows = [line for line in rendered.splitlines() if quiet_writer[1].name in line]
        assert len(rows) == 1, 'quiet JSON produces multiple invocation rows\n' + rendered
        cargo_pid = (quiet_writer[1] / 'cargo-pid').read_text()
        assert re.match(r'^\s*│\s*' + cargo_pid + r'\s', rows[0]), rendered
        assert 'cargo check ' + quiet_writer[1].name in rows[0], rendered
        assert 'blocked' in rows[0], rendered
        settings = settings_screen()
        assert 'ambiguous' not in settings and 'unconfirmed selection' not in settings, settings
    if scenario == 'child-source-switch':
        assert_child_source_switch(required(carrier, 'child-source-switch setup does not assign carrier'))
    if scenario == 'root-headings':
        account = pwd.getpwuid(capture.stat().st_uid).pw_name
        heading = '[' + account + '] ~/' + work.name
        assert sum(first[1].name in line for line in rendered.splitlines()) == 1, rendered
        assert sum(heading in line for line in rendered.splitlines()) == 1, rendered
        assert foreign_work.name not in rendered, rendered
        assert '[' + str(other_uid) + ']' not in rendered, rendered
        settings = settings_screen()
        assert str(capture_parent) in settings and '1777' in settings, settings
        assert any(account in line and 'yours · 1 capture ·' in line
                   for line in settings.splitlines()), settings
        assert not any('cleanup' in row.lower() for row in capture_settings_rows(settings, account)), settings
        ignored = str(other_capture) + ': owned by ' + account + ', not by ' + str(other_uid) + ' — ignored'
        assert ignored in settings, settings
        assert not any('configured' in row.lower() for row in capture_settings_rows(settings, account)), settings

    if scenario == 'locale':
        assert any(first[1].name in line and 'blocked' in line
                   for line in rendered.splitlines()), rendered
    assert first[2].exists() and first[4].exists(), 'live record is removed by reader'
    if scenario == 'excluded':
        rendered = assert_excluded_command(required(enclosing, 'excluded setup does not assign enclosing'),
                                           required(nested_directory, 'excluded setup does not assign nested_directory'),
                                           rendered)
    for path in removed:
        assert not path.exists(), 'reader retains ended artifact: ' + str(path) + '\n' + rendered
    for path in retained:
        assert path.exists(), 'reader removes unknown or live artifact: ' + str(path)
    record_timestamp('assertions')
    print(rendered)
except Exception:
    if scenario == 'cpu-cache-server':
        try:
            output = (root / 'cpu-server/output').read_text(errors='replace')
        except OSError as error:
            output = 'cannot read retained server output: ' + str(error)
        print('cache server output before cleanup:\n' + output, file=sys.stderr)
    raise
finally:
    record_timestamp('cleanup start')
    try:
        if cache_server is not None:
            (root / 'cpu-server/release').touch()
            try:
                os.killpg(cache_server, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for observations in parent_owned_children:
            trigger_writer(observations, 'release')
        # Release every writer before waiting on any, so their release polls overlap.
        for _, observations in writers:
            for nested_command in ('check', 'test'):
                if (observations / nested_command).is_dir():
                    (observations / nested_command / 'release').touch()
            trigger_writer(observations, 'release')
        record_timestamp('cleanup releases sent')
    finally:
        try:
            if reader is not None and reader != 0:
                request = signal.SIGTERM if scenario == 'settings-scroll-burst' else b'q'
                end_reader(request)
            record_timestamp('cleanup reader ended')
        finally:
            for server_process in detached_servers:
                description = 'cache server to exit'
                with named_wait(description):
                    try:
                        _ = server_process.wait(timeout=5)
                    except subprocess.TimeoutExpired as error:
                        raise AssertionError(description) from error
            record_timestamp('cleanup servers ended')
            for child, observations in writers:
                description = 'writer ' + observations.name + ' to exit'
                try:
                    with named_wait(description):
                        _ = child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    description = 'writer ' + observations.name + ' to exit after SIGKILL'
                    with named_wait(description):
                        try:
                            _ = child.wait(timeout=WRITER_KILLED_EXIT_SECONDS)
                        except subprocess.TimeoutExpired as error:
                            raise AssertionError(description) from error
                record_timestamp('cleanup writer ended ' + observations.name)
            for notification in writer_notifications.values():
                notification.close()
            record_timestamp('cleanup notifications closed')
        record_timestamp('cleanup')

cancel_scenario_deadline()
