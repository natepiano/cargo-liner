"""Observe the live capture log before the stand-in cargo exits."""

import os
from pathlib import Path
import shutil
import sys
import time

from fixture_events import FixtureChanges

account = Path(os.environ['SHIM_TEST_ACCOUNT_DIRECTORY'])
observations = Path(os.environ['SHIM_TEST_OBSERVATIONS'])
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
            shutil.copyfile(log, observations / log.name)
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
