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

root=${CARGO_TILE_ROOT:-/tmp/cargo-tile}
pids="$root/state/pids"

capture=1

# A nested cargo -- a build script, or cargo driving cargo -- is already
# inside a captured run and must not open a second one. The flag carries
# the enclosing shim's pid and counts only while that pid is alive, so a
# shell environment captured inside a run does not carry it forever.
#
# The name must NOT begin with CARGO_: sccache hashes every CARGO_*
# variable into its cache key, so a per-run value under that prefix would
# make every compilation unique and drive the hit rate to zero.
case ${CARGOTILE_NESTED:-} in
    '' | *[!0-9]*) ;;
    *) if kill -0 "$CARGOTILE_NESTED" 2>/dev/null; then capture=0; fi ;;
esac

# Query and tooling invocations compile nothing, so they have no progress
# to report -- and rust-analyzer issues them constantly, which would bury
# the real runs in the log directory. The subcommand is the first
# argument past any +toolchain selector.
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
    # The grid itself, reached as `cargo tile`. It compiles nothing, and
    # capturing it would run a whole terminal UI under `script` -- every
    # redraw copied into a log for as long as the grid stays open, and
    # the grid listing itself as a running invocation.
    tile)
        capture=0
        ;;
esac
previous=
for arg in "$@"; do
    case $arg in
        --message-format=json*) capture=0 ;;
        json*) if [ "$previous" = --message-format ]; then capture=0; fi ;;
    esac
    previous=$arg
done

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
cleanup() {
    rm -f "$pids/$$"
    if [ -n "$fifo" ]; then rm -f "$fifo"; fi
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
        # its own; the BSD one does that already.
        script -q -e -c "$(quote_command "$real" "$@")" "$log"
    else
        script -q "$log" "$real" "$@"
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
