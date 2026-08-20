#!/usr/bin/env bash
set -euo pipefail

# Post-release install verification, dispatched on the crate being released.
# /release passes the version as $1 and the crate name as $2; the crate name is
# required here because cargo-liner publishes one crate at a time and each one
# installs differently.

VERSION="${1:?Usage: install_verify.sh <version> <crate>}"
CRATE="${2:?Usage: install_verify.sh <version> <crate>}"

case "${CRATE}" in
cargo-port)
    echo "Installing cargo-port v${VERSION}..."
    cargo install cargo-port --version "${VERSION}"
    echo "Install verified: cargo-port v${VERSION}"
    ;;
tui_pane)
    echo "tui_pane v${VERSION} is a library — nothing to install."
    ;;
*)
    echo "install_verify.sh: no install rule for crate '${CRATE}'" >&2
    exit 1
    ;;
esac
