#!/usr/bin/env python3

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path


LINE_RE = re.compile(r"^(?P<ts>\d+)\s+(?P<label>\S+)(?:\s+elapsed_ms=(?P<elapsed>\d+))?(?:\s+(?P<details>.*))?$")
KV_RE = re.compile(r"(\w+)=([^\s]+)")


def default_log_path() -> Path:
    return Path(tempfile.gettempdir()) / "cargo-port-tui-perf.log"


def parse_line(line: str) -> dict[str, object] | None:
    match = LINE_RE.match(line.strip())
    if not match:
        return None
    details = match.group("details") or ""
    data = {
        "ts": int(match.group("ts")),
        "label": match.group("label"),
        "elapsed_ms": int(match.group("elapsed")) if match.group("elapsed") else None,
        "details": details,
        "kv": {key: value for key, value in KV_RE.findall(details)},
    }
    return data


def load_entries(path: Path) -> list[dict[str, object]]:
    return [
        entry
        for line in path.read_text(errors="replace").splitlines()
        if (entry := parse_line(line)) is not None
    ]


def print_section(title: str) -> None:
    print(f"\n== {title} ==")


def top_entries(entries: list[dict[str, object]], label: str, limit: int = 8) -> list[dict[str, object]]:
    rows = [entry for entry in entries if entry["label"] == label and entry["elapsed_ms"] is not None]
    rows.sort(key=lambda entry: int(entry["elapsed_ms"]), reverse=True)
    return rows[:limit]


def entries_before(entries: list[dict[str, object]], cutoff_ts: int, labels: set[str]) -> list[dict[str, object]]:
    return [
        entry
        for entry in entries
        if entry["ts"] <= cutoff_ts and entry["label"] in labels and entry["elapsed_ms"] is not None
    ]


def format_entry(entry: dict[str, object]) -> str:
    elapsed = entry["elapsed_ms"]
    details = entry["details"]
    return f"{elapsed:>6}ms {details}"


def summarize(path: Path) -> int:
    if not path.exists():
        print(f"log not found: {path}")
        return 1

    entries = load_entries(path)
    if not entries:
        print(f"no parseable entries in {path}")
        return 1

    scan_complete_entries = [entry for entry in entries if entry["label"] == "scan_complete"]
    if not scan_complete_entries:
        print("no scan_complete entry found")
        return 1

    scan_complete = scan_complete_entries[-1]
    scan_complete_ts = int(scan_complete["ts"])

    print(f"log: {path}")
    print_section("Startup")
    discover = [entry for entry in entries if entry["label"] == "phase1_discover_total"]
    if discover:
        print(format_entry(discover[-1]))
    print(format_entry(scan_complete))

    print_section("Startup Phase Plan")
    plans = [entry for entry in entries if entry["label"] == "startup_phase_plan"]
    if not plans:
        print("  none")
    else:
        print(f"  {plans[-1]['details']}")

    print_section("Startup Phase Completion")
    completions = [entry for entry in entries if entry["label"] == "startup_phase_complete"]
    if not completions:
        print("  none")
    else:
        for entry in completions:
            print(f"  {entry['details']}")

    print_section("Startup Git Hotspots")
    startup_labels = {"phase1_cached_git_info", "phase1_local_work", "git_info_detect_call"}
    startup = entries_before(entries, scan_complete_ts, startup_labels)
    by_label: dict[str, list[dict[str, object]]] = {}
    for entry in startup:
        by_label.setdefault(str(entry["label"]), []).append(entry)
    for label in ("phase1_cached_git_info", "phase1_local_work", "git_info_detect_call"):
        print(f"-- {label}")
        rows = sorted(by_label.get(label, []), key=lambda entry: int(entry["elapsed_ms"]), reverse=True)[:8]
        if not rows:
            print("   none")
            continue
        for entry in rows:
            print(f"   {format_entry(entry)}")

    print_section("Deferred After Scan")
    deferred = top_entries(
        [entry for entry in entries if int(entry["ts"]) >= scan_complete_ts],
        "git_first_commit_fetch",
        limit=10,
    )
    if not deferred:
        print("  none")
    else:
        for entry in deferred:
            print(f"  {format_entry(entry)}")

    print_section("Slow UI Frames")
    slow_frames = top_entries(entries, "slow_frame", limit=8)
    if not slow_frames:
        print("  none")
    else:
        for entry in slow_frames:
            print(f"  {format_entry(entry)}")

    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Summarize cargo-port perf log into actionable hotspots.")
    parser.add_argument("path", nargs="?", type=Path, default=default_log_path())
    args = parser.parse_args()
    return summarize(args.path)


if __name__ == "__main__":
    raise SystemExit(main())
