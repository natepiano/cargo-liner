"""Exercise the reader cache with private files and process owners."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from setup_reader import publish_reader


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


if __name__ == '__main__':
    case = sys.argv[1]
    if case == 'retiring-owner':
        setup = Path(__file__).with_name('setup_reader.py')
        subprocess.run([sys.executable, '-S', str(setup), 'stage', *sys.argv[2:]], check=True)
    elif case == 'retired-owners':
        retired_owners()
    elif case == 'changed-source':
        changed_source()
    else:
        raise ValueError('The reader staging case is invalid.')
