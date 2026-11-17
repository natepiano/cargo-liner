#!/usr/bin/env -S python3 -S
"""Publish and load the shared reader before the tests start."""
import fcntl
import hashlib
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def cache_directory() -> Path:
    """Select the cache directory for `setup_reader.py` and its Rust callers."""
    if sys.platform == 'darwin':
        parent = Path.home() / 'Library/Caches'
    else:
        parent = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache'))
    return parent / 'cargo-tile/test-readers'


def process_is_alive(owner: int) -> bool:
    """Keep files while their nextest process or test executable is alive."""
    try:
        os.kill(owner, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        pass
    return True


def remove_retired_readers(cache: Path) -> None:
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


def publish_reader(content: bytes, mode: int, cache: Path) -> Path:
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
            _ = output.write(content)
            os.fchmod(output.fileno(), mode)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return destination


def stage(source: Path, cache: Path | None = None) -> Path:
    """Read the source once, then publish those bytes through `publish_reader`."""
    content = source.read_bytes()
    return publish_reader(content, source.stat().st_mode,
                          cache_directory() if cache is None else cache)


def native_source_identity(source: Path) -> bytes:
    """Record the source path, file identity, and native strip options."""
    metadata = source.stat()
    options = ('-u', '-r') if sys.platform == 'darwin' else ('--strip-all',)
    identity = (metadata.st_dev, metadata.st_ino, metadata.st_size,
                metadata.st_mtime_ns, metadata.st_ctime_ns, options)
    return os.fsencode(source.resolve()) + b'\0' + repr(identity).encode()


def remove_retired_native_fixtures(cache: Path, retained: Path) -> None:
    """Remove stale native fixtures only while their publication locks are free."""
    names = {entry.name.split('.', 1)[0] for entry in cache.iterdir()
             if re.fullmatch(r'[0-9a-f]{64}(?:\.lock|\.source)?', entry.name)}
    for name in names:
        destination = cache / name
        if destination == retained:
            continue
        lock = cache / (name + '.lock')
        with lock.open('a+b') as guard:
            try:
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                continue
            try:
                if os.fstat(guard.fileno()).st_ino != lock.stat().st_ino:
                    continue
            except FileNotFoundError:
                continue
            record = cache / (name + '.source')
            try:
                identity = record.read_bytes()
                source = Path(os.fsdecode(identity.split(b'\0', 1)[0]))
                if destination.is_file() and native_source_identity(source) == identity:
                    continue
            except OSError:
                pass
            destination.unlink(missing_ok=True)
            record.unlink(missing_ok=True)
            lock.unlink(missing_ok=True)


def native_fixture(source: Path) -> Path:
    """Publish a native fixture and retire fixtures whose source identity changed."""
    if sys.platform not in ('linux', 'darwin'):
        return source
    options = ('-u', '-r') if sys.platform == 'darwin' else ('--strip-all',)
    cache = cache_directory() / 'native-fixtures'
    cache.mkdir(parents=True, exist_ok=True)
    while True:
        identity = native_source_identity(source)
        name = hashlib.sha256(identity).hexdigest()
        destination = cache / name
        lock = cache / (name + '.lock')
        with lock.open('a+b') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX)
            try:
                if os.fstat(guard.fileno()).st_ino != lock.stat().st_ino:
                    continue
            except FileNotFoundError:
                continue
            if not destination.is_file():
                with tempfile.TemporaryDirectory(prefix='prepare-', dir=cache) as temporary:
                    executable = Path(temporary) / 'cargo-tile'
                    _ = subprocess.run(['strip', *options, '-o', str(executable), str(source)],
                                       check=True)
                    if sys.platform == 'darwin':
                        _ = subprocess.run(['codesign', '--force', '--sign', '-', str(executable)],
                                           check=True)
                    executable.chmod(0o755)
                    if native_source_identity(source) != identity:
                        continue
                    _ = (cache / (name + '.source')).write_bytes(identity)
                    os.link(executable, destination)
            remove_retired_native_fixtures(cache, destination)
            return destination


def prepare_named_shell() -> Path:
    """Load the signed shell inode that `copy_named_shell` links into each fixture."""
    with tempfile.TemporaryDirectory(prefix='cargo-tile-named-shell-') as temporary:
        shell = Path(temporary) / 'cargo-tile-real'
        _ = shell.write_bytes(Path('/bin/bash').read_bytes())
        shell.chmod(0o755)
        _ = subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(shell)],
                           check=True, capture_output=True)
        prepared = stage(shell)
    # macOS checks each new executable inode on its first launch; pay that cost here once.
    _ = subprocess.run([str(prepared), '-c', ':'], check=True)
    return prepared


def preload_product_fixture(reader: Path, publication: Path) -> None:
    """Prepare and launch the product fixture once, before any test needs it."""
    # `reader_child` supplies its compiled `CARGO_BIN_EXE_cargo-tile` path and fixture receipt.
    native_publication = publication.with_name(publication.name + '.native')
    started = time.monotonic()
    preparation_environment = dict(os.environ,
                                   CARGO_TILE_TEST_NATIVE_FIXTURE_SETUP=str(native_publication))
    try:
        _ = subprocess.run([str(reader), '--exact',
                            'shim_registration::reader_scenarios::reader_child'],
                           env=preparation_environment, check=True,
                           stdout=subprocess.DEVNULL)
        product, prepared = native_publication.read_bytes().split(b'\0')
        fixture = os.fsdecode(prepared)
        _ = subprocess.run([fixture, '--version'], check=True, stdout=subprocess.DEVNULL)
    finally:
        native_publication.unlink(missing_ok=True)
    elapsed = time.monotonic() - started
    print(f'Prepared and loaded the product fixture in {elapsed:.3f} s: '
          + f'{os.fsdecode(product)} -> {fixture}', file=sys.stderr)


def main() -> None:
    """Select publication, listing, or setup from the command arguments."""
    mode = sys.argv[1]
    if mode == 'native-fixture':
        print(native_fixture(Path(sys.argv[2])))
        return
    if mode == 'stage':
        print(stage(Path(sys.argv[2]), Path(sys.argv[3]) if len(sys.argv) == 4 else None))
        return
    target = Path(os.environ.get('CARGO_TARGET_DIR', 'target'))
    if not target.is_absolute():
        target = Path(os.environ['NEXTEST_WORKSPACE_ROOT']) / target
    publication = target / 'tmp/cargo-tile-reader-setup' / os.environ['NEXTEST_RUN_ID']
    if mode == 'list':
        publication.parent.mkdir(parents=True, exist_ok=True)
        _ = publication.write_text(str(Path(sys.argv[2]).resolve()))
        os.execv(sys.argv[2], sys.argv[2:])
    if mode != 'setup':
        raise ValueError('The reader setup mode is invalid.')
    source = Path(publication.read_text())
    reader = stage(source)
    _ = subprocess.run([str(reader), '--list'], check=True, stdout=subprocess.DEVNULL)
    preload_product_fixture(reader, publication)
    shell: Path | None = None
    if sys.platform == 'darwin':
        started = time.monotonic()
        shell = prepare_named_shell()
        print(f'Prepared and loaded the named shell in {time.monotonic() - started:.3f} s: '
              + f'{shell}', file=sys.stderr)
    with open(os.environ['NEXTEST_ENV'], 'a') as environment:
        _ = environment.write(f'CARGO_TILE_TEST_READER_EXECUTABLE={reader}\n')
        if shell is not None:
            _ = environment.write(f'CARGO_TILE_TEST_NAMED_SHELL={shell}\n')
    publication.unlink()
    print(f'Loaded the shared reader: {reader}', file=sys.stderr)


if __name__ == '__main__':
    main()
