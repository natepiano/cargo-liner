#!/usr/bin/env python3
"""Measure the disk reads of an idle cargo-port.

The tool runs one cargo-port binary in its own systemd user scope, waits for
the end of startup and a settle time, and measures one idle interval. The
scope cgroup counts the reads of the app and of every child process (Git,
cargo). Its memory counters give per-process swap-in evidence. At the end the
tool kills every process in the scope, also on failure and on SIGTERM.

With `--trace`, the app writes its trace log and the tool samples each thread
of the scope. Each sample charges the thread's new read bytes to the file the
thread reads in a syscall, or to a page fault when the thread is not in a
syscall. Acceptance runs omit `--trace`, because trace logging changes how the
app captures Git output.

Usage:
    idle_read_trace.py --binary PATH --out DIR --max-load 8 \
        --load-wait 3600 --startup-limit 300 --settle 60 --duration 600 [--trace]

The caller also gives the run a hard limit from outside, for example with
`timeout`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time
from collections import Counter
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from types import FrameType
from typing import TypedDict

PAGE_BYTES = os.sysconf("SC_PAGE_SIZE")
PERF_LOG_NAME = "cargo-port-tui-perf.log"
STEADY_MARKER = "steady_state_begin"
STOP_LIMIT_S = 10
TRACE_LOG_FILTER = "info,tui_pane::perf=trace"
NORMAL_LOG_FILTER = "info"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
LOG_TIME = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+)Z\s")
FIELD = re.compile(r'(\w+)=("(?:[^"\\]|\\.)*"|\S+)')
# x86-64 syscall numbers that read file data.
READ_SYSCALLS = {0: "read", 17: "pread64", 19: "readv", 295: "preadv", 327: "preadv2"}


class Counters(TypedDict):
    at: str
    load_1m: float
    host_pswpin: int
    host_pgmajfault: int
    scope_read_bytes: int
    scope_pswpin: int
    scope_pgmajfault: int


class Summary(TypedDict):
    binary: str
    trace: bool
    start: Counters
    end: Counters
    interval_s: float
    scope_read_bytes: int
    scope_swapin_bytes: int
    scope_file_fault_count: int
    reads_after_swapin: int
    host_pswpin_bytes: int
    log_events: dict[str, int]
    git_reads_by_op: dict[str, int]
    git_unmeasured_calls: int
    disk_usage_read_bytes: int
    event_paths: dict[str, int]
    sampled_syscall_bytes: dict[str, int]
    sampled_fault_bytes: int
    sampled_fault_count: int


@dataclass
class ThreadSample:
    read_bytes: int
    majflt: int


@dataclass
class Sampler:
    procs_path: Path
    last: dict[tuple[int, int], ThreadSample] = field(default_factory=dict)
    syscall_bytes: Counter[str] = field(default_factory=Counter)
    fault_bytes: int = 0
    fault_count: int = 0

    def sample(self) -> None:
        for pid in read_pids(self.procs_path):
            task_dir = Path(f"/proc/{pid}/task")
            try:
                tids = [int(name) for name in os.listdir(task_dir)]
            except OSError:
                continue
            comm = read_text(Path(f"/proc/{pid}/comm")).strip()
            for tid in tids:
                self.sample_thread(pid, tid, comm)

    def sample_thread(self, pid: int, tid: int, comm: str) -> None:
        base = Path(f"/proc/{pid}/task/{tid}")
        read_bytes = io_read_bytes(base / "io")
        majflt = thread_majflt(base / "stat")
        if read_bytes is None or majflt is None:
            return
        key = (pid, tid)
        previous = self.last.get(key)
        self.last[key] = ThreadSample(read_bytes, majflt)
        if previous is None:
            return
        new_bytes = read_bytes - previous.read_bytes
        new_faults = majflt - previous.majflt
        self.fault_count += max(new_faults, 0)
        if new_bytes <= 0:
            return
        syscall = read_text(base / "syscall").split()
        if syscall and syscall[0].isdigit() and int(syscall[0]) in READ_SYSCALLS:
            fd = int(syscall[1], 16) if len(syscall) > 1 else -1
            target = fd_target(pid, fd)
            self.syscall_bytes[f"{comm}: {target}"] += new_bytes
        else:
            self.fault_bytes += new_bytes


def read_text(path: Path) -> str:
    try:
        return path.read_text()
    except OSError:
        return ""


def read_pids(procs_path: Path) -> list[int]:
    return [int(line) for line in read_text(procs_path).split() if line.isdigit()]


def io_read_bytes(path: Path) -> int | None:
    for line in read_text(path).splitlines():
        if line.startswith("read_bytes:"):
            return int(line.split()[1])
    return None


def thread_majflt(path: Path) -> int | None:
    text = read_text(path)
    if not text:
        return None
    # Fields after the comm, which can hold spaces; majflt is field 12.
    fields = text.rsplit(")", 1)[-1].split()
    return int(fields[9]) if len(fields) > 9 else None


def fd_target(pid: int, fd: int) -> str:
    if fd < 0:
        return "unknown fd"
    try:
        return os.readlink(f"/proc/{pid}/fd/{fd}")
    except OSError:
        return f"closed fd {fd}"


def key_values(path: Path, keys: set[str]) -> dict[str, int]:
    values: dict[str, int] = {}
    for line in read_text(path).splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0] in keys:
            values[parts[0]] = int(parts[1])
    return values


def scope_read_bytes(cgroup: Path) -> int:
    total = 0
    for line in read_text(cgroup / "io.stat").splitlines():
        for item in line.split()[1:]:
            name, _, value = item.partition("=")
            if name == "rbytes":
                total += int(value)
    return total


def load_1m() -> float:
    return float(read_text(Path("/proc/loadavg")).split()[0])


def counters(cgroup: Path) -> Counters:
    host = key_values(Path("/proc/vmstat"), {"pswpin", "pgmajfault"})
    scope = key_values(cgroup / "memory.stat", {"pswpin", "pgmajfault"})
    return Counters(
        at=datetime.now(timezone.utc).isoformat(),
        load_1m=load_1m(),
        host_pswpin=host.get("pswpin", 0),
        host_pgmajfault=host.get("pgmajfault", 0),
        scope_read_bytes=scope_read_bytes(cgroup),
        scope_pswpin=scope.get("pswpin", 0),
        scope_pgmajfault=scope.get("pgmajfault", 0),
    )


def scope_cgroup(unit: str, limit_s: float) -> Path:
    deadline = time.monotonic() + limit_s
    while time.monotonic() < deadline:
        result = subprocess.run(
            ["systemctl", "--user", "show", "-p", "ControlGroup", "--value", f"{unit}.scope"],
            capture_output=True,
            text=True,
            check=False,
        )
        group = result.stdout.strip()
        if group:
            return Path("/sys/fs/cgroup") / group.lstrip("/")
        time.sleep(0.2)
    raise RuntimeError(f"scope {unit} has no cgroup")


def wait_for_marker(log: Path, limit_s: float, child: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + limit_s
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError("cargo-port exited before startup finished")
        if STEADY_MARKER in read_text(log):
            return
        time.sleep(1)
    raise RuntimeError(f"startup did not finish in {limit_s:.0f} s")


def log_time(line: str) -> datetime | None:
    match = LOG_TIME.match(line)
    if match is None:
        return None
    return datetime.fromisoformat(match.group(1)).replace(tzinfo=timezone.utc)


def summarize_log(log: Path, start: datetime, end: datetime) -> tuple[Counter[str], Counter[str], int, int, Counter[str]]:
    events: Counter[str] = Counter()
    git_reads: Counter[str] = Counter()
    unmeasured = 0
    disk_bytes = 0
    event_paths: Counter[str] = Counter()
    for raw in read_text(log).splitlines():
        line = ANSI.sub("", raw)
        at = log_time(line)
        if at is None or at < start or at > end:
            continue
        fields = {match.group(1): match.group(2).strip('"') for match in FIELD.finditer(line)}
        words = line.split()
        name = words[2] if len(words) > 2 else "?"
        events[name] += 1
        if name == "git_info_get_call":
            value = fields.get("read_bytes", "")
            if value.isdigit():
                git_reads[f"{fields.get('op', '?')} {fields.get('repo_root', '?')}"] += int(value)
            else:
                unmeasured += 1
        if name == "watcher_disk_usage" and fields.get("read_bytes", "").isdigit():
            disk_bytes += int(fields["read_bytes"])
        if "event_path" in fields:
            event_paths[f"{name} {fields['event_path']}"] += 1
    return events, git_reads, unmeasured, disk_bytes, event_paths


class Options(argparse.Namespace):
    binary: str = ""
    out: str = ""
    max_load: float = 0.0
    load_wait: float = 0.0
    startup_limit: float = 0.0
    settle: float = 0.0
    duration: float = 0.0
    sample_ms: float = 0.0
    trace: bool = False


class Run:
    def __init__(self, args: Options) -> None:
        self.binary: Path = Path(args.binary).resolve()
        self.out: Path = Path(args.out).resolve()
        self.trace: bool = args.trace
        self.unit: str = f"cargo-port-idle-{os.getpid()}-{int(time.time())}"
        self.cgroup: Path | None = None
        self.app: subprocess.Popen[bytes] | None = None
        self.vmstat: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self.out.mkdir(parents=True, exist_ok=True)
        vmstat_out = (self.out / "vmstat.txt").open("wb")
        self.vmstat = subprocess.Popen(["vmstat", "-t", "1"], stdout=vmstat_out, start_new_session=True)
        env = dict(os.environ)
        env["TMPDIR"] = str(self.out)
        env["TUI_PANE_LOG"] = TRACE_LOG_FILTER if self.trace else NORMAL_LOG_FILTER
        command = f"stty cols 200 rows 50; exec {self.binary}"
        self.app = subprocess.Popen(
            ["systemd-run", "--user", "--scope", "-q", "--unit", self.unit, "--",
             "script", "-q", "-f", "-c", command, str(self.out / "screen.txt")],
            cwd=self.out,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            start_new_session=True,
        )
        self.cgroup = scope_cgroup(self.unit, 10)

    def stop(self) -> None:
        if self.cgroup is not None:
            try:
                _ = (self.cgroup / "cgroup.kill").write_text("1")
            except OSError:
                pass
            deadline = time.monotonic() + STOP_LIMIT_S
            while read_pids(self.cgroup / "cgroup.procs") and time.monotonic() < deadline:
                time.sleep(0.1)
            if read_pids(self.cgroup / "cgroup.procs"):
                print(f"failed: scope {self.unit} still has processes", file=sys.stderr)
        for child in (self.app, self.vmstat):
            if child is None or child.poll() is not None:
                continue
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            _ = child.wait()


def wait_for_load(max_load: float, limit_s: float) -> bool:
    deadline = time.monotonic() + limit_s
    while load_1m() >= max_load:
        if time.monotonic() > deadline:
            return False
        time.sleep(20)
    return True


def measure(run: Run, args: Options) -> Summary:
    run.start()
    assert run.app is not None and run.cgroup is not None
    log = run.out / PERF_LOG_NAME
    if run.trace:
        wait_for_marker(log, args.startup_limit, run.app)
    else:
        time.sleep(args.startup_limit)
    time.sleep(args.settle)
    sampler = Sampler(run.cgroup / "cgroup.procs")
    first = counters(run.cgroup)
    started = time.monotonic()
    deadline = started + args.duration
    while time.monotonic() < deadline:
        if run.app.poll() is not None:
            raise RuntimeError("cargo-port exited during the measured interval")
        if run.trace:
            sampler.sample()
        time.sleep(args.sample_ms / 1000)
    last = counters(run.cgroup)
    interval = time.monotonic() - started
    events, git_reads, unmeasured, disk_bytes, event_paths = summarize_log(
        log, datetime.fromisoformat(first["at"]), datetime.fromisoformat(last["at"])
    )
    read_bytes = last["scope_read_bytes"] - first["scope_read_bytes"]
    swapin = (last["scope_pswpin"] - first["scope_pswpin"]) * PAGE_BYTES
    faults = last["scope_pgmajfault"] - first["scope_pgmajfault"]
    swapin_pages = last["scope_pswpin"] - first["scope_pswpin"]
    return Summary(
        binary=str(run.binary),
        trace=run.trace,
        start=first,
        end=last,
        interval_s=round(interval, 1),
        scope_read_bytes=read_bytes,
        scope_swapin_bytes=swapin,
        scope_file_fault_count=max(faults - swapin_pages, 0),
        reads_after_swapin=read_bytes - swapin,
        host_pswpin_bytes=(last["host_pswpin"] - first["host_pswpin"]) * PAGE_BYTES,
        log_events=dict(events.most_common()),
        git_reads_by_op=dict(git_reads.most_common()),
        git_unmeasured_calls=unmeasured,
        disk_usage_read_bytes=disk_bytes,
        event_paths=dict(event_paths.most_common(40)),
        sampled_syscall_bytes=dict(sampler.syscall_bytes.most_common(40)),
        sampled_fault_bytes=sampler.fault_bytes,
        sampled_fault_count=sampler.fault_count,
    )


def parse_args() -> Options:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    _ = parser.add_argument("--binary", required=True)
    _ = parser.add_argument("--out", required=True)
    _ = parser.add_argument("--max-load", required=True, type=float)
    _ = parser.add_argument("--load-wait", required=True, type=float, help="seconds to wait for the load")
    _ = parser.add_argument("--startup-limit", required=True, type=float, help="seconds to wait for startup")
    _ = parser.add_argument("--settle", required=True, type=float, help="seconds after startup")
    _ = parser.add_argument("--duration", required=True, type=float, help="measured seconds")
    _ = parser.add_argument("--sample-ms", type=float, default=100)
    _ = parser.add_argument("--trace", action="store_true")
    return parser.parse_args(namespace=Options())


def main() -> int:
    args = parse_args()
    if not wait_for_load(args.max_load, args.load_wait):
        print(f"skipped: load stayed at or over {args.max_load}", file=sys.stderr)
        return 3
    run = Run(args)

    def on_signal(signum: int, _frame: FrameType | None) -> None:
        run.stop()
        sys.exit(128 + signum)

    _ = signal.signal(signal.SIGTERM, on_signal)
    _ = signal.signal(signal.SIGINT, on_signal)
    try:
        summary = measure(run, args)
    except RuntimeError as error:
        print(f"failed: {error}", file=sys.stderr)
        return 1
    finally:
        run.stop()
    text = json.dumps(summary, indent=2)
    _ = (run.out / "summary.json").write_text(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
