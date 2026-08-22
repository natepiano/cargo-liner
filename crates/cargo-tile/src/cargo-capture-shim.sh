#!/bin/sh
# cargo-tile-capture-shim
#
# Installed and owned by the cargo-tile binary: `cargo-tile install`
# writes this file from the copy compiled into it, so editing it in a
# toolchain's bin achieves nothing the next install will not undo.
#
# It stands where a toolchain's `cargo` was, with the real binary kept
# beside it as `cargo-tile-real`. Every run it captures is mirrored into
# a log the grid reads cargo's own progress counter out of, and is
# registered for as long as it is alive. It never alters what cargo
# does, what it prints, or what it exits with.
#
# POSIX sh on purpose. This runs in front of every cargo invocation on
# the machine, on macOS and on Linux, where neither zsh nor bash is a
# safe assumption -- /bin/sh is dash on Debian and bash in POSIX mode on
# macOS, and nothing here may depend on which.
set -u

# Resolve this script through any symlinks to reach the real cargo kept
# beside it. `readlink -f` would do it in one step but is GNU-only.
self=$0
while [ -L "$self" ]; do
    link=$(readlink "$self")
    case $link in
        /*) self=$link ;;
        *) self=$(dirname "$self")/$link ;;
    esac
done
self_dir=$(cd "$(dirname "$self")" && pwd -P)
real="$self_dir/cargo-tile-real"
if [ ! -x "$real" ]; then
    printf '%s\n' "cargo-tile: real cargo missing at $real -- run 'cargo-tile uninstall' to repair" >&2
    exit 127
fi

# Cargo publishes the binary it is running as `CARGO`, and the tools
# that wrap it honour that over the path: `cargo-clippy` and
# `cargo-nextest` both invoke `$CARGO` rather than looking `cargo` up
# again. Left alone it names the real binary the shim exec'd into, so
# every run those tools start goes around the shim -- and the value
# outlives the run in any environment that inherits it, which takes the
# shim out of the picture for good. Naming the shim instead keeps them
# coming back through here and costs nothing, since the shim ends in the
# real binary either way. Cargo takes an already-set `CARGO` over its own
# path, so this survives into everything it starts.
#
# `CARGO` is not `CARGO_`-prefixed, so it is not among the variables
# sccache hashes into its cache key.
CARGO=$self_dir/${self##*/}
export CARGO

root=${CARGO_TILE_ROOT:-/tmp/cargo-tile}
pids="$root/state/pids"

capture=1

# Query and tooling invocations compile nothing, so they have no progress
# to report. The subcommand is the first argument past any +toolchain
# selector.
#
# A `--message-format=json` run is not among them. Its caller is usually
# rust-analyzer, and it compiles and takes the build-directory lock like
# any other, so a build waiting behind one has to be able to say so. It
# is only ever captured down the no-terminal path -- the pty path needs
# all three streams on a tty, which a caller parsing the output never has
# -- and that path mirrors stderr alone, leaving the JSON on stdout byte
# for byte what the caller expects.
first=
for arg in "$@"; do
    case $arg in
        +*) continue ;;
    esac
    first=$arg
    break
done
case $first in
    metadata | pkgid | locate-project | read-manifest | config | -V | --version | -vV | --list | '')
        capture=0
        ;;
    # This workspace's own terminal UIs, reached as `cargo tile` and
    # `cargo port`. They compile nothing, and capturing one would run a
    # whole terminal UI under `script` -- every redraw copied into a log
    # for as long as it stays open, and it listing itself as a running
    # invocation.
    tile | port)
        capture=0
        ;;
esac
# A nested cargo -- a build script, or cargo driving cargo -- is already
# inside a captured run and must not open a second one. The flag carries
# the enclosing shim's pid.
#
# Liveness alone does not prove nesting. A captured run that outlives the
# build that started it leaves the flag naming a pid that stays alive for
# hours, and anything inheriting that environment would go uncaptured for
# just as long. Nesting is ancestry, so the pid counts only when it really
# is above this one. The walk runs inside awk over a single `ps` sweep
# rather than forking once per hop, and stops at the same depth the grid's
# own parent walk does.
#
# This is the last gate on purpose: it costs a process listing, and the
# cheap argument tests above have already dismissed the query invocations
# rust-analyzer issues constantly.
#
# The name must NOT begin with CARGO_: sccache hashes every CARGO_*
# variable into its cache key, so a per-run value under that prefix would
# make every compilation unique and drive the hit rate to zero.
encloses_this_run() {
    ps -Ao pid=,ppid= 2>/dev/null | awk -v self="$$" -v enclosing="$1" '
        { parent[$1] = $2 }
        END {
            walk = self
            for (hop = 0; hop < 32; hop++) {
                walk = parent[walk]
                if (walk == "" || walk + 0 <= 1) exit 1
                if (walk + 0 == enclosing + 0) exit 0
            }
            exit 1
        }
    '
}
case ${CARGOTILE_NESTED:-} in
    '' | *[!0-9]*) ;;
    *) if encloses_this_run "$CARGOTILE_NESTED"; then capture=0; fi ;;
esac

[ "$capture" -eq 1 ] || exec "$real" "$@"
mkdir -p "$pids" 2>/dev/null || exec "$real" "$@"

CARGOTILE_NESTED=$$
export CARGOTILE_NESTED
log="$root/run-$(date +%Y%m%d-%H%M%S)-$$.log"

# The grid reads the working directory and command from this file, and
# treats its presence as proof the run is still going.
case $PWD in
    "$HOME") directory='~' ;;
    "$HOME"/*) directory="~${PWD#"$HOME"}" ;;
    *) directory=$PWD ;;
esac
if [ "$#" -gt 0 ]; then
    printf '%s\tcargo %s\n' "$directory" "$*" > "$pids/$$"
else
    printf '%s\tcargo\n' "$directory" > "$pids/$$"
fi

fifo=
# What makes a finished log worth keeping: cargo's unit counter, or the
# line it prints while it waits for the build-directory lock. These are
# the two things the grid reads, spelled here as an ERE because the shim
# cannot see the constants the reader uses.
worth_keeping='waiting for file lock|\] [0-9]+/[0-9]+:'
cleanup() {
    rm -f "$pids/$$"
    if [ -n "$fifo" ]; then rm -f "$fifo"; fi
    # A run that reached no unit and waited on no lock leaves a log with
    # nothing in it the grid could ever have read. Editors issue those
    # constantly -- rust-analyzer checks on every save -- and nothing
    # else prunes the directory, so they go here rather than accumulate
    # one per save until the system sweeps /tmp. A log that did record
    # something is left alone: `grep -q` stops at the first match, so
    # the scan costs a real build almost nothing and only reads an empty
    # one to the end.
    if [ -f "$log" ] && ! grep -qE "$worth_keeping" "$log" 2> /dev/null; then
        rm -f "$log"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Quote a command and its arguments into the single command line that
# util-linux's `script` takes, since it reads a string where the BSD one
# reads an argument list.
quote_command() {
    quoted=
    for word in "$@"; do
        escaped=$(printf '%s' "$word" | sed "s/'/'\\\\''/g")
        quoted="$quoted'$escaped' "
    done
    printf '%s' "$quoted"
}

# The two `script` implementations disagree about their arguments and
# neither accepts the other's form, so which one is here has to be
# settled before it is called. Only util-linux answers `--version`.
pty=none
if command -v script > /dev/null 2>&1; then
    if script --version 2> /dev/null | grep -q util-linux; then
        pty=util_linux
    else
        pty=bsd
    fi
fi

if [ -t 0 ] && [ -t 1 ] && [ -t 2 ] && [ "$pty" != none ]; then
    # A pty gives cargo a terminal to draw its progress bar on, which is
    # where the counter comes from, and leaves the run looking to the
    # caller exactly as it would have without any of this.
    if [ "$pty" = util_linux ]; then
        # -e is what makes it exit with the child's status rather than
        # its own; the BSD one does that already. -f flushes the log
        # after every write, for the reason -t 0 gives below.
        script -q -e -f -c "$(quote_command "$real" "$@")" "$log"
    else
        # -t 0 flushes the log after every write. Left at its default
        # the BSD one holds output for thirty seconds at a time, which
        # is longer than many runs last and long enough to make the one
        # line a blocked run prints useless: cargo says it is waiting
        # immediately, the terminal shows it immediately, and the log
        # the grid reads stays empty until the wait is long over.
        script -q -t 0 "$log" "$real" "$@"
    fi
    status=$?
else
    # No terminal, so the bar has to be asked for -- and cargo rejects
    # `always` unless a width comes with it. stderr is what the bar is
    # drawn on; stdout is left alone so piped output stays byte for byte
    # what the caller expects.
    export CARGO_TERM_PROGRESS_WHEN=${CARGO_TERM_PROGRESS_WHEN:-always}
    export CARGO_TERM_PROGRESS_WIDTH=${CARGO_TERM_PROGRESS_WIDTH:-100}
    fifo="$root/state/stderr-$$"
    rm -f "$fifo"
    if mkfifo "$fifo" 2> /dev/null; then
        tee -a "$log" < "$fifo" >&2 &
        tee_pid=$!
        "$real" "$@" 2> "$fifo"
        status=$?
        # Let tee drain the pipe before the run is taken off the live
        # list, so the last redraw is in the log when the grid looks.
        wait "$tee_pid" 2> /dev/null
    else
        "$real" "$@"
        status=$?
    fi
fi

exit $status
