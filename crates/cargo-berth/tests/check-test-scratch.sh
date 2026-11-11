#!/bin/sh
# Run one nextest test and report the scratch paths it fails to remove.

memory_has_room() {
    # Require 1 GiB free for 32 concurrent histories of up to 32 MiB each.
    minimum_free_kib=1048576
    storage=$(LC_ALL=C df -Pk /dev/shm) || return 1
    while read -r filesystem blocks used available remaining; do
        case $available in
            ''|*[!0-9]*) continue ;;
        esac
        [ "$available" -ge "$minimum_free_kib" ] && return 0
    done <<EOF
$storage
EOF
    return 1
}

case $1 in
    /*) executable=$1 ;;
    */*) executable=$PWD/$1 ;;
    *) executable=$(command -v "$1") || exit 127 ;;
esac
profile=${executable%/*}
case $profile in */deps) profile=${profile%/*} ;; esac
disk_parent=${profile%/*}/tmp/cargo-berth-templates/private
parent=${CARGO_BERTH_TEST_SCRATCH_PARENT:-$disk_parent}
# Private histories need isolation, but they do not need persistent storage.
# Keep the shared template cache on its original filesystem.
if [ -z "${CARGO_BERTH_TEST_SCRATCH_PARENT:-}" ] && [ -r /proc/mounts ] && [ -w /dev/shm ]; then
    while read -r device mount filesystem options remaining; do
        if [ "$mount" = /dev/shm ] && [ "$filesystem" = tmpfs ]; then
            case ,$options, in
                *,noexec,*) ;;
                *) memory_has_room && parent=/dev/shm ;;
            esac
            break
        fi
    done < /proc/mounts
fi
if [ ! -d "$parent" ]; then
    mkdir -p "$parent" || exit 1
fi
parent=$(CDPATH= cd -P "$parent" && pwd -P) || exit 1
root=$(mktemp -d "$parent/cargo-berth-test-XXXXXXXXXX") || {
    [ "$parent" = /dev/shm ] || exit 1
    parent=$disk_parent
    mkdir -p "$parent" || exit 1
    parent=$(CDPATH= cd -P "$parent" && pwd -P) || exit 1
    root=$(mktemp -d "$parent/cargo-berth-test-XXXXXXXXXX") || exit 1
}
cleanup() {
    trap - 0
    if ! rmdir "$root" 2>/dev/null; then
        chmod -R u+rwX "$root"
        rm -rf -- "$root"
    fi
}
propagate_signal() {
    cleanup
    trap - "$1"
    kill -s "$1" "$$"
}
trap cleanup 0
trap 'propagate_signal HUP' HUP
trap 'propagate_signal INT' INT
trap 'propagate_signal TERM' TERM
TMPDIR=$root TMP=$root TEMP=$root CARGO_BERTH_TEST_SCRATCH_ROOT=$root
# Git must never discover the checkout that contains the fixture cache.
GIT_CEILING_DIRECTORIES=$parent
export TMPDIR TMP TEMP CARGO_BERTH_TEST_SCRATCH_ROOT GIT_CEILING_DIRECTORIES

"$@"
status=$?
leaked=false
for path in "$root"/* "$root"/.[!.]* "$root"/..?*; do
    if [ -e "$path" ] || [ -L "$path" ]; then
        if [ "$leaked" = false ]; then
            printf 'test %s left scratch paths:\n' "${NEXTEST_TEST_NAME:-$*}" >&2
            leaked=true
        fi
        printf '  %s\n' "$path" >&2
    fi
done
if [ "$leaked" = true ]; then
    exit 1
fi
if [ "$status" -gt 128 ]; then
    signal=$(kill -l "$status" 2>/dev/null) || exit "$status"
    propagate_signal "$signal"
fi
exit "$status"
