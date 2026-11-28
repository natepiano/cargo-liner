#!/bin/sh
# Run one nextest test and report the scratch paths it fails to remove.

case $1 in
    /*) executable=$1 ;;
    */*) executable=$PWD/$1 ;;
    *) executable=$(command -v "$1") || exit 127 ;;
esac
profile=${executable%/*}
case $profile in */deps) profile=${profile%/*} ;; esac
parent=${CARGO_BERTH_TEST_SCRATCH_PARENT:-${profile%/*}/tmp/cargo-berth-templates/private}
if [ ! -d "$parent" ]; then
    mkdir -p "$parent" || exit 1
fi
parent=$(CDPATH= cd -P "$parent" && pwd -P) || exit 1
root=$(mktemp -d "$parent/cargo-berth-test-XXXXXXXXXX") || exit 1
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
