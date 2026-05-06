#!/usr/bin/env python3
"""Count methods on `impl App` across src/tui/app/**.

Outputs total count plus per-file breakdown. Used after each phase of the
reduce-app-footprint plan to confirm the count-delta matches the phase's
predicted method-removal count.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
APP_DIR = REPO_ROOT / "src" / "tui" / "app"

IMPL_APP_RE = re.compile(r"^\s*impl(?:<[^>]*>)?\s+App\b(?!\w)")
IMPL_OTHER_RE = re.compile(r"^\s*impl\b")
FN_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[<(]")


def count_in_file(path: Path) -> list[tuple[int, str]]:
    """Return list of (line_no, method_name) for methods inside `impl App` blocks."""
    methods: list[tuple[int, str]] = []
    in_impl_app = False
    brace_depth = 0
    with path.open(encoding="utf-8") as fh:
        for lineno, raw in enumerate(fh, start=1):
            stripped = raw.rstrip("\n")
            if not in_impl_app:
                if IMPL_APP_RE.match(stripped):
                    in_impl_app = True
                    brace_depth = stripped.count("{") - stripped.count("}")
                continue
            brace_depth += stripped.count("{") - stripped.count("}")
            if brace_depth <= 0:
                in_impl_app = False
                continue
            m = FN_RE.match(stripped)
            if m:
                methods.append((lineno, m.group(1)))
    return methods


def main() -> int:
    if not APP_DIR.is_dir():
        print(f"error: {APP_DIR} not found", file=sys.stderr)
        return 1

    files = sorted(APP_DIR.rglob("*.rs"))
    per_file: list[tuple[Path, list[tuple[int, str]]]] = []
    total = 0
    for f in files:
        methods = count_in_file(f)
        if methods:
            per_file.append((f, methods))
            total += len(methods)

    print(f"Total App methods: {total}")
    print()
    print("Per-file breakdown:")
    for path, methods in per_file:
        rel = path.relative_to(REPO_ROOT)
        print(f"  {len(methods):>4}  {rel}")

    if "--list" in sys.argv:
        print()
        print("Method list:")
        for path, methods in per_file:
            rel = path.relative_to(REPO_ROOT)
            for lineno, name in methods:
                print(f"  {rel}:{lineno}  {name}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
