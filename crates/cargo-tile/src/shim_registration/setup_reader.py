#!/usr/bin/env -S python3 -S
"""Publish and load the shared reader before the tests start."""
import hashlib
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile


def cache_directory():
    """Select the cache directory for `setup_reader.py` and its Rust callers."""
    if sys.platform == 'darwin':
        parent = Path.home() / 'Library/Caches'
    else:
        parent = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache'))
    return parent / 'cargo-tile/test-readers'


def process_is_alive(owner):
    """Keep files while their nextest process or test executable is alive."""
    try:
        os.kill(owner, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        pass
    return True


def remove_retired_readers(cache):
    """Remove copies and temporary files that have no live run owner."""
    for entry in cache.iterdir():
        owned = re.fullmatch(r'([1-9][0-9]*)-[0-9a-f]{64}(?:\.tmp-.+)?', entry.name)
        if owned:
            if process_is_alive(int(owned[1])):
                continue
        elif not re.fullmatch(r'[0-9a-f]{64}|[0-9a-f]{16}-[0-9]+|tmp[^/]+', entry.name):
            continue
        # The earlier `stage` format had no run owner and cannot be reused.
        entry.unlink(missing_ok=True)


def publish_reader(content, mode, cache):
    """Hash and publish the same bytes for the calling run."""
    owner = os.getppid()
    name = f'{owner}-{hashlib.sha256(content).hexdigest()}'
    cache.mkdir(parents=True, exist_ok=True)
    remove_retired_readers(cache)
    destination = cache / name
    if destination.is_file() and destination.read_bytes() == content:
        return destination
    descriptor, temporary = tempfile.mkstemp(prefix=name + '.tmp-', dir=cache)
    try:
        with os.fdopen(descriptor, 'wb') as output:
            output.write(content)
            os.fchmod(output.fileno(), mode)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return destination


def stage(source, cache=None):
    """Read the source once, then publish those bytes through `publish_reader`."""
    content = source.read_bytes()
    return publish_reader(content, source.stat().st_mode,
                          cache_directory() if cache is None else cache)


def main():
    """Select publication, listing, or setup from the command arguments."""
    mode = sys.argv[1]
    if mode == 'stage':
        print(stage(Path(sys.argv[2]), Path(sys.argv[3]) if len(sys.argv) == 4 else None))
        return
    target = Path(os.environ.get('CARGO_TARGET_DIR', 'target'))
    if not target.is_absolute():
        target = Path(os.environ['NEXTEST_WORKSPACE_ROOT']) / target
    publication = target / 'tmp/cargo-tile-reader-setup' / os.environ['NEXTEST_RUN_ID']
    if mode == 'list':
        publication.parent.mkdir(parents=True, exist_ok=True)
        publication.write_text(str(Path(sys.argv[2]).resolve()))
        os.execv(sys.argv[2], sys.argv[2:])
    if mode != 'setup':
        raise ValueError('The reader setup mode is invalid.')
    source = Path(publication.read_text())
    reader = stage(source)
    subprocess.run([str(reader), '--list'], check=True, stdout=subprocess.DEVNULL)
    with open(os.environ['NEXTEST_ENV'], 'a') as environment:
        environment.write(f'CARGO_TILE_TEST_READER_EXECUTABLE={reader}\n')
    publication.unlink()
    print(f'Loaded the shared reader: {reader}', file=sys.stderr)


if __name__ == '__main__':
    main()
