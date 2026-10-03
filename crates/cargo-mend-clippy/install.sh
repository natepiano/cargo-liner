#!/usr/bin/env bash
# Builds this crate and installs its `cargo-mend` binary, which runs clippy's
# lints in mend's own compile, in place of the installed `cargo-mend`. Run it
# from any directory after every `rustup update stable`: it fetches clippy's
# source for the new stable rustc when that commit is not cached yet, then
# rebuilds. `cargo mend --clippy-status` reports whether the installed build
# matches the host rustc.
#
# Arguments pass through to `cargo install`, e.g. `--root <dir>` to install
# somewhere other than ~/.cargo/bin.
set -euo pipefail

crate_dir=$(dirname "$(realpath "$0")")

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

rustc_info=$(rustc +stable -vV) ||
    fail "\`rustc +stable -vV\` failed; install the stable toolchain with \`rustup toolchain install stable\`"
rustc_version=$(head -n 1 <<<"$rustc_info")
commit=$(sed -n 's/^commit-hash: //p' <<<"$rustc_info")
host=$(sed -n 's/^host: //p' <<<"$rustc_info")
[[ $commit =~ ^[0-9a-f]{40}$ ]] ||
    fail "\`rustc +stable -vV\` reported no commit hash for $rustc_version"

rustup component list --toolchain stable --installed | grep -x "rustc-dev-$host" >/dev/null ||
    fail "the stable toolchain has no rustc-dev; run \`rustup component add rustc-dev --toolchain stable\`"

# Fetches only when the commit is not cached, and points `clippy` at it.
"$crate_dir/fetch-clippy.sh" "$commit" ||
    fail "fetching clippy's source for $rustc_version (commit $commit) failed; see the git error above"

# `--force` replaces a `cargo-mend` binary installed from the `cargo-mend`
# package. No `--locked`: Cargo.lock records the versions of clippy's crates and
# of cargo-mend, so it stops matching on every toolchain bump and release.
if ! RUSTC_BOOTSTRAP=1 cargo +stable install --path "$crate_dir" --force "$@"; then
    fail "building cargo-mend with clippy for $rustc_version failed; see the cargo error above.
The installed cargo-mend is unchanged. To install cargo-mend without clippy's lints instead, run:
  RUSTC_BOOTSTRAP=1 cargo +stable install --path $(dirname "$crate_dir")/cargo-mend --force"
fi
