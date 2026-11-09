#!/usr/bin/env -S python3 -S
"""Record cargo observations and read the live capture log in one process."""

import os
from pathlib import Path
import shutil
import sys
import time

account = Path(os.environ['SHIM_TEST_ACCOUNT_DIRECTORY'])
observations = Path(os.environ['SHIM_TEST_OBSERVATIONS'])

def write_fields(name, values):
    """Retain Unix argument and environment bytes with NUL framing."""
    (observations / name).write_bytes(b''.join(os.fsencode(value) + b'\0' for value in values))


def observe_birth():
    """Read the live shim independently from the native publisher."""
    shim_pid = (observations / 'shim-pid').read_text()
    if os.environ['SHIM_TEST_NATIVE_PLATFORM'] == 'linux':
        (observations / 'shim-stat').write_bytes(Path(f'/proc/{shim_pid}/stat').read_bytes())
        (observations / 'boot').write_bytes(Path('/proc/sys/kernel/random/boot_id').read_bytes())
        return
    import subprocess

    environment = dict(os.environ, LC_ALL='C', TZ='UTC0')
    started = subprocess.check_output(
        [os.environ['SHIM_TEST_REAL_PS'], '-o', 'lstart=', '-p', shim_pid],
        env=environment).rstrip()
    birth = subprocess.check_output(
        [os.environ['SHIM_TEST_REAL_DATE'], '-j', '-f', '%a %b %e %H:%M:%S %Y',
         os.fsdecode(started), '+%s'], env=environment)
    (observations / 'birth').write_bytes(birth)
    (observations / 'boot').write_bytes(subprocess.check_output(
        [os.environ['SHIM_TEST_REAL_SYSCTL'], '-n', 'kern.bootsessionuuid']))


write_fields('arguments', sys.argv[1:])
mask = os.umask(0)
os.umask(mask)
(observations / 'umask').write_text(f'{mask:04o}\n')
settings = ('CARGOTILE_NESTED', 'CARGO_TERM_PROGRESS_WHEN', 'CARGO_TERM_PROGRESS_WIDTH')
write_fields('capture-settings', (os.environ.get(key, '') for key in settings))
write_fields('environment', (os.environ.get(key, 'unset') for key in settings))
write_fields('locale-settings', (os.environ.get(key, '') for key in ('LC_ALL', 'TZ', 'LANG', 'HOME')))
if os.environ['SHIM_TEST_NATIVE_BIRTH_VERIFICATION'] == 'required':
    observe_birth()
pending = os.environ.get('SHIM_TEST_PENDING_DELETE')
if pending and Path(pending).is_file():
    for name in Path(pending).read_text().splitlines():
        Path(name).unlink(missing_ok=True)
    (observations / 'delayed-delete').write_text('delayed unlink attempted\n')
registrations = account / 'state/pids'
if registrations.is_dir():
    shutil.copytree(registrations, observations / 'pids', symlinks=True)
state = account / 'state'
if state.is_dir() and not (observations.parent / 'repeat').is_file():
    (observations / 'state-entries').write_bytes(
        b''.join(os.fsencode(name) + b'\n' for name in sorted(os.listdir(state))))
os.write(1, b'cargo stdout\n')
os.write(2, b'registration log marker\n')
if not os.environ.get('CARGOTILE_NESTED'):
    sys.exit(37)

# The copied cargo uses the existing event observer without another process.
sys.path.insert(0, str(Path(os.environ['SHIM_TEST_LOG_OBSERVER']).parent))
from fixture_events import FixtureChanges

timeout_file = Path(os.environ['SHIM_TEST_LOG_TIMEOUT_FILE'])
timeout = float(timeout_file.read_text()) if timeout_file.exists() else 2.0
deadline = time.monotonic() + timeout
while time.monotonic() < deadline:
    # Subscribe before reading. A later log write remains in the event queue.
    changes = FixtureChanges(account)
    try:
        for log in account.glob('run-*.log'):
            if b'registration log marker\n' not in log.read_bytes():
                continue
            shutil.copy2(log, observations / log.name)
            pending = os.environ.get('SHIM_TEST_PENDING_DELETE')
            if pending and not Path(pending).exists():
                for registration in (account / 'state/pids').iterdir():
                    if registration.name.endswith('.tmp'):
                        continue
                    _ = Path(pending).write_text(str(registration) + '\n' + str(log) + '\n')
            sys.exit(37)
        changes.wait(max(0.0, deadline - time.monotonic()))
    finally:
        changes.close()
sys.stderr.write('capture never wrote the cargo stderr marker\n')
sys.exit(98)
