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
cargo-mend)
    # cargo-mend links rustc_driver. Installing with stable + RUSTC_BOOTSTRAP
    # produces a binary that can read .rmeta from stable-toolchain projects;
    # installing with nightly produces one that fails with E0514 against them.
    echo "Installing cargo-mend v${VERSION} with stable toolchain..."
    RUSTC_BOOTSTRAP=1 cargo +stable install cargo-mend --version "${VERSION}"
    echo "Install verified: cargo-mend v${VERSION}"
    ;;
tui_pane)
    echo "tui_pane v${VERSION} is a library — nothing to install."
    ;;
*)
    echo "install_verify.sh: no install rule for crate '${CRATE}'" >&2
    exit 1
    ;;
esac
