#!/usr/bin/env bash
# Install the desktop entry that lets cargo-tile's or cargo-handler's attract
# backdrop draw the windows underneath it on KDE Plasma.
#
# KWin grants org.kde.KWin.ScreenShot2 only to a process whose desktop entry
# names that interface, matched by the Exec path, so a machine without this
# entry draws Plasma's wallpaper instead and says nothing about why. Each
# entry carries the full explanation; see crates/<crate>/assets.
#
#     scripts/install-desktop-entry.sh [--crate <crate>] [binary] [entry-name]
#
# The crate chooses the template, crates/<crate>/assets/<crate>.desktop.in,
# the entry name when none is passed, and the binary looked up on PATH when
# none is passed. Without --crate it is the passed binary's own file name when
# a crate has a template under that name, and cargo-tile otherwise, so a
# cargo-tile invocation reads as it always has:
#
#     scripts/install-desktop-entry.sh                     # the cargo-tile on PATH
#     scripts/install-desktop-entry.sh target/release/cargo-tile cargo-tile-dev
#     scripts/install-desktop-entry.sh --crate cargo-handler    # the cargo-handler on PATH
#     scripts/install-desktop-entry.sh target/release/cargo-handler cargo-handler-dev
#
# A cargo-handler binary under another file name needs --crate cargo-handler.
#
# Safe to repeat, and re-running it is the repair after the binary moves.
# Remove an entry by deleting it from ~/.local/share/applications.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

# The crate whose entry is installed when neither --crate nor the binary's
# file name names another.
DEFAULT_CRATE="cargo-tile"
APPLICATIONS_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"

usage() {
    printf 'usage: %s [--crate <crate>] [binary] [entry-name]\n' "$(basename -- "$0")"
}

# The template a crate's entry is written from.
template_for() {
    printf '%s/crates/%s/assets/%s.desktop.in' "${REPO_ROOT}" "$1" "$1"
}

crate=""
binary_argument=""
entry_argument=""
positionals=0
while (($# > 0)); do
    case "$1" in
        --crate)
            if (($# < 2)); then
                usage >&2
                exit 1
            fi
            crate="$2"
            shift 2
            ;;
        --crate=*)
            crate="${1#--crate=}"
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        -*)
            printf 'unknown option: %s\n' "$1" >&2
            usage >&2
            exit 1
            ;;
        *)
            case "${positionals}" in
                0) binary_argument="$1" ;;
                1) entry_argument="$1" ;;
                *)
                    usage >&2
                    exit 1
                    ;;
            esac
            positionals=$((positionals + 1))
            shift
            ;;
    esac
done

if [[ -z "${crate}" ]]; then
    crate="${DEFAULT_CRATE}"
    if [[ -n "${binary_argument}" ]]; then
        binary_name="$(basename -- "${binary_argument}")"
        if [[ -f "$(template_for "${binary_name}")" ]]; then
            crate="${binary_name}"
        fi
    fi
fi

TEMPLATE="$(template_for "${crate}")"
if [[ ! -f "${TEMPLATE}" ]]; then
    printf 'template missing: %s\n' "${TEMPLATE}" >&2
    exit 1
fi

entry_name="${entry_argument:-${crate}}"

if [[ -n "${binary_argument}" ]]; then
    if [[ ! -x "${binary_argument}" ]]; then
        printf 'no executable at %s\n' "${binary_argument}" >&2
        exit 1
    fi
    binary="$(cd -- "$(dirname -- "${binary_argument}")" && pwd)/$(basename -- "${binary_argument}")"
elif ! binary="$(command -v "${crate}")"; then
    printf '%s is not on PATH; pass the binary as an argument\n' "${crate}" >&2
    exit 1
fi

entry="${APPLICATIONS_DIR}/${entry_name}.desktop"
mkdir -p "${APPLICATIONS_DIR}"
# Written whole rather than edited in place, so a half-written entry from an
# interrupted run cannot be left behind granting the interface to nothing.
sed "s#@EXEC@#${binary}#" "${TEMPLATE}" >"${entry}.partial"
mv -- "${entry}.partial" "${entry}"

# KDE reads the entry out of its own cache, so a fresh one is invisible until
# the cache is rebuilt. Missing on a machine without Plasma, which is not an
# error: the entry is already on disk for whenever KDE next reads it.
for builder in kbuildsycoca6 kbuildsycoca5; do
    if command -v "${builder}" >/dev/null 2>&1; then
        "${builder}" >/dev/null 2>&1 || true
        break
    fi
done

printf 'installed %s\n' "${entry}"
printf '  Exec=%s\n' "${binary}"
