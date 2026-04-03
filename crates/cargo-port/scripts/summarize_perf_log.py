#!/usr/bin/env python3

from __future__ import annotations

import argparse
import tempfile
import re
from pathlib import Path


LINE_RE = re.compile(r"^(?P<ts>\d+)\s+(?P<label>\S+)(?:\s+elapsed_ms=(?P<elapsed>\d+))?(?:\s+(?P<details>.*))?$")
KV_RE = re.compile(r"(\w+)=([^\s]+)")
COMPARE_LABELS = (
    "phase1_discover_total",
    "scan_complete",
    "startup_complete",
    "watcher_git_queue_wait",
    "watcher_git_refresh",
    "watcher_git_info_detect",
    "git_info_detect_call",
    "watcher_disk_queue_wait",
    "watcher_disk_usage",
    "poll_background",
    "slow_frame",
    "git_first_commit_fetch",
)


def default_log_path() -> Path:
    return Path(tempfile.gettempdir()) / "cargo-port-tui-perf.log"


def previous_log_path() -> Path:
    return Path(tempfile.gettempdir()) / "cargo-port-tui-perf.prev.log"


def parse_line(line: str) -> dict[str, object] | None:
    match = LINE_RE.match(line.strip())
    if not match:
        return None
    details = match.group("details") or ""
    return {
        "ts": int(match.group("ts")),
        "label": match.group("label"),
        "elapsed_ms": int(match.group("elapsed")) if match.group("elapsed") else None,
        "details": details,
        "kv": {key: value for key, value in KV_RE.findall(details)},
    }


def load_entries(path: Path) -> list[dict[str, object]]:
    return [
        entry
        for line in path.read_text(errors="replace").splitlines()
        if (entry := parse_line(line)) is not None
    ]


def print_section(title: str) -> None:
    print(f"\n== {title} ==")


def format_entry(entry: dict[str, object]) -> str:
    elapsed = entry["elapsed_ms"]
    details = entry["details"]
    return f"{elapsed:>6}ms {details}"


def top_entries(entries: list[dict[str, object]], label: str, limit: int = 8) -> list[dict[str, object]]:
    rows = [entry for entry in entries if entry["label"] == label and entry["elapsed_ms"] is not None]
    rows.sort(key=lambda entry: int(entry["elapsed_ms"]), reverse=True)
    return rows[:limit]


def last_entry(entries: list[dict[str, object]], label: str) -> dict[str, object] | None:
    rows = [entry for entry in entries if entry["label"] == label]
    return rows[-1] if rows else None


def startup_boundary_ts(entries: list[dict[str, object]]) -> int | None:
    startup_complete = last_entry(entries, "startup_complete")
    if startup_complete is not None:
        return int(startup_complete["ts"])

    completions = [entry for entry in entries if entry["label"] == "startup_phase_complete"]
    if completions:
        return max(int(entry["ts"]) for entry in completions)

    scan_complete = last_entry(entries, "scan_complete")
    return int(scan_complete["ts"]) if scan_complete is not None else None


def duration_entries(entries: list[dict[str, object]]) -> list[dict[str, object]]:
    return [entry for entry in entries if entry["elapsed_ms"] is not None]


def entries_in_window(
    entries: list[dict[str, object]],
    start_ts: int | None = None,
    end_ts: int | None = None,
) -> list[dict[str, object]]:
    rows = duration_entries(entries)
    if start_ts is not None:
        rows = [entry for entry in rows if int(entry["ts"]) >= start_ts]
    if end_ts is not None:
        rows = [entry for entry in rows if int(entry["ts"]) <= end_ts]
    return rows


def aggregate(entries: list[dict[str, object]]) -> dict[str, dict[str, int]]:
    totals: dict[str, dict[str, int]] = {}
    for entry in entries:
        label = str(entry["label"])
        elapsed = int(entry["elapsed_ms"])
        row = totals.setdefault(label, {"count": 0, "total_ms": 0, "max_ms": 0})
        row["count"] += 1
        row["total_ms"] += elapsed
        row["max_ms"] = max(row["max_ms"], elapsed)
    return totals


def print_top_labels(entries: list[dict[str, object]], limit: int = 10) -> None:
    totals = aggregate(entries)
    if not totals:
        print("  none")
        return
    rows = sorted(totals.items(), key=lambda item: item[1]["total_ms"], reverse=True)[:limit]
    for label, data in rows:
        avg_ms = data["total_ms"] // data["count"]
        print(
            f"  {label} total_ms={data['total_ms']} count={data['count']} avg_ms={avg_ms} max_ms={data['max_ms']}"
        )


def print_compare(current_entries: list[dict[str, object]], previous_entries: list[dict[str, object]]) -> None:
    current = aggregate(duration_entries(current_entries))
    previous = aggregate(duration_entries(previous_entries))

    print_section("Current vs Previous")
    for label in COMPARE_LABELS:
        current_row = current.get(label)
        previous_row = previous.get(label)
        if current_row is None and previous_row is None:
            continue
        current_total = current_row["total_ms"] if current_row else 0
        previous_total = previous_row["total_ms"] if previous_row else 0
        current_count = current_row["count"] if current_row else 0
        previous_count = previous_row["count"] if previous_row else 0
        delta_total = current_total - previous_total
        delta_count = current_count - previous_count
        print(
            f"  {label} current_total_ms={current_total} previous_total_ms={previous_total} "
            f"delta_total_ms={delta_total:+} current_count={current_count} previous_count={previous_count} "
            f"delta_count={delta_count:+}"
        )


def summarize_single(path: Path, previous_path_override: Path | None) -> int:
    if not path.exists():
        print(f"log not found: {path}")
        return 1

    entries = load_entries(path)
    if not entries:
        print(f"no parseable entries in {path}")
        return 1

    scan_complete = last_entry(entries, "scan_complete")
    if scan_complete is None:
        print("no scan_complete entry found")
        return 1
    startup_ts = startup_boundary_ts(entries)

    print(f"log: {path}")
    print_section("Startup")
    discover = last_entry(entries, "phase1_discover_total")
    if discover is not None:
        print(format_entry(discover))
    print(format_entry(scan_complete))
    startup_complete = last_entry(entries, "startup_complete")
    if startup_complete is not None:
        print(format_entry(startup_complete))

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

    print_section("Startup Hotspots")
    startup_entries = entries_in_window(entries, end_ts=startup_ts)
    print_top_labels(startup_entries)

    print_section("Steady State Hotspots")
    steady_entries = entries_in_window(entries, start_ts=startup_ts + 1 if startup_ts is not None else None)
    print_top_labels(steady_entries)

    print_section("Deferred After Scan")
    deferred = top_entries(
        [entry for entry in entries if int(entry["ts"]) >= int(scan_complete["ts"])],
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

    previous_path = previous_path_override or previous_log_path()
    if previous_path.exists():
        previous_entries = load_entries(previous_path)
        if previous_entries:
            print(f"\nprevious_log: {previous_path}")
            print_compare(entries, previous_entries)

    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Summarize cargo-port perf logs into startup and steady-state hotspots."
    )
    parser.add_argument("path", nargs="?", type=Path, default=default_log_path())
    parser.add_argument(
        "--previous",
        type=Path,
        default=None,
        help="Optional previous log path. Defaults to cargo-port-tui-perf.prev.log in the temp dir.",
    )
    args = parser.parse_args()
    return summarize_single(args.path, args.previous)


if __name__ == "__main__":
    raise SystemExit(main())
