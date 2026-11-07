"""Wait for changes to files in an owned test fixture."""

import ctypes
import os
from pathlib import Path
import select
import stat
import sys

class FixtureChanges:
    """Wake a file predicate when a fixture file changes."""

    def __init__(self, root: Path) -> None:
        self.descriptors: list[int] = []
        if sys.platform == 'darwin':
            self.queue = select.kqueue()
        else:
            self.libc = ctypes.CDLL(None, use_errno=True)
            self.libc.inotify_init1.argtypes = (ctypes.c_int,)
            self.libc.inotify_init1.restype = ctypes.c_int
            self.libc.inotify_add_watch.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32)
            self.libc.inotify_add_watch.restype = ctypes.c_int
            self.descriptor = self.libc.inotify_init1(os.O_CLOEXEC | os.O_NONBLOCK)
            if self.descriptor < 0:
                raise OSError(ctypes.get_errno(), 'open fixture change events')
        try:
            for directory, _names, files in os.walk(root):
                paths = [Path(directory)]
                if sys.platform == 'darwin':
                    paths.extend(Path(directory) / name for name in files)
                for path in paths:
                    self.watch(path)
        except BaseException:
            self.close()
            raise

    def watch(self, path: Path) -> None:
        try:
            if sys.platform == 'darwin':
                # Opening a FIFO would block its producer. Watch its directory instead.
                if not stat.S_ISREG(path.stat().st_mode) and not path.is_dir():
                    return
                descriptor = os.open(path, os.O_EVTONLY)
                self.descriptors.append(descriptor)
                event = select.kevent(descriptor, filter=select.KQ_FILTER_VNODE,
                                      flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                      fflags=select.KQ_NOTE_WRITE | select.KQ_NOTE_RENAME
                                      | select.KQ_NOTE_DELETE | select.KQ_NOTE_ATTRIB)
                _ = self.queue.control([event], 0, 0)
            elif self.libc.inotify_add_watch(self.descriptor, os.fsencode(path), 0x3c6) < 0:
                raise OSError(ctypes.get_errno(), 'watch fixture directory', path)
        except FileNotFoundError:
            pass

    def wait(self, timeout: float) -> None:
        if sys.platform == 'darwin':
            _ = self.queue.control([], 64, timeout)
        elif select.select([self.descriptor], [], [], timeout)[0]:
            _ = os.read(self.descriptor, 65536)

    def close(self) -> None:
        if sys.platform == 'darwin':
            self.queue.close()
            for descriptor in self.descriptors:
                os.close(descriptor)
        else:
            os.close(self.descriptor)
