"""Exercise the reader cache with private files and process owners."""
import fcntl
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from setup_reader import native_source_identity, publish_reader, remove_retired_native_fixtures


def retired_owners():
    """Retain the live run and remove the dead run at the next publication."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / 'source'
        source.write_bytes(b'first reader')
        source.chmod(0o700)
        cache = root / 'cache'
        setup = Path(__file__).with_name('setup_reader.py')
        live = Path(subprocess.check_output(
            [sys.executable, '-S', str(setup), 'stage', str(source), str(cache)], text=True).strip())
        owner = subprocess.run(
            [sys.executable, '-S', __file__, 'retiring-owner', str(source), str(cache)],
            check=True, capture_output=True, text=True)
        dead = Path(owner.stdout.strip())
        temporary = dead.with_name(dead.name + '.tmp-interrupted')
        temporary.write_bytes(b'incomplete copy')
        live_temporary = live.with_name(live.name + '.tmp-publishing')
        live_temporary.write_bytes(b'live publication')
        (cache / 'bf738597c041509e-53026712').write_bytes(b'unowned earlier reader')
        subprocess.run([sys.executable, '-S', str(setup), 'stage', str(source), str(cache)],
                       check=True, stdout=subprocess.DEVNULL)
        assert sorted(path.name for path in cache.iterdir()) == sorted((live.name, live_temporary.name))
        assert live.read_bytes() == b'first reader'
        assert live_temporary.read_bytes() == b'live publication'


def changed_source():
    """Publish the retained bytes after the real source path changes."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / 'source'
        source.write_bytes(b'first reader')
        content = source.read_bytes()
        source.write_bytes(b'changed reader')
        published = publish_reader(content, 0o700, root / 'cache')
        assert published.name == f'{os.getppid()}-{hashlib.sha256(content).hexdigest()}'
        assert published.read_bytes() == b'first reader'
        assert source.read_bytes() == b'changed reader'


def native_retirement():
    """Retire invalid entries and retain sources with readers or current identities."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()
        environment = dict(os.environ, HOME=str(root), XDG_CACHE_HOME=str(root / 'cache'))
        parent = root / ('Library/Caches' if sys.platform == 'darwin' else 'cache')
        cache = parent / 'cargo-tile/test-readers/native-fixtures'
        cache.mkdir(parents=True)
        setup = Path(__file__).with_name('setup_reader.py')
        retained = set()
        held = []
        removed = []
        for label, state, shared in (
                ('publisher', 'live', False),
                ('other-source', 'live', False),
                ('stale', 'stale', False),
                ('legacy', 'legacy', False),
                ('held-stale', 'stale', True),
                ('held-legacy', 'legacy', True),
                ('unreadable-source', 'unreadable', False)):
            folder = root / label
            folder.mkdir()
            source = folder / 'source'
            source.write_bytes(label.encode())
            identity = native_source_identity(source)
            fixture = cache / hashlib.sha256(identity).hexdigest()
            fixture.write_bytes(label.encode())
            lock = fixture.with_suffix('.lock')
            lock.touch()
            record = fixture.with_suffix('.source')
            if state != 'legacy':
                record.write_bytes(identity)
            if shared:
                guard = lock.open('r+b')
                fcntl.flock(guard, fcntl.LOCK_SH)
                held.append((guard, fixture))
            if state == 'stale':
                source.write_bytes(b'changed source')
            elif state == 'unreadable':
                source.unlink()
                folder.rmdir()
                folder.write_bytes(b'a file now replaces the source parent')
            if state == 'live' or shared:
                retained.update((fixture.name, lock.name))
                if state != 'legacy':
                    retained.add(record.name)
            else:
                removed.append(fixture)
            if label == 'publisher':
                publisher = source
                published = fixture
        orphan = cache / ('0' * 64 + '.lock')
        orphan.touch()
        try:
            for release in (False, True):
                if release:
                    for guard, fixture in held:
                        guard.close()
                        retained.remove(fixture.name)
                        retained.remove(fixture.with_suffix('.lock').name)
                        retained.discard(fixture.with_suffix('.source').name)
                        removed.append(fixture)
                result = subprocess.run(
                    [sys.executable, '-S', str(setup), 'native-fixture', str(publisher)],
                    env=environment, check=True, capture_output=True, text=True)
                assert Path(result.stdout.strip()) == published
                assert {path.name for path in cache.iterdir()} == retained
                assert published.read_bytes() == b'publisher'
                for fixture in removed:
                    assert not fixture.exists(), fixture
                    assert not fixture.with_suffix('.lock').exists(), fixture
                    assert not fixture.with_suffix('.source').exists(), fixture
                assert not orphan.exists()
                for lock in cache.glob('*.lock'):
                    assert lock.with_suffix('').is_file(), lock
        finally:
            for guard, fixture in held:
                guard.close()


if __name__ == '__main__':
    case = sys.argv[1]
    if case == 'retiring-owner':
        setup = Path(__file__).with_name('setup_reader.py')
        subprocess.run([sys.executable, '-S', str(setup), 'stage', *sys.argv[2:]], check=True)
    elif case == 'retired-owners':
        retired_owners()
    elif case == 'retire-native-cache':
        cache = Path(sys.argv[2])
        remove_retired_native_fixtures(cache, cache / 'retained')
    elif case == 'native-retirement':
        native_retirement()
    elif case == 'changed-source':
        changed_source()
    else:
        raise ValueError('The reader staging case is invalid.')
