# Empty Git repository seed

The shared test initializer copies these two loose objects. This avoids three
Git subprocesses for an empty fixture. Git still reads the real objects in all
repository probes. Fixtures with files run `git add` and `git commit --amend`.

The initial branch is `main`. The root commit has a fixed author and date. No
fixture relies on its identity or date.

Regenerate the objects with Python 3:

```python
import hashlib
import pathlib
import zlib

objects = [
    (b"tree", b""),
    (b"commit", (
        b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"
        b"author cargo-port-tests <cargo-port-tests@example.com> 1700000000 +0000\n"
        b"committer cargo-port-tests <cargo-port-tests@example.com> 1700000000 +0000\n"
        b"\ninit\n"
    )),
]
for kind, body in objects:
    loose = kind + b" " + str(len(body)).encode() + b"\0" + body
    name = hashlib.sha1(loose).hexdigest() + ".zlib"
    pathlib.Path(name).write_bytes(zlib.compress(loose))
```
