# cargo-port

[![CI](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cargo-port.svg)](https://crates.io/crates/cargo-port)
[![docs.rs](https://docs.rs/cargo-port/badge.svg)](https://docs.rs/cargo-port)
[![license](https://img.shields.io/crates/l/cargo-port.svg)](LICENSE-MIT)

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

### Owned Owners

To edit Version and Description fields directly from the TUI, add your GitHub username and/or organization names to `owned_owners`:

```toml
[tui]
owned_owners = ["your-username", "your-org"]
```

Only projects whose GitHub owner matches one of these entries will have editable fields. This prevents accidental edits to forked or third-party projects.

### Include Non-Rust Projects

To also show non-Rust git repositories in the project tree:

```toml
[tui]
include_non_rust = true
```

These show up with reduced details (no types, version, examples) but can still display disk usage, git info, and CI runs.
