# Cargo mechanics — workspace conversion (Phase 1)

Spec for converting `cargo-port-api-fix` into a package-workspace with one library member `tui_pane/`. Companion to `docs/tui-pane-lib.md` Phase 1.

---

## 1. Root `Cargo.toml` after Phase 1

The root manifest is dual-role: `[package]` (binary `cargo-port`) **and** `[workspace]`. Lints move to `[workspace.lints.*]`; per-crate `[lints]` becomes a single-line inheritance.

### Full target layout

```toml
[package]
authors     = ["natepiano"]
categories  = ["command-line-utilities", "development-tools"]
description = "A TUI for inspecting and managing Rust projects"
edition     = "2024"
homepage    = "https://github.com/natepiano/cargo-port"
keywords    = ["cargo", "inspect", "project", "rust", "tui"]
license     = "MIT OR Apache-2.0"
name        = "cargo-port"
repository  = "https://github.com/natepiano/cargo-port"
version     = "0.0.4-dev"

[workspace]
members  = ["tui_pane"]
resolver = "3"

[dependencies]
ansi-to-tui        = "8.0.1"
cargo_metadata     = "0.23.1"
chrono             = "0.4.44"
clap               = { version = "4", features = ["derive"] }
colored            = "3.1.1"
comfy-table        = { version = "7", features = ["custom_styling"] }
confique           = { version = "0.4", features = ["toml"] }
crossterm          = "0.29.0"
dirs               = "6.0.0"
indexmap           = "2"
notify             = "9.0.0-rc.2"
nucleo-matcher     = "0.3.1"
ratatui            = "0.30.0"
rayon              = "1.12.0"
reqwest            = { version = "0.13.3", features = ["json"] }
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
sha2               = "0.11.0"
strum              = { version = "0.28.0", features = ["derive"] }
sysinfo            = "0.38.4"
tokei              = "14.0.0"
tokio              = { version = "1", features = ["rt-multi-thread"] }
toml               = "1.1.2"
tracing            = "0.1.44"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
unicode-width      = "0.2.2"
walkdir            = "2"

# tui_pane re-exports KeyBind / Keymap / etc. used by the binary.
tui_pane = { path = "tui_pane", version = "0.0.4-dev" }

[dev-dependencies]
tempfile = "3"

[lints]
workspace = true

[workspace.lints.clippy]
# Strict per-rule denies. Test modules opt back in via:
#   #[allow(clippy::expect_used, reason = "tests should panic on unexpected values")]
allow_attributes_without_reason = "deny"
expect_used                     = "deny"
panic                           = "deny"
self_named_module_files         = "deny" # use module/mod.rs (directory form) when a module has submodules
unreachable                     = "deny"
unwrap_used                     = "deny"

# Lint groups as deny with lower priority so per-rule allows below override.
all      = { level = "deny", priority = -1 }
cargo    = { level = "deny", priority = -1 }
nursery  = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }

# Conflicts with common patterns.
multiple_crate_versions = "allow"
redundant_pub_crate     = "allow"

[workspace.lints.rust]
missing_docs = "deny"
```

### Split rationale

Every entry in the existing `[lints.clippy]` and `[lints.rust]` blocks is generic Rust hygiene — none of it is binary-specific. **All of it moves verbatim into `[workspace.lints.*]`.** Each member then writes:

```toml
[lints]
workspace = true
```

`missing_docs = "deny"` lands in `[workspace.lints.rust]` from day one. No deferral, no per-crate override.

### `[workspace.dependencies]`

Skip in Phase 1. The shared deps between binary and library are `crossterm`, `ratatui`, `dirs` — three crates. Hoisting them now buys version alignment but adds two-line indirection per dep in both manifests. Re-evaluate when Phase 2 lands and `tui_pane/Cargo.toml` is real; until then both manifests pin the same versions inline.

### `resolver`

Edition 2024 implies resolver 3, but in a `[workspace]` the resolver is **not** inferred from the package edition — it must be set explicitly on `[workspace]`. Set `resolver = "3"` to match edition 2024 semantics. Without it, cargo emits a warning and falls back to resolver 1 for the workspace.

---

## 2. `tui_pane/Cargo.toml` after Phase 1

```toml
[package]
authors     = ["natepiano"]
description = "Reusable ratatui pane framework: keymap, status bar, framework panes."
edition     = "2024"
license     = "MIT OR Apache-2.0"
name        = "tui_pane"
repository  = "https://github.com/natepiano/cargo-port"
version     = "0.0.4-dev"

[lints]
workspace = true

[dependencies]
crossterm = "0.29.0"
dirs      = "6.0.0"
ratatui   = "0.30.0"
serde     = { version = "1", features = ["derive"] }
toml      = "1.1.2"

[dev-dependencies]
# none in Phase 1; Phase 2 invariant tests use only std + the crate's own types.
```

Notes:
- Crate name `tui_pane` (snake_case) per `user_crate_naming.md` — only `cargo-*` bin crates use hyphens.
- Versions for `crossterm` / `ratatui` / `dirs` match the root manifest verbatim. Cargo deduplicates because the version requirements are identical.
- `serde` + `toml` are required by `keymap/load.rs` (Phase 2). Including them in Phase 1 keeps Phase 2 a pure-source diff.
- No `[features]`. The framework is generic; no opt-in surface area.
- `categories` / `keywords` / `homepage` omitted — the crate is not separately published in this plan. Add them if/when publishing to crates.io.

---

## 3. `cargo install --path .` keeps working

**Rule.** A manifest may contain both `[package]` and `[workspace]`. This is a "root package" / "package workspace". `cargo install --path <dir>` looks at the manifest at `<dir>`; if it has a `[package]`, that package is installed. The presence of `[workspace]` does not change which package `--path .` installs.

Concretely, after Phase 1:

```
cargo install --path .          # installs cargo-port (the [package] at root)
cargo install --path tui_pane   # would attempt to install tui_pane, but it has no [[bin]] — no-op / error
```

Subtleties:

- **`default-members`.** Not needed. `cargo build` from the root with no `default-members` builds **all** workspace members; `cargo install --path .` is unaffected because `install` operates on the manifest at the given path, not on `default-members`. If we ever want bare `cargo run` / `cargo build` to target only the binary, set `default-members = ["."]` — but this also means `cargo build` stops type-checking `tui_pane` by default, which we do not want. Leave `default-members` unset.
- **`resolver`.** Must be set on `[workspace]` (see §1). MSRV unchanged — both members are edition 2024.
- **MSRV.** Edition 2024 requires Rust 1.85+. No new MSRV constraint introduced by going to a workspace. If a `rust-version` field is added later, it should live in `[workspace.package]` and both members inherit via `rust-version.workspace = true`.
- **`Cargo.lock` location.** Workspace lockfile lives at the workspace root (the same path it lives at today). `cargo install --path .` reads it. No relock churn.

---

## 4. `Cargo.lock` and `target/` behavior

- **`Cargo.lock`.** Stays at workspace root (`/Cargo.lock`). Cargo writes one lockfile per workspace, never per member. Existing CI cache keys on `Cargo.lock` continue to hit on the same path. No `tui_pane/Cargo.lock` is ever created (and if one appears, delete it — cargo will warn).
- **`target/`.** Stays at workspace root (`/target/`). Cargo computes target dir from the workspace root, not the invoked member. Existing `target/`-based caches and `.gitignore` entries are unaffected.
- **`.cargo/config.toml`.** None present. `ls -la /Users/natemccoy/rust/cargo-port-api-fix/.cargo/` returns no such directory. No config to update. If one is added later (e.g. for `[build] target-dir = "..."`), it must live at the workspace root, not under `tui_pane/`.

---

## 5. Stale `crates/tui_pane/` references in `docs/tui-pane-lib.md`

`grep -n 'crates/tui_pane' docs/tui-pane-lib.md`:

```
712:// crates/tui_pane re-export
897:**Root `Cargo.toml` layout.** The root manifest stays a `[package]` (binary crate) and additionally declares `[workspace]` listing `crates/tui_pane` as a member. …
899:**Lockfile and target dir.** … No new lockfile under `crates/tui_pane/`.
```

Three locations. Each needs `crates/tui_pane` → `tui_pane`:

| Line | Context | Current | Target |
| ---- | ------- | ------- | ------ |
| 712  | Code-block comment in "Cargo-port action enums" | `// crates/tui_pane re-export` | `// tui_pane re-export` |
| 897  | Phase 1 prose, `[workspace] members` description | `…listing \`crates/tui_pane\` as a member.` | `…listing \`tui_pane\` as a member.` |
| 899  | Phase 1 prose, lockfile note | `No new lockfile under \`crates/tui_pane/\`.` | `No new lockfile under \`tui_pane/\`.` |

(Per the task instructions: enumerated, not edited.)

---

## 6. Per-phase rustdoc precondition

Phase 1 sets `missing_docs = "deny"` in `[workspace.lints.rust]`. From Phase 2 onward, every `pub` item added to `tui_pane` (struct, enum, fn, trait, type alias, macro, module, re-export) **must** ship with its rustdoc comment in the same commit. Without the doc, `cargo build` fails — there is no way to land a Phase N PR that adds an undocumented `pub` item.

State this as a phase precondition: **every `pub` item ships with rustdoc as a phase precondition.**

### One-line PR-checklist bullet (paste into each phase doc)

```
- [ ] Every new `pub` item in `tui_pane` has a rustdoc comment (`missing_docs = "deny"` is workspace-wide).
```

This bullet belongs in the checklist for Phases 2, 3, 4, 5, 6, 7, 8, 9, 10, 11 — every phase that touches `tui_pane`'s public surface. (Phase 1 itself only adds `lib.rs` with a crate-level doc comment, so the bullet is degenerate but worth keeping for uniformity.)
