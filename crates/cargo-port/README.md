# cargo-port

[![CI](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cargo-port.svg)](https://crates.io/crates/cargo-port)
[![docs.rs](https://docs.rs/cargo-port/badge.svg)](https://docs.rs/cargo-port)
[![license](https://img.shields.io/crates/l/cargo-port.svg)](LICENSE-MIT)

<img src="assets/screenshot.png" alt="cargo-port TUI" width="100%">

A terminal dashboard for all your Rust projects. Point it at a directory and it discovers every workspace, crate, worktree, and vendored dependency underneath.

- **Find everything** — examples, benchmarks, binaries, and test targets across all your projects in one place
- **Launch instantly** — run any example, benchmark, or binary in debug or release mode with live output
- **Jump to context** — open crates.io, GitHub, or your editor directly from any project field
- **CI at a glance** — per-project GitHub Actions status with job-level detail and run history
- **Fuzzy search** — find any project, example, or binary across your entire tree in seconds
- **Offline-ready** — CI data cached to disk, works without network

## Try me

```bash
git clone https://github.com/natepiano/cargo-port.git
cd cargo-port
cargo build
cargo run
```

## Configuration

cargo-port creates a config file on first run at:
- **macOS**: `~/Library/Application Support/cargo-port/config.toml`
- **Linux**: `~/.config/cargo-port/config.toml`

### Scan directories

By default, cargo-port scans the entire scan root (defaults to `~`). To limit scanning to specific directories, set `include_dirs` in the config file or via the in-app settings editor (press `s`).

Paths can be relative to the scan root or absolute:

```toml
[tui]
include_dirs = ["rust", "projects", "/opt/work"]
```

An empty list (the default) scans the entire scan root. Changes to `include_dirs` in the settings editor trigger an automatic rescan.

### Include Non-Rust Projects

To also show non-Rust git repositories in the project tree:

```toml
[tui]
include_non_rust = true
```

These show up with reduced details (no types, version, examples) but can still display disk usage, git info, and CI runs.

### Port Report

Port Report is cargo-port's local lint/watch runtime. When enabled, cargo-port watches only the projects you allow-list, runs configured commands when they change, and shows the current status in the project list.

Port Report is off by default.

In the Settings popup (`s`), the `Port Report` section exposes:
- `Enabled`
- `Projects`
- `Commands`

`Projects` is an allow-list. If it is empty, Port Report watches nothing.

#### Basic config

```toml
[lint]
enabled = true
include = ["cargo-port", "bevy_lagrange"]
exclude = []
commands = []
```

Notes:
- `include` entries can be bare project names, display-path prefixes, or absolute-path prefixes
- `exclude` is applied after `include`
- an empty `commands` list uses the built-in default command

#### Commands

The released default is a single clippy command:

```toml
[lint]
enabled = true
include = ["cargo-port"]
exclude = []
commands = []
```

That expands to:

```toml
[[lint.commands]]
name = "clippy"
command = "cargo clippy --workspace --all-targets --all-features --manifest-path \"$MANIFEST_PATH\" -- -D warnings"
```

If you want to override that, you can configure explicit commands:

```toml
[lint]
enabled = true
include = ["cargo-port"]

[[lint.commands]]
name = "mend"
command = "cargo mend"

[[lint.commands]]
name = "clippy"
command = "cargo clippy --workspace --all-targets --all-features -- -D warnings"
```

`command` is executed as a shell command in the project root, not as an implied Cargo subcommand. That means values like `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --manifest-path "$MANIFEST_PATH" -- -D warnings`, or `something --else` are all valid.

In the Settings popup, `Commands` accepts a comma-separated list of full shell commands.

Legacy preset-style entries such as `clippy` or `mend` are normalized to their built-in command definitions when config is loaded or saved.

#### Cache location

Port Report writes its cache under cargo-port's shared cache root.

By default this uses the platform cache directory:
- macOS: `~/Library/Caches/cargo-port`
- Linux: `~/.cache/cargo-port`

You can override the root:

```toml
[cache]
root = ""
```

Rules:
- empty string means use the default platform cache root
- a relative path extends the default cargo-port cache root
- an absolute path replaces it

Port Report data is stored under the `port-report/` cache subtree. CI cache uses the same shared root under `ci/`.
