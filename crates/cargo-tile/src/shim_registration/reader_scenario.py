from datetime import datetime, timezone
import codecs
import errno
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
import time

root = Path(sys.argv[1]).resolve()
binary, source, scenario, registration_limit, poll_millis, cpu_scans = sys.argv[2:]
registration_limit = int(registration_limit)
poll_seconds = int(poll_millis) / 1000
cpu_scans = int(cpu_scans)
# Two CPU seconds within wait_for's ten-second limit preserve the original
# twenty-percent workload margin above the refusal rows' ten-percent ceiling.
refusal_cpu_seconds = 2
# Ratatui completes every cursorless draw with Crossterm's Hide command.
frame_end = b'\x1b[?25l'
registration_fallback_scenarios = ('fallback-nested-source-switch', 'fallback-root-duplicate',
                                   'fallback-selected-unknown')
# Parallel reader tests share the host census; leave room for every fixture's rows.
terminal_rows = 300
terminal_columns = 300
home = root / 'home'
work = home / ('repair-group-' + root.name)
capture_parent = root / 'capture'
capture = capture_parent / str(os.getuid())
other_uid = 4294967294
other_capture = capture_parent / str(other_uid)
pids = capture / 'state/pids'
bin_directory = root / 'bin'
for directory in (work, pids, bin_directory, root / 'config/cargo-tile',
                  home / 'Library/Application Support/cargo-tile', root / 'rustup/toolchains'):
    directory.mkdir(parents=True)

def copy_named_shell(path):
    # Darwin's /bin/sh re-execs another shell, losing the fixture process name.
    shutil.copyfile('/bin/bash' if sys.platform == 'darwin' else shutil.which('sh'), path)
    path.chmod(0o755)
    if sys.platform == 'darwin':
        # A relocated platform shell is killed before exec completes. Sign only
        # the owned fixture copy so it can run under its cargo/compiler name.
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(path)],
                       check=True, capture_output=True, text=True)

if scenario == 'settings-scroll-burst':
    account_directories = [capture_parent / str(other_uid - index) for index in range(24)]
    for directory in account_directories:
        directory.mkdir()
configuration = '[capture]\nauto_install = false\n'
if scenario in ('root-headings', 'summary-root-headings', 'root-duplicate', 'fallback-root-duplicate', 'fallback-selected-unknown'):
    (other_capture / 'state/pids').mkdir(parents=True)
    # Account discovery is automatic; no roots configuration is written.
configuration += '[tiles]\ninitial_rows = 100\n'
if scenario == 'excluded':
    configuration += '[commands]\nexcluded = ["clippy"]\n'
elif scenario == 'exec-excluded':
    configuration += '[commands]\nexcluded = ["run"]\n'
elif scenario == 'cpu-cache-excluded':
    configuration += '[commands]\nexcluded = ["check"]\n'
elif scenario in ('summary-root-headings', 'child-source-switch'):
    # These rows must lead their own groups to appear in the summary. Exclude
    # the outer test driver, using the operator's ordinary configuration surface.
    configuration += '[commands]\nexcluded = ["nextest"]\n'
for directory in (root / 'config/cargo-tile', home / 'Library/Application Support/cargo-tile'):
    (directory / 'config.toml').write_text(configuration)
shim_source = Path(source).read_text()
assignment = 'capture_parent=/tmp/cargo-tile'
assert shim_source.splitlines().count(assignment) == 1
(bin_directory / 'cargo').write_text(shim_source.replace(assignment, 'capture_parent=' + shlex.quote(str(capture_parent))))
copy_named_shell(bin_directory / 'cargo-tile-real')
(work / 'build').write_text('''printf '%s\\0' "$LC_ALL" "$TZ" "$LANG" "$HOME" > "$OBSERVED/environment"
printf '%s' "$$" > "$OBSERVED/cargo-pid"
printf '%s' "${CARGOTILE_NESTED-}" > "$OBSERVED/enclosing-pid"
printf '%s\\0' "$@" > "$OBSERVED/arguments"
printf 'process' > "$OBSERVED/source"
printf 'Blocking waiting for file lock on build directory\\n' >&2
if [ -n "${CPU_WORKLOAD-}" ]; then
    # Establish the invocation's own nonzero counter before its first scan.
    remaining=20000
    while [ "$remaining" -gt 0 ]; do remaining=$((remaining - 1)); done
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
        exec "$CARRIER_PYTHON" "$REGISTRATION_CARRIER"
    fi
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
shutil.copyfile(work / 'build', work / 'check')
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
                   RUSTUP_HOME=str(root / 'rustup'),
                   PYTHONUTF8='1',
                   LC_ALL=writer_locale, LANG=writer_locale,
                   TZ='EST5EDT,M3.2.0,M11.1.0', TERM='xterm-256color')
writers = []
parent_owned_children = []
cache_server = None
scan_reader = None
reader = None
terminal = None
transcript = bytearray()

def wait_for(predicate, description, diagnostics=None):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    details = diagnostics() if diagnostics is not None else ('\n' + screen() if transcript else '')
    raise AssertionError(description + details)

def start_writer(name, writer_home, command='build', nested_directory=None,
                 capture_root=capture, directory=work, arguments=()):
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
        child = subprocess.Popen(['sh', str(bin_directory / 'cargo'), command, name, *arguments],
                                 cwd=directory, env=child_environment, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=output, start_new_session=True)
    writers.append((child, observations))
    wait_for(lambda: (observations / 'cargo-pid').exists(), 'cargo does not start')
    publications = capture / 'state/pids'
    wait_for(lambda: any(publications.glob(str(child.pid) + '.*')), 'shim does not publish')
    registration = next(publications.glob(str(child.pid) + '.*'))
    fields = registration.read_bytes().split(b'\0')
    log = capture / os.fsdecode(fields[4])
    wait_for(lambda: log.exists() and b'Blocking waiting' in log.read_bytes(),
             'writer does not capture progress')
    if capture_root != capture:
        # The directory claims a different uid but still belongs to this process.
        # Its valid publication must be ignored; process census can still see cargo.
        destination = capture_root / 'state/pids' / registration.name
        registration.rename(destination)
        registration = destination
        destination = capture_root / log.name
        log.rename(destination)
        log = destination
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

class ParentOwnedChild:
    # The enclosing writer owns wait and process-group cleanup for this child.
    def __init__(self, pid):
        self.pid = pid

def start_registration_carrier(template):
    # A live Python process has kernel identity but no cargo argv. Publish the
    # shim's wire format for that lifetime, then exec cargo without changing pid.
    observations = root / ('probe-carrier-' + root.name)
    observations.mkdir()
    directory = home / ('registered-directory-' + root.name)
    directory.mkdir(parents=True)
    shutil.copyfile(work / 'build', directory / 'build')
    carrier_script = observations / 'carrier.py'
    carrier_script.write_text('''import os
from pathlib import Path
import subprocess
import time
observations = Path(os.environ['OBSERVED'])
(observations / 'source').write_text('registration')
while not (observations / 'release').exists():
    if (observations / 'start-children').exists() and not (observations / 'children-started').exists():
        for command in ('check', 'test'):
            child_environment = dict(os.environ, OBSERVED=str(observations / command),
                                     CARGOTILE_NESTED=str(os.getpid()))
            subprocess.Popen(['sh', os.environ['CARRIER_SHIM'], command,
                              os.environ['CARRIER_NESTED_MARKER'] + '-' + command],
                             cwd=os.environ['CARRIER_NESTED_WORK'], env=child_environment)
        (observations / 'children-started').touch()
    if (observations / 'activate').exists():
        program = os.environ['CARRIER_CARGO']
        os.execv(program, [program, os.environ['CARRIER_COMMAND'], observations.name])
    time.sleep(0.02)
''')
    child_environment = dict(environment, HOME=str(home), OBSERVED=str(observations),
                             REGISTRATION_CARRIER=str(carrier_script), CARRIER_PYTHON=sys.executable,
                             CARRIER_CARGO=str(bin_directory / 'cargo-tile-real'),
                             CARRIER_COMMAND='build')
    if scenario == 'fallback-nested-source-switch':
        nested_directory = home / ('nested-carrier-' + root.name)
        nested_directory.mkdir()
        for nested_command in ('check', 'test'):
            (observations / nested_command).mkdir()
            shutil.copyfile(work / 'build', nested_directory / nested_command)
        child_environment.update(CARRIER_SHIM=str(bin_directory / 'cargo'),
                                 CARRIER_NESTED_WORK=str(nested_directory),
                                 CARRIER_NESTED_MARKER='probe-nested-' + root.name)
    if scenario == 'child-source-switch':
        parent_owned_children.append(observations)
        launch = ['env', *(key + '=' + value for key, value in child_environment.items()
                          if environment.get(key) != value), sys.executable, str(carrier_script)]
        launch_file = template[1] / 'spawn-child.tmp'
        launch_file.write_text(
            'cd ' + shlex.quote(str(directory)) + '\nexec ' + shlex.join(launch) + '\n')
        launch_file.rename(template[1] / 'spawn-child')
        observed_pid = template[1] / 'child-started'
        wait_for(lambda: observed_pid.exists() and observed_pid.read_text().isdigit(),
                 'readable parent does not spawn its only child')
        child = ParentOwnedChild(int(observed_pid.read_text()))
    else:
        with (observations / 'output').open('wb') as output:
            child = subprocess.Popen([sys.executable, str(carrier_script)], cwd=directory,
                                     env=child_environment, stdin=subprocess.DEVNULL,
                                     stdout=output, stderr=output, start_new_session=True)
        writers.append((child, observations))
    wait_for(lambda: (observations / 'source').exists(), 'registration carrier does not start')
    fields = list(template[3])
    if sys.platform == 'linux':
        fields[3] = Path('/proc/' + str(child.pid) + '/stat').read_bytes().rsplit(b') ', 1)[1].split()[19]
    else:
        started = subprocess.run(['ps', '-p', str(child.pid), '-o', 'lstart='], check=True,
                                 capture_output=True, text=True,
                                 env=dict(environment, LC_ALL='C', TZ='UTC0')).stdout.strip()
        fields[3] = str(int(datetime.strptime(started, '%a %b %d %H:%M:%S %Y')
                           .replace(tzinfo=timezone.utc).timestamp())).encode()
    fields[1] += b'-carrier'
    fields[4] = b'run-' + fields[1] + b'-' + str(child.pid).encode() + b'.log'
    fields[5:8] = [os.fsencode(directory), os.fsencode(home), b'2']
    fields[8:] = [b'build', observations.name.encode(), b'']
    registration = pids / (str(child.pid) + '.' + fields[1].decode())
    log = capture / os.fsdecode(fields[4])
    log.write_bytes(b'Blocking waiting for file lock on build directory\n')
    registration.write_bytes(b'\0'.join(fields))
    if scenario == 'fallback-nested-source-switch':
        (observations / 'start-children').touch()
        for nested_command in ('check', 'test'):
            observed_parent = observations / nested_command / 'enclosing-pid'
            wait_for(lambda: observed_parent.exists() and observed_parent.read_text() == str(child.pid),
                     'nested carrier command does not inherit registration')
    return child, observations, registration, fields, log

def end_writer(writer):
    child, observations = writer[:2]
    for nested_command in ('check', 'test'):
        if (observations / nested_command).is_dir():
            (observations / nested_command / 'release').touch()
    (observations / 'release').touch()
    assert child.wait(timeout=5) == 37, (observations / 'output').read_text()

def prepare_cpu_workload():
    global cache_server
    workload = root / 'cpu-workload.sh'
    environment['CPU_WORKLOAD'] = str(workload)
    environment['CPU_PYTHON'] = sys.executable
    if scenario == 'cpu-turnover':
        children = root / 'turnover.py'
        children.write_text('''import os
from pathlib import Path
import time

observations = Path(os.environ['OBSERVED'])
child_seconds = float(os.environ['CPU_CHILD_SECONDS'])
scans = Path(os.environ['CPU_SCANS'])

def completed_scans():
    try:
        return scans.read_text().count('\\n')
    except FileNotFoundError:
        return 0

def compile_child():
    started = time.process_time()
    while time.process_time() - started < child_seconds:
        pass
    first_scan = completed_scans()
    # One scan may already be in progress. Keeping the positive counter alive
    # through two further receipts lets the next complete scan observe it.
    while completed_scans() < first_scan + 2 and not (observations / 'release').exists():
        for step in range(10000):
            pass
    last_scan = completed_scans()
    if last_scan >= first_scan + 2:
        with (observations / 'busy-children').open('a') as history:
            history.write(f'{os.getpid()} {first_scan} {last_scan} {time.process_time() - started}\\n')
    os._exit(0)

busy = 0
with (observations / 'children').open('w', buffering=1) as history:
    while not (observations / 'release').exists():
        if busy == 0:
            busy = os.fork()
            if busy == 0:
                compile_child()
        started = time.monotonic()
        child = os.fork()
        if child == 0:
            # These new, idle descendants must never blank the invocation CPU.
            time.sleep(child_seconds)
            os._exit(0)
        waited, status = os.waitpid(child, 0)
        assert waited == child and status == 0
        history.write(f'{child} {time.monotonic() - started}\\n')
        waited, status = os.waitpid(busy, os.WNOHANG)
        if waited == busy:
            assert status == 0
            busy = 0
if busy:
    waited, status = os.waitpid(busy, 0)
    assert waited == busy and status == 0
''')
        environment['CPU_CHILD_SECONDS'] = str(poll_seconds / 5)
        environment['CPU_SCANS'] = str(root / 'cpu-scans')
        workload.write_text('exec "$CPU_PYTHON" ' + shlex.quote(str(children)) + '\n')
        return

    server = root / 'cpu-server'
    server.mkdir()
    environment['CPU_SERVER'] = str(server)
    environment['CPU_TARGET'] = str(work / 'target')
    (work / 'target/debug/deps').mkdir(parents=True)
    for name in ('sccache', 'rustc'):
        copy_named_shell(bin_directory / name)
    environment['RUSTC_WRAPPER'] = str(bin_directory / 'sccache')
    compiler = server / 'compile.sh'
    compiler.write_text('''printf '%s' "$$" > "$CPU_SERVER/compiler-pid"
while [ ! -f "$CPU_SERVER/release" ]; do :; done
''')
    service = server / 'serve.sh'
    service.write_text('''printf '%s' "$$" > "$CPU_SERVER/server-pid"
while [ ! -f "$CPU_SERVER/request" ]; do sleep 0.02; done
"$CPU_RUSTC" "$CPU_COMPILE" --crate-name cache_fixture --out-dir "$CPU_TARGET/debug/deps" &
wait
''')
    client = server / 'client.sh'
    client.write_text('''printf '%s' "$$" > "$OBSERVED/client-pid"
printf 'compile' > "$CPU_SERVER/request"
while [ ! -f "$OBSERVED/release" ]; do sleep 0.2; done
''')
    workload.write_text('exec "$RUSTC_WRAPPER" ' + shlex.quote(str(client)) + '\n')
    server_environment = dict(environment, CPU_RUSTC=str(bin_directory / 'rustc'),
                              CPU_COMPILE=str(compiler))
    # Reparent the server before cargo starts. A process supervisor can adopt
    # orphans instead of init, so ancestry is checked against the invocation below.
    with (server / 'output').open('wb') as output:
        launcher = subprocess.Popen([sys.executable, '-c', '''import os, sys
if os.fork():
    os._exit(0)
os.setsid()
os.execve(sys.argv[1], sys.argv[1:], dict(os.environ))
''', str(bin_directory / 'sccache'), str(service)], env=server_environment,
            stdin=subprocess.DEVNULL, stdout=output, stderr=output)
    launcher_status = launcher.wait(timeout=5)
    assert launcher_status == 0, (launcher_status, (server / 'output').read_text())
    wait_for(lambda: (server / 'server-pid').exists()
             and (server / 'server-pid').read_text(), 'cache server does not start')
    cache_server = int((server / 'server-pid').read_text())
    wait_for_cache_server_parent(cache_server, launcher.pid, server)

def process_parent(pid):
    return int(subprocess.run(['ps', '-p', str(pid), '-o', 'ppid='], check=True,
                              capture_output=True, text=True).stdout.strip())

def wait_for_cache_server_parent(pid, launcher_pid, server):
    observations = []
    def reparented():
        result = subprocess.run(['ps', '-p', str(pid), '-o', 'ppid='],
                                capture_output=True, text=True)
        observations[:] = [(result.returncode, result.stdout, result.stderr)]
        if result.returncode == 1:
            return False
        result.check_returncode()
        parent = result.stdout.strip()
        return bool(parent) and int(parent) > 0 and int(parent) != launcher_pid
    wait_for(reparented, 'cache server is not reparented',
             lambda: '\nlast ps observation: ' + repr(observations)
             + '\nserver output: ' + (server / 'output').read_text())

def process_ancestry(pid):
    ancestry = []
    while pid > 1:
        assert pid not in ancestry, ancestry
        ancestry.append(pid)
        pid = process_parent(pid)
    return ancestry

def compiler_cpu_seconds(pid):
    if sys.platform == 'linux':
        fields = Path('/proc/' + str(pid) + '/stat').read_text().rsplit(') ', 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')
    elapsed = subprocess.run(['ps', '-p', str(pid), '-o', 'time='], check=True,
                             capture_output=True, text=True).stdout.strip()
    seconds = 0
    for component in elapsed.split(':'):
        seconds = seconds * 60 + float(component)
    return seconds

def rendered_cpu(rendered, writer, markers):
    commands = fixture_pane(rendered, markers)
    pid = (writer[1] / 'cargo-pid').read_text()
    rows = [line for line in commands if writer[1].name in line
            and re.match(r'^\s*│\s*' + pid + r'\s', line)]
    assert len(rows) == 1, ('CPU fixture loses or duplicates its command row; expected pid=' + pid
                           + '\n' + subprocess.run(['ps', '-p', pid, '-o', 'pid=,ppid=,comm=,args='],
                                                  capture_output=True, text=True).stdout
                           + '\n' + rendered)
    # Header and row redraws can be observed separately; read CPU from this row.
    prefix, command, _ = rows[0].partition('cargo ')
    assert command, 'CPU fixture row has no cargo command\n' + rows[0]
    cells = prefix.replace('│', ' ').split()
    assert cells[0] == pid, rows[0]
    percentages = [cell for cell in cells if re.fullmatch(r'\d+%', cell)]
    assert len(percentages) == 1, 'CPU must be one numeric reading after the first scan\n' + rows[0]
    return int(percentages[0][:-1])

def assert_cpu_workload(writer, unrelated=None):
    visible_other = unrelated is not None and scenario != 'cpu-cache-excluded'
    markers = (writer[1].name, unrelated[1].name) if visible_other else (writer[1].name,)
    rendered = wait_for_fixture_pane(markers)
    readings = []
    other_readings = []
    refusal = scenario in ('cpu-cache-ambiguous', 'cpu-cache-excluded')
    if refusal:
        compiler_pid = int((root / 'cpu-server/compiler-pid').read_text())
        compiler_started = compiler_cpu_seconds(compiler_pid)
        compiler_consumed = 0
    # Inspect throughout several reporting windows, including child replacements.
    started = time.monotonic()
    deadline = started + 4
    def observe_cpu():
        nonlocal rendered, compiler_consumed
        if refusal:
            compiler_consumed = compiler_cpu_seconds(compiler_pid) - compiler_started
        readings.append((time.monotonic() - started, rendered_cpu(rendered, writer, markers)))
        if visible_other:
            other_readings.append(rendered_cpu(rendered, unrelated, markers))
        if scenario == 'cpu-cache-excluded':
            assert unrelated[1].name not in rendered, 'excluded owner still displays a row\n' + rendered
        rendered = wait_for_fixture_pane(markers)
        return time.monotonic() >= deadline and (
            not refusal or compiler_consumed >= refusal_cpu_seconds)
    wait_for(observe_cpu, 'compiler does not complete the observed CPU workload',
             lambda: repr((readings, compiler_consumed if refusal else 'not a refusal')))
    if scenario in ('cpu-cache-ambiguous', 'cpu-cache-excluded'):
        assert max(cpu for elapsed, cpu in readings) < 10, ('first candidate borrows server CPU', readings)
        if visible_other:
            assert max(other_readings) < 10, ('second candidate borrows server CPU', other_readings)
        else:
            assert unrelated[2].exists() and unrelated[4].exists(), 'excluded owner loses capture artifacts'
        # Startup CPU is excluded. Keep observing rows while a loaded compiler
        # accumulates the required work, bounded by the existing wait_for limit.
        assert compiler_consumed >= refusal_cpu_seconds, (
            'external compiler must consume CPU during refusal', compiler_consumed, refusal_cpu_seconds)
    else:
        sustained = [cpu for elapsed, cpu in readings if elapsed >= 2]
        assert len(sustained) >= 3 and min(sustained) >= 10, (
            'compile CPU must remain charged across reporting windows', readings)
        if unrelated is not None:
            assert max(other_readings) < min(sustained) / 2, (readings, other_readings)
    if scenario == 'cpu-turnover':
        completed = [line.split() for line in (writer[1] / 'children').read_text().splitlines()]
        assert len({pid for pid, elapsed in completed if float(elapsed) < poll_seconds}) >= 10, completed
        compiled = [line.split() for line in (writer[1] / 'busy-children').read_text().splitlines()]
        assert len({pid for pid, first_scan, last_scan, cpu in compiled}) >= 3, compiled
        assert all(int(last_scan) >= int(first_scan) + 2 and float(cpu) >= poll_seconds / 5
                   for pid, first_scan, last_scan, cpu in compiled), compiled
    observations = finish_cpu_scans()
    if scenario == 'cpu-cache-excluded':
        assert all(int(cpu[:-1]) < 10 for index, cpu in observations[2:]), (
            'a completed scan charges the excluded requester to the visible build', observations)
    return rendered

def finish_cpu_scans():
    assert scan_reader.wait(timeout=10) == 0, (root / 'cpu-scanner-output').read_text()
    observations = [line.split('\t') for line in (root / 'cpu-scans').read_text().splitlines()]
    assert [int(index) for index, cpu in observations] == list(range(cpu_scans)), observations
    assert all(re.fullmatch(r'\d+%', cpu) for index, cpu in observations[1:]), (
        'each completed scan after the first must publish a measurement', observations)
    return observations

def assert_cpu_identity_recovery(writer):
    compiler_pid = int((root / 'cpu-server/compiler-pid').read_text())
    scans = root / 'cpu-scans'
    wait_for(lambda: scans.exists() and len(scans.read_text().splitlines()) >= cpu_scans // 2,
             'process identity never receives its initial CPU samples')
    restored = Path(str(writer[2]) + '.tmp')
    restored.write_bytes(b'\0'.join(writer[3]))
    restored.replace(writer[2])
    observations = finish_cpu_scans()
    identities = (root / 'cpu-identities').read_text().splitlines()
    assert len(identities) == cpu_scans, identities
    assert identities[0] == 'process' and identities[-1] == 'captured', identities
    changed = identities.index('captured')
    assert identities == ['process'] * changed + ['captured'] * (cpu_scans - changed), identities
    before = [int(cpu[:-1]) for index, cpu in observations[2:changed]]
    after = [int(cpu[:-1]) for index, cpu in observations[changed:]]
    assert len(before) >= 3 and len(after) >= 3, ('identity transition lacks samples', observations)
    assert min(before + after) >= 10, ('identity recovery strands live compiler time', observations)
    # Only one CPU-consuming compiler runs, so replaying its accumulated history
    # would exceed the processor time available in an ordinary reporting interval.
    assert max(after) <= 150, ('identity recovery charges prior compiler history again', observations)
    assert int((root / 'cpu-server/compiler-pid').read_text()) == compiler_pid
    assert process_parent(compiler_pid) == cache_server, 'fixture replaces or ends the credited compiler'

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

class CompletedTerminalFrames:
    """Keep the mutable draw separate from the last completed screen."""

    # Match the valid prefix even when its terminator has not arrived.
    csi = re.compile(r'\x1b\[[0-?]*[ -/]*([@-~]?)')
    osc = re.compile(r'\x1b\][^\x07\x1b]*(\x07|\x1b\\)?')

    def __init__(self):
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.pending = ''
        self.offset = 0
        self.row = self.column = 0
        self.foreground = None
        self.cells = []
        self.colors = []
        self.rows = self.columns = 0
        self.published = ('', [])

    def resize(self, rows, columns):
        if (rows, columns) == (self.rows, self.columns):
            return
        self.cells = [(line[:columns] + [' '] * max(0, columns - len(line)))
                      for line in self.cells[:rows]]
        self.colors = [(line[:columns] + [None] * max(0, columns - len(line)))
                       for line in self.colors[:rows]]
        self.cells.extend([[' '] * columns for _ in range(rows - len(self.cells))])
        self.colors.extend([[None] * columns for _ in range(rows - len(self.colors))])
        self.rows, self.columns = rows, columns

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        index = 0
        while index < len(self.pending):
            character = self.pending[index]
            if character == '\x1b':
                if index + 1 == len(self.pending):
                    break
                introducer = self.pending[index + 1]
                if introducer == '[':
                    match = self.csi.match(self.pending, index)
                    if not match[1] and match.end() == len(self.pending):
                        break
                    if match[1]:
                        token = match[0]
                        self.command(token[-1], token[2:-1])
                    # An invalid prefix ends before the byte that cancels it.
                    index = match.end()
                    continue
                if introducer == ']':
                    match = self.osc.match(self.pending, index)
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

    def command(self, command, parameters):
        if command == 'l' and parameters == '?25':
            self.published = ('\n'.join(''.join(line).rstrip() for line in self.cells),
                              [line.copy() for line in self.colors])
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

def terminal_snapshot():
    terminal_frames.resize(terminal_rows, terminal_columns)
    terminal_frames.feed(transcript[terminal_frames.offset:])
    terminal_frames.offset = len(transcript)
    return terminal_frames.published

def screen():
    return terminal_snapshot()[0]

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

def fixture_panes(rendered, markers):
    return [commands for commands in command_panes(rendered)
            if all(any(marker in line for line in commands) for marker in markers)]

def fixture_pane(rendered, markers):
    matches = fixture_panes(rendered, markers)
    assert len(matches) == 1, 'fixture must occupy one command pane\n' + rendered
    return matches[0]

def wait_for_fixture_pane(markers):
    rendered = ''
    matches = []
    def pane_is_ready():
        nonlocal rendered, matches
        read_terminal(0.1)
        rendered = screen()
        matches = fixture_panes(rendered, markers)
        return len(matches) == 1
    wait_for(pane_is_ready, 'fixture must occupy one command pane',
             lambda: f'; observed {len(matches)} matching panes\n' + rendered)
    return rendered

def summary_pane(rendered):
    lines = rendered.splitlines()
    beginning = next(index for index, line in enumerate(lines) if '┌ summary' in line)
    ending = next(index for index in range(beginning + 1, len(lines))
                  if re.match(r'^\s*[└├╰╞╘].*[─━═]{3}', lines[index]))
    return lines[beginning + 1:ending]

def summary_row_is_unobscured(rendered, marker, following_marker):
    global terminal_rows
    try:
        summary = summary_pane(rendered)
    except StopIteration:
        # Resizing can leave a partial frame until the summary borders redraw.
        return False
    rows = [index for index, line in enumerate(summary) if marker in line]
    following = [index for index, line in enumerate(summary) if following_marker in line]
    if (rows and following and max(rows) < min(following)
            and all(index < len(summary) - 1 and 'content rows:' not in summary[index]
                    for index in rows)):
        return True
    # Marker visibility alone can accept the footer after its sizing readout
    # overwrites compiler/runs. Give the summary more room, then await a redraw.
    # Keep readiness geometric: incorrect measurements must still fail below.
    if terminal_rows < 2400:
        terminal_rows *= 2
        fcntl.ioctl(terminal, termios.TIOCSWINSZ,
                    struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
        read_terminal(0.1)
    return False

def assert_registered_row(rendered, writer):
    commands = fixture_pane(rendered, (writer[1].name,))
    rows = [line for line in commands if writer[1].name in line]
    assert len(rows) == 1, 'registration duplicates its invocation\n' + rendered
    assert 'cargo ' + writer[3][8].decode() + ' ' + writer[1].name in rows[0], rendered
    assert re.match(r'^\s*│\s*' + str(writer[0].pid) + r'\s', rows[0]), rendered
    directory = Path(os.fsdecode(writer[3][5]))
    headings = [line for line in commands if directory.name in line]
    assert len(headings) == 1, 'registration loses or duplicates its heading\n' + rendered
    account = pwd.getpwuid(capture.stat().st_uid).pw_name
    display = ('~/' + str(directory.relative_to(home))
               if writer[3][6] == os.fsencode(home) else str(directory))
    assert '[' + account + '] ' + display in headings[0], rendered
    # Other tests share this process tree and can add rows before this fixture.
    # Compare the fixture's directory order across scans, not its screen offset.
    directories = (work.name, directory.name, 'nested-carrier-' + root.name)
    fixture_headings = [line for line in commands if any(name in line for name in directories)]
    return rows[0], fixture_headings.index(headings[0])

def unavailable_measurements(row):
    # The final runs cell can touch the pane border without trailing padding.
    # A border delimits that cell just as whitespace delimits the inner cells.
    return row.replace('│', ' ').split().count('--')

rows_readout = re.compile(r'content rows: (\d+)(?: @ \d+)?  r/c: (\d+)/(\d+)')

def carrier_source_is_rendered(writer, source):
    global terminal_rows
    read_terminal(0.1)
    rendered = screen()
    rows = [line for pane in command_panes(rendered) for line in pane if writer[1].name in line]
    if not rows and scenario == 'child-source-switch' and terminal_rows < 2400:
        # Concurrent host rows can compress the parent's pane below its child.
        # Resize only an observed clipped pane, not the reader's startup frame.
        panes = fixture_panes(rendered, (first[1].name,))
        if len(panes) == 1:
            # The final interior row belongs to the readout, not the content.
            sizes = [rows_readout.search(line) for line in panes[0]]
            if any(size and int(size[1]) > int(size[2]) - 1 for size in sizes):
                terminal_rows *= 2
                fcntl.ioctl(terminal, termios.TIOCSWINSZ,
                            struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
    if len(rows) != 1:
        return False
    # A sleeping process may never earn a CPU baseline. Its observed compiler
    # absence and managed count still distinguish it from registration-only data.
    unavailable = unavailable_measurements(rows[0])
    expected = 2 if scenario == 'fallback-nested-source-switch' else 3
    return unavailable == expected if source == 'registration' else unavailable < expected

def assert_child_family(parent, child):
    rendered = wait_for_fixture_pane((parent[1].name, child[1].name))
    # No PTY read separates the ready screen from its matching color snapshot.
    colors = terminal_snapshot()[1]
    commands = fixture_pane(rendered, (parent[1].name, child[1].name))
    rows = {}
    for writer in (parent, child):
        matching = [line for line in commands if writer[1].name in line]
        assert len(matching) == 1, 'source change duplicates an invocation\n' + rendered
        rows[writer[0].pid] = matching[0]
    parent_row, child_row = rows[parent[0].pid], rows[child[0].pid]
    parent_pid = (parent[1] / 'cargo-pid').read_text()
    assert re.match(r'^\s*│\s*' + parent_pid + r'\s', parent_row), rendered
    assert re.match(r'^\s*│\s*' + str(child[0].pid) + r'\s+' + parent_pid + r'\s',
                    child_row), rendered
    # The readable lead counts its only assembled child, whichever source supplies it.
    assert re.search(r'\s1\s*│\s*$', parent_row), 'parent loses its managed child\n' + rendered
    lines = rendered.splitlines()
    parent_line, child_line = lines.index(parent_row), lines.index(child_row)
    parent_column = parent_row.index(parent_pid)
    child_column = child_row.index(str(child[0].pid))
    reference_column = child_row.index(parent_pid, child_column + len(str(child[0].pid)))
    family = colors[parent_line][parent_column]
    assert family is not None, 'terminal parser does not observe the family color'
    assert family == colors[child_line][reference_column], 'child loses its parent family color'
    assert family != colors[child_line][child_column], 'parent has plain pid color instead of a family'
    child_header = next(line for line in reversed(lines[:child_line])
                        if 'parent' in line and 'command' in line)
    starts = tuple(row[child_header.index('start'):child_header.index('dur')].strip()
                   for row in (parent_row, child_row))
    assert all(starts), 'start cells are absent\n' + rendered
    headings = tuple(line.strip() for line in commands
                     if work.name in line or Path(os.fsdecode(child[3][5])).name in line)
    assert len(headings) == 2, 'source change loses a directory heading\n' + rendered
    return family, starts, headings

def assert_carrier_children(rendered, writer):
    markers = ['probe-nested-' + root.name + '-' + command for command in ('check', 'test')]
    commands = fixture_pane(rendered, (writer[1].name, *markers))
    parent_row = next(line for line in commands if writer[1].name in line)
    assert re.search(r'\s2\s*│\s*$', parent_row), 'parent must count both assembled children\n' + rendered
    for command, marker in zip(('check', 'test'), markers):
        rows = [line for line in commands if marker in line]
        assert len(rows) == 1, 'source transition duplicates a nested row\n' + rendered
        assert 'cargo ' + command + ' ' + marker in rows[0], rendered
        pid = (writer[1] / command / 'cargo-pid').read_text()
        assert re.match(r'^\s*│\s*' + pid + r'\s+' + str(writer[0].pid) + r'\s', rows[0]), rendered
    heading = '[' + pwd.getpwuid(capture.stat().st_uid).pw_name + '] ~/nested-carrier-' + root.name
    assert sum(heading in line for line in commands) == 1, rendered

def publish_other_root(writer):
    fields = list(writer[3])
    fields[5] = os.fsencode(home / ('unused-directory-' + root.name))
    fields[8:] = [b'test', ('probe-unused-' + root.name).encode(), b'']
    (other_capture / 'state/pids' / writer[2].name).write_bytes(b'\0'.join(fields))
    shutil.copyfile(writer[4], other_capture / writer[4].name)

def expand_arguments(expected_rows):
    os.write(terminal, b'p')
    def arguments_are_rendered():
        read_terminal(0.1)
        rows = [line for commands in command_panes(screen()) for line in commands]
        return all(any(re.match(r'^\s*│\s*' + str(pid) + r'\s', line) and command in line
                       for line in rows) for pid, command in expected_rows)
    wait_for(arguments_are_rendered, 'full command arguments do not finish rendering')
    return screen()

def settings_screen():
    os.write(terminal, b's')
    def settings_are_visible():
        read_terminal(0.1)
        rendered = screen()
        return 'Capture:' in rendered and 'auto install' in rendered and 'Commands:' in rendered
    wait_for(settings_are_visible, 'settings do not open')
    rendered = screen()
    os.write(terminal, b'\x1b')
    def settings_are_closed():
        read_terminal(0.1)
        return 'Capture:' not in screen()
    wait_for(settings_are_closed, 'settings do not close')
    return rendered

def assert_settings_scroll():
    global terminal_rows, terminal_columns
    # Keep every account below the initial viewport without cleanup rows.
    terminal_rows = 10
    transcript_start = len(transcript)
    input_attributes = termios.tcgetattr(terminal)
    if scenario == 'settings-scroll-burst':
        # Queue both events while the already-rendering reader is descheduled.
        os.kill(reader, signal.SIGSTOP)
        stopped, status = os.waitpid(reader, os.WUNTRACED)
        assert stopped == reader and os.WIFSTOPPED(status), (stopped, status)
    try:
        fcntl.ioctl(terminal, termios.TIOCSWINSZ,
                    struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
        written = os.write(terminal, b's')
    finally:
        if scenario == 'settings-scroll-burst':
            os.kill(reader, signal.SIGCONT)
    def popup_diagnostics():
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
        return ('\nsettings input: ' + repr({
            'written': written,
            'canonical': bool(input_attributes[3] & termios.ICANON),
            'echo': bool(input_attributes[3] & termios.ECHO),
            'unread_input': unread,
            'received_bytes': len(received),
            'settings_in_raw_output': b'Settings' in received,
            'completed_frames': received.count(frame_end),
            'raw_tail': received[-2048:],
        }) + '\n' + screen())
    def settings_are_visible():
        read_terminal(0.1)
        rendered = screen()
        return 'Settings' in rendered and any('▶' in line and 'mode' in line
                                             for line in rendered.splitlines())
    wait_for(settings_are_visible, 'small settings popup does not open', popup_diagnostics)
    initial = screen()
    assert all(str(directory) not in initial for directory in account_directories), initial
    pending = {str(directory) for directory in account_directories}
    selected_accounts = set()
    def accounts_are_selected():
        os.write(terminal, b'\x1b[B')
        read_terminal(0.1)
        rendered = screen()
        selected = '\n'.join(line for line in rendered.splitlines() if '▶' in line)
        selected_accounts.update(directory for directory in pending if directory in selected)
        return selected_accounts == pending
    wait_for(accounts_are_selected, 'keyboard navigation cannot select every drawn account',
             lambda: '\nmissing: ' + repr(sorted(pending - selected_accounts)) + '\n' + screen())
    selected = next(line for line in screen().splitlines() if '▶' in line)
    account = next(directory for directory in pending if directory in selected)
    configurations = {path: path.read_bytes() for path in
                      (root / 'config/cargo-tile/config.toml',
                       home / 'Library/Application Support/cargo-tile/config.toml')}
    os.write(terminal, b'\r\x1b[C\x1b[D')
    read_terminal(0.2)
    assert any('▶' in line and account in line for line in screen().splitlines()), screen()
    assert all(path.read_bytes() == contents for path, contents in configurations.items()), \
        'account navigation edits configuration'
    terminal_rows, terminal_columns = 9, 240
    fcntl.ioctl(terminal, termios.TIOCSWINSZ,
                struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
    def selection_survives_resize():
        read_terminal(0.1)
        return any('▶' in line and account in line for line in screen().splitlines())
    wait_for(selection_survives_resize, 'selected account disappears after terminal resize')
    remaining_settings = {'excluded', 'hidden when idle', 'config', 'themes', 'keymap'}
    def later_settings_are_selected():
        os.write(terminal, b'\x1b[B')
        read_terminal(0.1)
        selected = '\n'.join(line for line in screen().splitlines() if '▶' in line)
        remaining_settings.difference_update(label for label in tuple(remaining_settings)
                                             if re.search('▶\\s+' + re.escape(label) + '\\s', selected))
        return not remaining_settings
    wait_for(later_settings_are_selected, 'settings below account directories cannot be reached',
             lambda: '\nmissing: ' + repr(sorted(remaining_settings)) + '\n' + screen())
    os.write(terminal, b'\x1b')
    read_terminal(0.1)

def assert_completed_terminal_frames():
    global terminal_rows, terminal_columns
    # The readout comes from draw_cell_for_test, including the reserved footer.
    readout = os.environ['CARGO_TILE_TEST_ROWS_READOUT']
    size = rows_readout.search(readout)
    assert size is not None, 'resize-on-clip regex misses production output: ' + readout
    assert tuple(map(int, size.groups())) == (11, 10, 80), readout
    assert int(size[1]) > int(size[2]) - 1, 'clipped content must trigger a resize'
    assert rows_readout.search('content rows: 11 @ 60  r/c: 10/80').groups() == size.groups()

    def split_frame(frame):
        previous = terminal_snapshot()
        for byte in frame[:-1]:
            transcript.append(byte)
            assert terminal_snapshot() == previous, 'snapshot exposes an unfinished redraw'
        transcript.append(frame[-1])
        return terminal_snapshot()

    first = ('\x1b[2J\x1b[H│ pid parent command\r\n'
             '│ earlier row\r\n│ cargo check probe-nested\r\n'
             '│ cargo test probe-child\r\n└────').encode() + frame_end
    split_frame(first)
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

if scenario == 'terminal-frame-completion':
    assert_completed_terminal_frames()
    sys.exit(0)

try:
    if scenario.startswith('cpu-'):
        prepare_cpu_workload()
    first_arguments = ('--target-dir', str(work / 'target')) if scenario.startswith('cpu-cache-') else ()
    first = start_writer('probe-first', home, arguments=first_arguments)
    retained = []
    removed = []
    if scenario.startswith('cpu-'):
        wait_for(lambda: (first[1] / 'cpu-ready').exists(), 'invocation baseline is not established')
    if scenario.startswith('cpu-cache-'):
        assert int((first[1] / 'cargo-pid').read_text()) not in process_ancestry(cache_server)
        wait_for(lambda: (root / 'cpu-server/compiler-pid').exists()
                 and (root / 'cpu-server/compiler-pid').read_text(), 'server does not start rustc')
        compiler_pid = int((root / 'cpu-server/compiler-pid').read_text())
        assert process_parent(compiler_pid) == cache_server
        client_pid = int((first[1] / 'client-pid').read_text())
        assert process_parent(client_pid) == int((first[1] / 'cargo-pid').read_text())
        if scenario in ('cpu-cache-server', 'cpu-cache-identity-recovery'):
            idle = root / 'idle-workload.sh'
            idle.write_text('while [ ! -f "$OBSERVED/release" ]; do sleep 0.2; done\n')
            environment['CPU_WORKLOAD'] = str(idle)
        unrelated_directory = home / ('unrelated-cpu-' + root.name)
        unrelated_directory.mkdir()
        shutil.copyfile(work / 'build', unrelated_directory / 'build')
        shutil.copyfile(work / 'build', unrelated_directory / 'check')
        other_target = (work / 'target' if scenario in ('cpu-cache-ambiguous', 'cpu-cache-excluded')
                        else unrelated_directory / 'target')
        unrelated = start_writer('probe-unrelated', home, directory=unrelated_directory,
                                 command='check' if scenario == 'cpu-cache-excluded' else 'build',
                                 arguments=('--target-dir', str(other_target)))
        wait_for(lambda: (unrelated[1] / 'cpu-ready').exists(), 'second invocation baseline is not established')
        if scenario in ('cpu-cache-ambiguous', 'cpu-cache-excluded'):
            wait_for(lambda: (unrelated[1] / 'client-pid').exists()
                     and (unrelated[1] / 'client-pid').read_text(), 'second wrapper client does not start')
            assert process_parent(int((unrelated[1] / 'client-pid').read_text())) == int(
                (unrelated[1] / 'cargo-pid').read_text())
            assert int((unrelated[1] / 'cargo-pid').read_text()) not in process_ancestry(cache_server)
    if scenario.startswith('version-'):
        assert first[3][0] == b'cargo-tile-v3', first[3]
        carrier = start_registration_carrier(first)
        sweep_probe = start_writer('probe-sweep', home)
        sweep_bytes = sweep_probe[2].read_bytes()
        end_writer(sweep_probe)
        if scenario in ('version-newer-ended', 'version-newer-oversized'):
            (carrier[1] / 'release').touch()
            assert carrier[0].wait(timeout=5) == 0
            try:
                os.kill(carrier[0].pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('unsupported fixture pid remains present')
        supported_version = int(first[3][0].removeprefix(b'cargo-tile-v'))
        newer_version = supported_version + 1
        if scenario == 'version-malformed':
            contents = first[3][0] + b'\0partial\0'
        else:
            # A future payload need not have today's field count, UTF-8,
            # generation, birth, or log positions.
            contents = ('cargo-tile-v' + str(newer_version)).encode() + b'\0\xfffuture-layout\0'
            if scenario == 'version-newer-oversized':
                contents += b'x' * (registration_limit + 1 - len(contents))
                assert len(contents) > registration_limit
        carrier[2].write_bytes(contents)
        preserved = {path: path.read_bytes() for path in (carrier[2], carrier[4])}
        retained.extend(preserved)
    elif scenario.startswith('quiet-json'):
        quiet = '--quiet' if scenario == 'quiet-json-long' else '-q'
        json_format = (('--message-format', 'json') if scenario == 'quiet-json-separate'
                       else ('--message-format=json',))
        arguments = (quiet, *json_format, quiet, '--', '--quiet', '-q')
        quiet_writer = start_writer('probe-json', home, command='check', arguments=arguments)
        observed = (quiet_writer[1] / 'arguments').read_bytes().split(b'\0')
        assert observed == [quiet_writer[1].name.encode(), *(value.encode() for value in json_format),
                            b'--', b'--quiet', b'-q', b''], observed
        retained.extend((quiet_writer[2], quiet_writer[4]))
    elif scenario.startswith('rejected-rewrite'):
        executed_arguments, registered_arguments = {
            'rejected-rewrite-non-json-long': ((), ('--quiet',)),
            'rejected-rewrite-non-json-short': ((), ('-q',)),
            'rejected-rewrite-post-long': (
                ('--message-format=json', '--'),
                ('--quiet', '--message-format=json', '--', '--quiet')),
            'rejected-rewrite-post-short': (
                ('--message-format=json', '--'),
                ('-q', '--message-format=json', '--', '-q')),
            'rejected-rewrite-unrelated': (
                ('--message-format=json', '--release'),
                ('--quiet', '--message-format=json', '--workspace')),
        }[scenario]
        mismatched = start_writer('probe-mismatch', home, command='check',
                                  arguments=executed_arguments)
        observed = (mismatched[1] / 'arguments').read_bytes().split(b'\0')
        assert observed == [mismatched[1].name.encode(),
                            *(value.encode() for value in executed_arguments), b''], observed
        # Change only the external record's argv, retaining its live kernel proof.
        # An unrecognized rewrite must not give the live cargo direct ownership.
        fields = list(mismatched[3])
        fields[7] = str(2 + len(registered_arguments)).encode()
        fields[8:] = [b'check', mismatched[1].name.encode(),
                      *(value.encode() for value in registered_arguments), b'']
        mismatched[2].write_bytes(b'\0'.join(fields))
        retained.extend((mismatched[2], mismatched[4]))
    elif scenario == 'child-source-switch':
        carrier = start_registration_carrier(first)
        retained.extend((carrier[2], carrier[4]))
    elif scenario in registration_fallback_scenarios:
        carrier = start_registration_carrier(first)
        retained.extend((carrier[2], carrier[4]))
        if scenario == 'fallback-selected-unknown':
            publish_other_root(carrier)
        if scenario == 'fallback-selected-unknown':
            fields = list(carrier[3])
            fields[3] = b''
            carrier[2].write_bytes(b'\0'.join(fields))
        elif scenario == 'fallback-root-duplicate':
            publish_other_root(carrier)
    elif scenario in ('root-headings', 'summary-root-headings'):
        second = start_writer('probe-second', home, capture_root=other_capture)
    elif scenario == 'root-duplicate':
        publish_other_root(first)
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

    if scenario.startswith('cpu-'):
        if scenario == 'cpu-cache-identity-recovery':
            unproven = list(first[3])
            unproven[3] = b''
            first[2].write_bytes(b'\0'.join(unproven))
        scan_environment = dict(environment, CARGO_TILE_TEST_CPU_PID=(first[1] / 'cargo-pid').read_text())
        if scenario == 'cpu-cache-excluded':
            scan_environment['CARGO_TILE_TEST_CPU_EXCLUDED'] = 'check'
        with (root / 'cpu-scanner-output').open('wb') as output:
            scan_reader = subprocess.Popen([binary, '--exact', 'shim_registration::tests::cpu_scan_child', '--nocapture'],
                                           cwd=root, env=scan_environment, stdin=subprocess.DEVNULL,
                                           stdout=output, stderr=output)
        if scenario == 'cpu-cache-identity-recovery':
            assert_cpu_identity_recovery(first)
            sys.exit(0)
    reader_environment = dict(environment, LC_ALL='C', LANG='POSIX', TZ='UTC-11')
    # Family assertions observe ANSI foregrounds even when the outer test runner is uncolored.
    reader_environment.pop('NO_COLOR', None)
    reader, terminal = pty.fork()
    if reader == 0:
        fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', terminal_rows, terminal_columns, 0, 0))
        os.chdir(root)
        if scenario == 'settings-scroll-burst':
            (root / 'reader-terminal').write_text(os.ttyname(0))
        reader_environment['CARGO_TILE_TEST_READER'] = '1'
        os.execve(binary, [binary, '--exact', 'shim_registration::reader_child', '--nocapture'], reader_environment)
    def reader_has_scanned():
        read_terminal(0.1)
        rendered = screen()
        marker = first[1].name
        return marker in rendered and 'summary' in rendered
    wait_for(reader_has_scanned, 'production reader does not display the live cargo row')
    rendered = screen()
    assert 'summary' in rendered, rendered
    if scenario.startswith('cpu-'):
        rendered = assert_cpu_workload(first, unrelated if scenario.startswith('cpu-cache-') else None)
    if scenario.startswith('version-'):
        for scan in range(3):
            sweep_probe[4].write_bytes(b'Blocking waiting for file lock on build directory\n')
            sweep_probe[2].write_bytes(sweep_bytes)
            def sweep_finishes():
                read_terminal(0.1)
                return not sweep_probe[2].exists() and not sweep_probe[4].exists()
            wait_for(sweep_finishes, 'reader does not sweep the supported ended sibling')
            rendered = screen()
            assert carrier[1].name not in rendered, 'ineligible registration supplies a row\n' + rendered
            if scenario not in ('version-newer-ended', 'version-newer-oversized'):
                assert carrier[0].poll() is None, 'fixture writer ends before retention checks'
            for path, contents in preserved.items():
                assert path.read_bytes() == contents, 'reader changes retained artifact: ' + str(path)
            settings = ' '.join(settings_screen().replace('│', ' ').split())
            assert str(carrier[2]) in settings, settings
            if scenario == 'version-malformed':
                assert 'invalid registration' in settings.lower(), settings
                assert 'unsupported' not in settings.lower(), settings
            else:
                assert 'unsupported' in settings.lower(), settings
                assert f'v{newer_version}' in settings and f'v{supported_version}' in settings, settings
                assert 'upgrade' in settings.lower() and 'restart' in settings.lower(), settings
                assert 'invalid registration' not in settings.lower(), settings
    if scenario == 'settings-scroll-burst':
        assert_settings_scroll()
    if scenario.startswith('quiet-json'):
        cargo_pid = (quiet_writer[1] / 'cargo-pid').read_text()
        rendered = expand_arguments([(cargo_pid, 'cargo check ' + quiet_writer[1].name + ' ' + ' '.join(arguments))])
        commands = fixture_pane(rendered, (first[1].name, quiet_writer[1].name))
        rows = [line for line in commands if quiet_writer[1].name in line]
        assert len(rows) == 1, 'quiet JSON produces multiple invocation rows\n' + rendered
        cargo_pid = (quiet_writer[1] / 'cargo-pid').read_text()
        assert re.match(r'^\s*│\s*' + cargo_pid + r'\s', rows[0]), rendered
        assert 'cargo check ' + quiet_writer[1].name + ' ' + ' '.join(arguments) in rows[0], rendered
        assert 'blocked' in rows[0], rendered
        settings = settings_screen()
        association = 'capture association: pid ' + cargo_pid + ' via registration ' + str(quiet_writer[0].pid)
        assert association in ' '.join(settings.replace('│', ' ').split()), settings
    if scenario.startswith('rejected-rewrite'):
        cargo_pid = (mismatched[1] / 'cargo-pid').read_text()
        rendered = expand_arguments([(pid, ' '.join(('cargo', 'check', mismatched[1].name, *arguments)))
                                     for pid, arguments in ((cargo_pid, executed_arguments),
                                                            (str(mismatched[0].pid), registered_arguments))])
        rows = [line for commands in command_panes(rendered) for line in commands
                if mismatched[1].name in line]
        assert len(rows) == 2, 'unsupported argv rewrite gains direct ownership\n' + rendered
        cargo_pid = (mismatched[1] / 'cargo-pid').read_text()
        for pid, arguments in ((cargo_pid, executed_arguments),
                               (str(mismatched[0].pid), registered_arguments)):
            matching = [line for line in rows if re.match(r'^\s*│\s*' + pid + r'\s', line)]
            assert len(matching) == 1, 'process and registration lose distinct identities\n' + rendered
            expected = ' '.join(('cargo', 'check', mismatched[1].name, *arguments))
            assert expected in matching[0], 'row takes another invocation source command\n' + rendered
    if scenario == 'child-source-switch':
        wait_for(lambda: carrier_source_is_rendered(carrier, 'registration'),
                 'parent does not display its registration-only child')
        initial = assert_child_family(first, carrier)
        for source, trigger in (('process', 'activate'), ('registration', 'retire')):
            (carrier[1] / trigger).touch()
            wait_for(lambda: (carrier[1] / 'source').read_text() == source,
                     'child does not switch to ' + source)
            wait_for(lambda: carrier_source_is_rendered(carrier, source),
                     'reader does not observe child source ' + source)
            observed = assert_child_family(first, carrier)
            assert observed == initial, \
                ('child source change alters family color, start, or headings: '
                 + repr(initial) + ' became ' + repr(observed) + '\n' + screen())
    if scenario in registration_fallback_scenarios:
        if scenario == 'fallback-selected-unknown':
            assert carrier[1].name not in rendered, 'ineligible registration sources a row\n' + rendered
        else:
            def carrier_is_visible():
                read_terminal(0.1)
                rendered = screen()
                if scenario == 'fallback-nested-source-switch':
                    # A completed frame can precede the first draw of both children.
                    markers = [carrier[1].name, *('probe-nested-' + root.name + '-' + command
                                                  for command in ('check', 'test'))]
                    return len(fixture_panes(rendered, markers)) == 1
                return carrier[1].name in rendered
            wait_for(carrier_is_visible,
                     'verified registration does not supply an unobscured command row')
            rendered = screen()
            row, heading = assert_registered_row(rendered, carrier)
            expected_unavailable = 2 if scenario == 'fallback-nested-source-switch' else 3
            assert unavailable_measurements(row) == expected_unavailable, \
                'registration invents CPU, compiler, or managed measurements: ' + repr(row) + '\n' + rendered
            assert 'blocked' in row, rendered
            if scenario == 'fallback-nested-source-switch':
                if scenario == 'fallback-nested-source-switch':
                    assert_carrier_children(rendered, carrier)
                initial_heading = heading
                (carrier[1] / 'activate').touch()
                wait_for(lambda: (carrier[1] / 'source').read_text() == 'process',
                         'carrier does not exec cargo')
                wait_for(lambda: carrier_source_is_rendered(carrier, 'process'),
                         'process source never supplies compiler and managed observations')
                rendered = screen()
                row, heading = assert_registered_row(rendered, carrier)
                assert heading == initial_heading, 'source change moves the heading\n' + rendered
                if scenario == 'fallback-nested-source-switch':
                    assert_carrier_children(rendered, carrier)
                (carrier[1] / 'retire').touch()
                wait_for(lambda: (carrier[1] / 'source').read_text() == 'registration',
                         'cargo does not return to registration-only visibility')
                wait_for(lambda: carrier_source_is_rendered(carrier, 'registration'),
                         'registration source never returns to unavailable measurements')
                rendered = screen()
                row, heading = assert_registered_row(rendered, carrier)
                assert heading == initial_heading, 'reverse source change moves the heading\n' + rendered
                assert unavailable_measurements(row) == expected_unavailable, repr(row) + '\n' + rendered
                if scenario == 'fallback-nested-source-switch':
                    assert_carrier_children(rendered, carrier)
    if scenario in ('root-headings', 'root-duplicate'):
        markers = (first[1].name,) if scenario == 'root-duplicate' else (first[1].name, second[1].name)
        commands = fixture_pane(rendered, markers)
        for marker in markers:
            assert sum(marker in line for line in commands) == 1, rendered
        account = pwd.getpwuid(capture.stat().st_uid).pw_name
        heading = '[' + account + '] ~/' + work.name
        assert sum(heading in line for line in commands) == 1, rendered
        if scenario == 'root-headings':
            assert not any('[' + str(other_uid) + ']' in line for line in commands), rendered
            settings = settings_screen()
            assert str(capture_parent) in settings and '1777' in settings, settings
            account_lines = [line for line in settings.splitlines() if 'active captures' in line]
            assert any(account in line and 'yours' in line and 'readable' in line
                       and '1 active captures' in line
                       for line in account_lines), settings
            assert 'cleanup' not in settings.lower(), settings
            ignored = str(other_capture) + ': owned by ' + account + ', not by ' + str(other_uid) + ' — ignored'
            assert ignored in settings, settings
            assert 'configured' not in settings.lower(), settings

    if scenario == 'summary-root-headings':
        def summary_writers_are_visible():
            read_terminal(0.1)
            return summary_row_is_unobscured(screen(), first[1].name, second[1].name)
        wait_for(summary_writers_are_visible, 'summary does not display both fixture writers')
        rendered = screen()
        summary = summary_pane(rendered)
        heading = '[' + pwd.getpwuid(capture.stat().st_uid).pw_name + '] ~/' + work.name
        assert sum(heading in line for line in summary) == 1, rendered
        assert not any('[' + str(other_uid) + ']' in line for line in summary), rendered
        for writer in (first, second):
            assert sum(writer[1].name in line for line in summary) == 1, rendered
    if scenario in ('root-duplicate', 'fallback-root-duplicate', 'fallback-selected-unknown'):
        assert 'probe-unused-' + root.name not in rendered, 'unused proof supplies command\n' + rendered
        assert 'unused-directory-' + root.name not in rendered, 'unused proof supplies directory\n' + rendered
        settings = settings_screen()
        owner = pwd.getpwuid(other_capture.stat().st_uid).pw_name
        ignored = str(other_capture) + ': owned by ' + owner + ', not by ' + str(other_uid) + ' — ignored'
        assert ignored in settings, settings
        assert 'unused directory' not in settings, settings
        publication = carrier[2] if scenario in registration_fallback_scenarios else first[2]
        assert publication.exists() and (other_capture / 'state/pids' / publication.name).exists()
        diagnostics = ' '.join(settings.replace('│', ' ').split())
        assert ': ' + publication.name + ')' in diagnostics, 'selected basename differs from file\n' + settings
        assert '(' + publication.name + '; ' not in diagnostics, 'ignored account supplies competing proof\n' + settings
    if scenario in ('locale', 'forged'):
        assert first[1].name in rendered, rendered
        assert any(first[1].name in line and 'blocked' in line
                   for line in rendered.splitlines()), rendered
        assert first[2].exists() and first[4].exists(), 'live record is removed by reader'
    if scenario in ('nested', 'excluded', 'exec-nested', 'exec-excluded'):
        markers = ['probe-nested-' + root.name + '-' + command for command in ('check', 'test')]
        rendered = wait_for_fixture_pane((first[1].name, *markers))
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
except Exception:
    if scenario.startswith('cpu-cache-'):
        try:
            output = (root / 'cpu-server/output').read_text(errors='replace')
        except OSError as error:
            output = 'cannot read retained server output: ' + str(error)
        print('cache server output before cleanup:\n' + output, file=sys.stderr)
    raise
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
        if cache_server is not None:
            (root / 'cpu-server/release').touch()
            try:
                os.killpg(cache_server, signal.SIGTERM)
            except ProcessLookupError:
                pass
        if scan_reader is not None:
            if scan_reader.poll() is None:
                scan_reader.terminate()
            scan_reader.wait(timeout=5)
        for observations in parent_owned_children:
            (observations / 'release').touch()
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
