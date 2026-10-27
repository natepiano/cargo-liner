#!/bin/sh
# Run one nextest test and report the scratch paths it fails to remove.

case $1 in
    /*) executable=$1 ;;
    */*) executable=$PWD/$1 ;;
    *) executable=$(command -v "$1") || exit 127 ;;
esac
profile=${executable%/*}
case $profile in */deps) profile=${profile%/*} ;; esac
parent=${profile%/*}/tmp/cargo-berth-templates/private
if [ ! -d "$parent" ]; then
    mkdir -p "$parent" || exit 1
fi
parent=$(CDPATH= cd -P "$parent" && pwd -P) || exit 1
root=$(mktemp -d "$parent/test-XXXXXXXXXX") || exit 1
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
rmdir "$root" || exit 1
if [ "$status" -gt 128 ]; then
    signal=$(kill -l "$status" 2>/dev/null) || exit "$status"
    trap - "$signal"
    kill -s "$signal" "$$"
fi
exit "$status"
