#!/usr/bin/env bash
set -euo pipefail

# cargo-mend links against rustc_driver. Installing with stable + RUSTC_BOOTSTRAP
# ensures the binary can read .rmeta files from stable-toolchain projects.
# Installing with nightly produces a binary that fails with E0514 on stable projects.

VERSION="${1:?Usage: install_verify.sh <version>}"

echo "Installing cargo-mend v${VERSION} with stable toolchain..."
RUSTC_BOOTSTRAP=1 cargo +stable install cargo-mend --version "${VERSION}"
echo "Install verified: cargo-mend v${VERSION}"
