#!/usr/bin/env bash
# Fetches `src/tools/clippy` from rust-lang/rust at a rustc release commit into
# ~/.cache/mend-clippy-src/<commit> and points this crate's `clippy` symlink at
# it. Clippy's own release tags build against the nightly they pin, not the
# stable rustc-dev, so the source must come from the commit stable was cut at.
# Run it on every toolchain bump, before building this crate:
#
#   ./fetch-clippy.sh "$(rustc +stable -vV | sed -n 's/^commit-hash: //p')"
set -euo pipefail

if [[ $# -ne 1 || ! $1 =~ ^[0-9a-f]{40}$ ]]; then
    echo "usage: $0 <40-character rustc commit hash>" >&2
    exit 2
fi
commit=$1
cache_dir=${XDG_CACHE_HOME:-$HOME/.cache}/mend-clippy-src/$commit
crate_dir=$(dirname "$(realpath "$0")")

if [[ ! -d $cache_dir/src/tools/clippy ]]; then
    partial_dir=$cache_dir.partial
    rm -rf "$partial_dir"
    git init --quiet "$partial_dir"
    git -C "$partial_dir" remote add origin https://github.com/rust-lang/rust.git
    git -C "$partial_dir" sparse-checkout set src/tools/clippy
    git -C "$partial_dir" fetch --quiet --depth 1 --filter=blob:none origin "$commit"
    git -C "$partial_dir" checkout --quiet FETCH_HEAD
    mv "$partial_dir" "$cache_dir"
fi
ln -sfn "$cache_dir/src/tools/clippy" "$crate_dir/clippy"
echo "clippy source: $cache_dir/src/tools/clippy"
