#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

export RUSTC_WRAPPER=

echo "==> cargo +nightly fmt -- --check"
cargo +nightly fmt -- --check

if command -v taplo >/dev/null 2>&1; then
  echo "==> taplo fmt --check"
  taplo fmt --check
else
  echo "==> taplo fmt --check (skipped: taplo not installed)"
fi

echo "==> cargo clippy --all-targets"
cargo clippy --all-targets

echo "==> cargo build --release"
cargo build --release

echo "==> cargo nextest run"
CARGO_TERM_COLOR=never cargo nextest run

echo "==> cargo run -- --fail-on-warn"
cargo run -- --fail-on-warn
