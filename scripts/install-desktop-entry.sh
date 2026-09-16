#!/usr/bin/env bash
# Install the desktop entry that lets cargo-tile's attract backdrop draw the
# windows underneath it on KDE Plasma.
#
# KWin grants org.kde.KWin.ScreenShot2 only to a process whose desktop entry
# names that interface, matched by the Exec path, so a machine without this
# entry draws Plasma's wallpaper instead and says nothing about why. The entry
# itself carries the full explanation; see crates/cargo-tile/assets.
#
#     scripts/install-desktop-entry.sh                     # the cargo-tile on PATH
#     scripts/install-desktop-entry.sh target/release/cargo-tile cargo-tile-dev
#
# Safe to repeat, and re-running it is the repair after the binary moves.
# Remove an entry by deleting it from ~/.local/share/applications.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

TEMPLATE="${REPO_ROOT}/crates/cargo-tile/assets/cargo-tile.desktop.in"
APPLICATIONS_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"

binary_argument="${1:-}"
entry_name="${2:-cargo-tile}"

if [[ -n "${binary_argument}" ]]; then
    if [[ ! -x "${binary_argument}" ]]; then
        printf 'no executable at %s\n' "${binary_argument}" >&2
        exit 1
    fi
    binary="$(cd -- "$(dirname -- "${binary_argument}")" && pwd)/$(basename -- "${binary_argument}")"
elif ! binary="$(command -v cargo-tile)"; then
    printf 'cargo-tile is not on PATH; pass the binary as the first argument\n' >&2
    exit 1
fi

if [[ ! -f "${TEMPLATE}" ]]; then
    printf 'template missing: %s\n' "${TEMPLATE}" >&2
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
