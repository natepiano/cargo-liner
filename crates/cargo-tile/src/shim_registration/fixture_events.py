"""Wait for changes to files in an owned test fixture."""

import ctypes
import errno
import os
from pathlib import Path
import select
import stat
import struct
import sys


class FixtureSubscription:
    """Bind one fixture path to its current native watch."""

    def __init__(self, device: int, inode: int, descriptor: int) -> None:
        self.device = device
        self.inode = inode
        self.descriptor = descriptor


class FixtureChanges:
    """Wake a file predicate when a fixture file changes."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.subscriptions: dict[Path, FixtureSubscription] = {}
        self.paths: dict[int, Path] = {}
        if sys.platform == 'darwin':
            self.queue = select.kqueue()
        else:
            self.libc = ctypes.CDLL(None, use_errno=True)
            self.libc.inotify_init1.argtypes = (ctypes.c_int,)
            self.libc.inotify_init1.restype = ctypes.c_int
            self.libc.inotify_add_watch.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32)
            self.libc.inotify_add_watch.restype = ctypes.c_int
            self.libc.inotify_rm_watch.argtypes = (ctypes.c_int, ctypes.c_int)
            self.libc.inotify_rm_watch.restype = ctypes.c_int
            self.descriptor = self.libc.inotify_init1(os.O_CLOEXEC | os.O_NONBLOCK)
            if self.descriptor < 0:
                raise OSError(ctypes.get_errno(), 'open fixture change events')
        try:
            self.refresh()
        except BaseException:
            self.close()
            raise

    def refresh(self) -> None:
        # FixtureChanges registers new paths before wait_for_files reads its predicate.
        for directory, _names, files in os.walk(self.root):
            paths = [Path(directory)]
            if sys.platform == 'darwin':
                paths.extend(Path(directory) / name for name in files)
            for path in paths:
                self.watch(path)

    def watch(self, path: Path) -> None:
        try:
            metadata = path.stat()
            previous = self.subscriptions.get(path)
            if previous is not None:
                if (previous.device, previous.inode) == (metadata.st_dev, metadata.st_ino):
                    return
                self.retire(path)
            if sys.platform == 'darwin':
                # FixtureChanges watches a FIFO's directory to avoid blocking its producer.
                if not stat.S_ISREG(metadata.st_mode) and not stat.S_ISDIR(metadata.st_mode):
                    return
                descriptor = os.open(path, os.O_EVTONLY)
                event = select.kevent(descriptor, filter=select.KQ_FILTER_VNODE,
                                      flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                      fflags=select.KQ_NOTE_WRITE | select.KQ_NOTE_RENAME
                                      | select.KQ_NOTE_DELETE | select.KQ_NOTE_ATTRIB)
                try:
                    _ = self.queue.control([event], 0, 0)
                except BaseException:
                    os.close(descriptor)
                    raise
            else:
                # inotify also reports deletion and movement of the watched directory itself.
                descriptor = self.libc.inotify_add_watch(self.descriptor, os.fsencode(path), 0xfc6)
                if descriptor < 0:
                    raise OSError(ctypes.get_errno(), 'watch fixture directory', path)
            self.subscriptions[path] = FixtureSubscription(metadata.st_dev, metadata.st_ino, descriptor)
            self.paths[descriptor] = path
        except FileNotFoundError:
            self.retire(path)

    def retire(self, path: Path) -> None:
        subscription = self.subscriptions.pop(path, None)
        if subscription is None:
            return
        del self.paths[subscription.descriptor]
        if sys.platform == 'darwin':
            os.close(subscription.descriptor)
        elif self.libc.inotify_rm_watch(self.descriptor, subscription.descriptor) < 0:
            error = ctypes.get_errno()
            if error != errno.EINVAL:
                raise OSError(error, 'remove fixture directory watch', path)

    def wait(self, timeout: float) -> None:
        if sys.platform == 'darwin':
            for event in self.queue.control([], 64, timeout):
                if event.fflags & (select.KQ_NOTE_DELETE | select.KQ_NOTE_RENAME):
                    path = self.paths.get(event.ident)
                    if path is not None:
                        self.retire(path)
        elif select.select([self.descriptor], [], [], timeout)[0]:
            events = os.read(self.descriptor, 65536)
            offset = 0
            while offset < len(events):
                descriptor, mask, length = struct.unpack_from('iI4xI', events, offset)
                name = events[offset + 16:offset + 16 + length].split(b'\0', 1)[0]
                offset += 16 + length
                path = self.paths.get(descriptor)
                if path is None:
                    continue
                if mask & 0x8c00:  # IN_IGNORED, IN_DELETE_SELF or IN_MOVE_SELF.
                    self.retire(path)
                elif mask & 0x240 and name:  # IN_DELETE or IN_MOVED_FROM.
                    self.retire(path / os.fsdecode(name))

    def close(self) -> None:
        if sys.platform == 'darwin':
            self.queue.close()
            for subscription in self.subscriptions.values():
                os.close(subscription.descriptor)
        else:
            os.close(self.descriptor)
