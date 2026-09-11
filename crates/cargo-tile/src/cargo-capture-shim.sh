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

capture_parent=/tmp/cargo-tile
capture_uid=$(id -u) || exec "$real" "$@"
capture_user=$(id -un) || exec "$real" "$@"
root="$capture_parent/$capture_uid"
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
    # This workspace's coordination sibling, reached as `cargo berth`.
    # It is fired from an editor hook several times a second and exits
    # in well under a poll interval, so capturing it opens a log per
    # invocation for a run with no build in it to mirror. The grid
    # leaves it out through `commands.excluded`; this is the same
    # decision on the capture side, kept here because a POSIX shell
    # cannot read the TOML the grid reads.
    berth)
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
# Settle the capture path before setup so even FIFO failures can pass
# the caller's original arguments and environment through unchanged.
pty=none
if [ -t 0 ] && [ -t 1 ] && [ -t 2 ] && command -v script > /dev/null 2>&1; then
    if script --version 2> /dev/null | grep -q util-linux; then
        pty=util_linux
    else
        pty=bsd
    fi
fi

# Calendar text is for people; the UUID prevents a later invocation from
# reusing a name after a reader has already approved its old deletion.
invocation=$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen 2>/dev/null) || exec "$real" "$@"
case $invocation in
    '' | *[!0-9a-fA-F-]*) exec "$real" "$@" ;;
esac
generation=$(date +%Y%m%d-%H%M%S) || exec "$real" "$@"
[ -n "$generation" ] || exec "$real" "$@"
generation="$generation-$invocation"
log_basename="run-$generation-$$.log"
log_path="$root/$log_basename"
registration_path="$pids/$$.$generation"
temporary_path="$registration_path.tmp"
fifo_path=
if [ "$pty" = none ]; then fifo_path="$root/state/stderr-$$.$generation"; fi

# Only owned artifacts enter cleanup. In particular, a failed exclusive
# publication must never remove the registration that already held the
# name. The setup child also installs cleanup: caught traps are reset
# on entering a subshell, and it owns partial setup until it succeeds.
temporary=
registration=
log=
fifo=
cleanup() {
    rm -f "$temporary" "$registration" "$log"
    if [ -n "$fifo" ]; then rm -f "$fifo"; fi
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# A signal delivered during an external setup command must wait until
# its result tells cleanup which names this invocation owns. The parent
# also waits for the setup child before adopting or discarding its work.
setup_signal=0
trap 'setup_signal=129' HUP
trap 'setup_signal=130' INT
trap 'setup_signal=143' TERM
setup_capture() (
    finish_setup() {
        result=$?
        trap - 0
        trap '' HUP INT TERM
        if [ "$setup_signal" -ne 0 ]; then result=$setup_signal; fi
        if [ "$result" -ne 0 ]; then cleanup; fi
        exit "$result"
    }
    trap finish_setup 0
    trap 'setup_signal=129' HUP
    trap 'setup_signal=130' INT
    trap 'setup_signal=143' TERM
    umask 0022 || exit 1
    # Do not follow a replaced account directory or any registration ancestor.
    owns_directory() {
        [ ! -L "$1" ] &&
            [ -n "$(find "$1" -maxdepth 0 -type d -user "$capture_user" -print)" ]
    }
    [ ! -L "$capture_parent" ] || exit 1
    if [ ! -d "$capture_parent" ]; then
        mkdir "$capture_parent" || [ -d "$capture_parent" ] || exit 1
    fi
    if owns_directory "$capture_parent"; then
        chmod 1777 "$capture_parent" || exit 1
    fi
    for directory in "$root" "$root/state" "$pids"; do
        if [ ! -e "$directory" ] && [ ! -L "$directory" ]; then
            mkdir "$directory" || [ -d "$directory" ] || exit 1
        fi
        owns_directory "$directory" || exit 1
        chmod 0755 "$directory" || exit 1
    done

    # A killed shim cannot run its exit trap. Its account's next invocation
    # removes each dead pid's exact publication, staging file, log, and FIFO.
    # This shim has not registered yet, so its own pid names a predecessor.
    for stale in "$pids"/*; do
        [ -e "$stale" ] || [ -L "$stale" ] || continue
        name=${stale##*/}
        pid=${name%%.*}
        case $pid in '' | *[!0-9]* | 0) continue ;; esac
        if [ "$pid" != "$$" ] && kill -0 "$pid" 2>/dev/null; then continue; fi
        publication=${name%.tmp}
        case $publication in
            "$pid".*)
                stale_generation=${publication#*.}
                rm -f "$root/run-$stale_generation-$pid.log"
                ;;
        esac
        rm -f "$pids/$publication" "$pids/$publication.tmp" "$root/state/stderr-$publication"
    done

    # The sentinel protects newlines belonging to the directory name.
    # Remove it and exactly the one newline written by pwd itself.
    directory=$(pwd -P && printf '.') || exit 1
    directory=${directory%.}
    directory=${directory%?}
    # A relative HOME cannot identify a prefix of this absolute directory.
    # Keep the caller's HOME unchanged; only the serialized field is absent.
    writer_home=${HOME-}
    case $writer_home in /*) ;; *) writer_home= ;; esac

    boot=
    birth=
    platform=$(uname -s) || platform=
    case $platform in
        Linux)
            boot=$(cat /proc/sys/kernel/random/boot_id) || boot=
            # The last ')' ends comm, even when it contains spaces or
            # parentheses. Field 22 is field 20 of what follows it.
            # $$ stays the parent shim's pid inside this setup child.
            birth=$(awk '{ sub(/^.*\) /, ""); print $20 }' "/proc/$$/stat") || birth=
            ;;
        Darwin)
            # A calendar correction can change kern.boottime during a live run.
            boot=$(sysctl -n kern.bootsessionuuid) || boot=
            birth=$(
                # UTC avoids ambiguous local times during a DST fallback.
                started=$(LC_ALL=C TZ=UTC0 ps -o lstart= -p "$$") || exit 1
                # BSD ps pads lstart to its column width. Remove that padding
                # before date parses it, so it consumes the entire timestamp.
                started=$(printf '%s\n' "$started" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//') || exit 1
                LC_ALL=C TZ=UTC0 date -j -f '%a %b %e %H:%M:%S %Y' "$started" +%s
            ) || birth=
            ;;
    esac

    # A staging name is never a registration, including after SIGKILL.
    # Exclusive creation gives this POSIX open the same collision policy
    # as publication, without a platform-specific mktemp suffix option.
    # Reject non-regular existing entries too: opening an old FIFO
    # under set -C could otherwise wait forever for its reader.
    for path in "$temporary_path" "$registration_path" "$log_path"; do
        if [ -e "$path" ] || [ -L "$path" ]; then exit 1; fi
    done
    (set -C; true > "$temporary_path") || exit 1
    temporary=$temporary_path
    # Preserve directory identity and each argv word. The optional home
    # field lets a reader shorten only a prefix its own user recognizes.
    printf '%s\000' cargo-tile-v2 "$generation" "$boot" "$birth" \
        "$log_basename" "$directory" "$writer_home" "$#" "$@" > "$temporary" || exit 1
    chmod 0644 "$temporary" || exit 1
    if [ -n "$fifo_path" ]; then
        # Remove any stale FIFO at this exact publication name before creating it.
        rm -f "$fifo_path" || exit 1
        mkfifo "$fifo_path" || exit 1
        fifo=$fifo_path
        chmod 0640 "$fifo" || exit 1
    fi
    # Unlike mv -f, a hard link cannot replace a prior registration
    # after pid reuse or a backward clock step reproduces its name.
    ln "$temporary" "$registration_path" || exit 1
    # A directory arriving after the existence check makes POSIX ln
    # succeed inside that directory. This successful command owns only
    # the nested link; preserve the directory and reject publication.
    if [ -d "$registration_path" ]; then
        rm -f "$registration_path/${temporary##*/}"
        exit 1
    fi
    registration=$registration_path
    # Publish first so a concurrent sweep cannot remove an orphan log.
    # true is not a special builtin: redirection failure returns to the
    # setup policy instead of terminating cargo's parent shell.
    (set -C; true > "$log_path") || exit 1
    log=$log_path
    chmod 0644 "$log" || exit 1
    rm -f "$temporary" || exit 1
)

if setup_capture "$@" 2>/dev/null; then
    registration=$registration_path
    log=$log_path
    fifo=$fifo_path
else
    result=$?
    cleanup
    trap - 0
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    if [ "$setup_signal" -ne 0 ]; then result=$setup_signal; fi
    case $result in
        129|130|143) exit "$result" ;;
    esac
    exec "$real" "$@"
fi
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
if [ "$setup_signal" -ne 0 ]; then exit "$setup_signal"; fi

# The setup subshell contained the permission window. Cargo and script
# inherit the caller's original umask on every path from here onward.
CARGOTILE_NESTED=$$
export CARGOTILE_NESTED

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

# Whether a caller is reading cargo's machine-readable output, which is
# what makes stderr free to carry the status this capture is for. Both
# spellings, since `--message-format json` is as valid as the joined
# form, and only ahead of a `--`, past which the arguments are another
# command's.
json=0
previous=
for arg in "$@"; do
    case $arg in
        --) break ;;
        --message-format=json*) json=1; break ;;
        json*)
            if [ "$previous" = --message-format ]; then
                json=1
                break
            fi
            ;;
    esac
    previous=$arg
done

if [ "$pty" != none ]; then
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
    # `--quiet` empties the log of the one thing a waiting run has to
    # say. Under it cargo prints no status at all, so an invocation
    # blocked on the artifact-directory lock says nothing for as long as
    # it waits -- not the lock line, and not a counter either, because it
    # has not started compiling. The log stays empty, the grid finds no
    # capture at or above the pid, and a check stuck behind a build reads
    # as idle rather than blocked. rust-analyzer passes `--quiet` on
    # every check it issues, which is most of what a working editor puts
    # through here.
    #
    # Dropping it puts the wait back on stderr and leaves stdout alone,
    # which is where a `--message-format=json` caller reads and what it
    # parses byte for byte. Only for those callers: a person who asked a
    # build to be quiet is asking about the output in front of them, and
    # that is theirs. Only past this branch, too -- a terminal run is a
    # person's by definition.
    #
    # Arguments after `--` belong to whatever cargo is about to run, so
    # the rewrite stops there.
    if [ "$json" -eq 1 ]; then
        remaining=$#
        passthrough=0
        while [ "$remaining" -gt 0 ]; do
            arg=$1
            shift
            remaining=$((remaining - 1))
            if [ "$passthrough" -eq 0 ]; then
                case $arg in
                    --) passthrough=1 ;;
                    --quiet | -q) continue ;;
                esac
            fi
            set -- "$@" "$arg"
        done
    fi
    # No terminal, so the bar has to be asked for -- and cargo rejects
    # `always` unless a width comes with it. stderr is what the bar is
    # drawn on; stdout is left alone so piped output stays byte for byte
    # what the caller expects.
    export CARGO_TERM_PROGRESS_WHEN=${CARGO_TERM_PROGRESS_WHEN:-always}
    export CARGO_TERM_PROGRESS_WIDTH=${CARGO_TERM_PROGRESS_WIDTH:-100}
    tee -a "$log" < "$fifo" >&2 &
    tee_pid=$!
    "$real" "$@" 2> "$fifo"
    status=$?
    # Let tee drain the pipe before the run is taken off the live
    # list, so the last redraw is in the log when the grid looks.
    wait "$tee_pid" 2> /dev/null
fi

exit $status
