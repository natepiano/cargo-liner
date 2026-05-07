# CI / tooling audit for the workspace conversion

Audit assumes the Phase 1 conversion described in the user task: root `Cargo.toml` becomes both `[package]` (the binary) **and** `[workspace]`, with a sibling library at `tui_pane/`. There is no relocation of the binary into a subdirectory; `cargo install --path .` keeps working unchanged.

(Note: this assumed layout differs from `docs/workspace.md`, which proposes `crates/cargo-port/` + `crates/pane_kit/`. This spec follows the task description, not that doc.)

## 1. Inventory

### `.github/workflows/ci.yml`

| Line | Command | Today |
|---|---|---|
| 39 | `cargo +nightly fmt -- --check` | rustfmt format check (no `--all`) |
| 53 | `taplo fmt --check` | TOML format check; `taplo.toml:1` sets `include = ["**/*.toml"]` |
| 82 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | already passes `--workspace` |
| 110 | `cargo build --release --all-features --workspace --examples` | already passes `--workspace --examples` |
| 149 | `cargo nextest run --all-features --workspace --tests` | already passes `--workspace --tests` |
| 184 | `cargo install cargo-mend` | installs the linter binary |
| 189 | `cargo mend --fail-on-warn` | runs from repo root with no scope flag |

### `.cargo/config.toml`

Does not exist.

### Root `*.yml`

Only `.github/workflows/ci.yml`. No top-level GitHub Actions, no `bacon.toml`, no `release.yml`.

### `Justfile` / `Makefile` / `pre-commit`

None present. No `.pre-commit-config.yaml`. No git hooks installed (`.git/hooks/` empty).

### Repo-level config

- `taplo.toml` — `include = ["**/*.toml"]` already covers a workspace tree (Cargo.toml of any new member is matched).
- `rustfmt.toml` — single root file; rustfmt picks it up for every member automatically.
- `Cargo.toml` — currently `[package]` only. Phase 1 adds `[workspace]` alongside.

### `~/.claude/CLAUDE.md` and `~/rust/CLAUDE.md`

| File:line | Command | Today |
|---|---|---|
| `~/rust/CLAUDE.md:17` | `cargo build && cargo +nightly fmt` | post-edit reminder |
| `~/.claude/projects/-Users-natemccoy-rust-cargo-port/memory/feedback_cargo_install.md` | `cargo install --path .` | end-of-phase install |
| `~/.claude/projects/.../feedback_cargo_nextest.md` | `cargo nextest run` | test runner default |

(No per-repo `CLAUDE.md` exists in `cargo-port-api-fix/`.)

### Nightly scripts under `~/.claude/scripts/nightly/`

`nightly-rust-clean-build.sh`:

| Line | Command |
|---|---|
| 118 | `cargo clean --manifest-path "$project_dir/Cargo.toml"` |
| 124 | `cargo build --workspace --examples --manifest-path "$project_dir/Cargo.toml"` |
| 130 | `cargo mend --workspace --all-targets --manifest-path "$project_dir/Cargo.toml"` |

`style-eval-all.sh`: no direct cargo invocation. Walks `~/rust/*/` for `Cargo.toml` (line 148), then hands the project root to a Claude/Codex agent.

`style-fix-worktrees.sh` (prompt body emitted to the style agent, lines 532–568):

| Step | Command |
|---|---|
| 4 preview | `cargo mend $cargo_scope_flag --all-targets --manifest-path $worktree_dir/Cargo.toml` |
| 4 fix | `cargo mend $cargo_scope_flag --all-targets --fix --manifest-path $worktree_dir/Cargo.toml` |
| 5a | `cargo clippy $cargo_scope_flag --all-targets --all-features --manifest-path $worktree_dir/Cargo.toml -- -D warnings` |
| 5b | `cargo clippy --fix $cargo_scope_flag --all-targets --all-features --allow-dirty --manifest-path $worktree_dir/Cargo.toml -- -D warnings` |
| 5c | re-run 5a |
| 6 | `CARGO_MEND_SKIP_NETWORK_TESTS=1 cargo nextest run $cargo_scope_flag --manifest-path $worktree_dir/Cargo.toml` |
| 8 | `cargo +nightly fmt $cargo_scope_flag --manifest-path $worktree_dir/Cargo.toml` |

`$cargo_scope_flag` is `--workspace` for standalone projects (line 424) and `-p $pkg` for `[workspace_members]` entries from `nightly-rust.conf` (line 421). `cargo-port` would resolve to the standalone branch and get `--workspace`.

### Hook scripts under `~/.claude/scripts/hooks/`

`post-tool-use-cargo-check.sh`:

| Line | Command |
|---|---|
| 109 | `cd "$CARGO_DIR" && cargo check 2>&1` |
| 113 | `cargo check 2>&1` |
| 187 | `cargo +nightly fmt >/dev/null 2>&1` (auto-fmt allowlist) |

Runs after every Edit/Write tool call. Walks up the tree to find the nearest `Cargo.toml`, then runs `cargo check` from that directory.

## 2. Per-invocation decision

| Invocation | Today's flag | After workspace conversion | Why |
|---|---|---|---|
| `ci.yml:39` `cargo +nightly fmt -- --check` | none | **Workspace.** Add `--all`: `cargo +nightly fmt --all -- --check` | rustfmt with no flag only checks the current package; with two members, the `tui_pane` files are silently skipped. `--all` walks workspace members. |
| `ci.yml:53` `taplo fmt --check` | `include = ["**/*.toml"]` in `taplo.toml` | **Workspace** (no change). | The glob already matches `tui_pane/Cargo.toml` and `tui_pane/**/*.toml`. |
| `ci.yml:82` `cargo clippy --workspace ...` | `--workspace` | **Workspace** (no change). | Already correct. |
| `ci.yml:110` `cargo build --release ... --workspace --examples` | `--workspace` | **Workspace** (no change). | Already correct. |
| `ci.yml:149` `cargo nextest run ... --workspace --tests` | `--workspace` | **Workspace** (no change). | Already correct. |
| `ci.yml:189` `cargo mend --fail-on-warn` | none | **Workspace.** Verify `cargo mend` defaults to walking the workspace from the root manifest; if not, add `--workspace`: `cargo mend --workspace --all-targets --fail-on-warn`. | Same risk `docs/workspace.md` Risk 5 already flagged. The nightly script (line 130) already passes `--workspace --all-targets`; CI should match. |
| `~/rust/CLAUDE.md:17` `cargo build && cargo +nightly fmt` | none | **Workspace.** Update guidance to `cargo build --workspace && cargo +nightly fmt --all`. | Same root-package-only trap as the CI fmt entry. |
| `feedback_cargo_install.md` `cargo install --path .` | n/a | **Binary-only** by construction. **Keeps working unchanged.** | Root `Cargo.toml` is `[package]` + `[workspace]`. `cargo install --path .` reads the `[package]` and installs the binary. No flag change needed. See section 3. |
| `feedback_cargo_nextest.md` `cargo nextest run` | none | **Either** depending on intent. For full-repo CI parity prefer `cargo nextest run --workspace`; for a single-member iteration loop use `-p cargo-port` or `-p tui_pane`. Update memory to say so. | Default `cargo nextest run` from the workspace root tests only the root package. |
| `nightly-rust-clean-build.sh:118` `cargo clean` | `--manifest-path` | **Workspace** (no change). | `cargo clean` is workspace-aware. |
| `nightly-rust-clean-build.sh:124` `cargo build --workspace --examples` | `--workspace` | **Workspace** (no change). | Already correct. |
| `nightly-rust-clean-build.sh:130` `cargo mend --workspace --all-targets` | `--workspace` | **Workspace** (no change). | Already correct. |
| `style-fix-worktrees.sh` agent prompt | `$cargo_scope_flag` resolves to `--workspace` for standalone projects | **Workspace** (no change). | `cargo-port` is in the standalone branch, so the scope flag is already `--workspace`. |
| `post-tool-use-cargo-check.sh:109/113` `cargo check` | none | **Workspace.** Add `--workspace` so edits inside `tui_pane/` aren't silently unchecked when the hook resolves to the root `Cargo.toml`. | The hook walks up to the nearest `Cargo.toml`. After conversion, edits inside `tui_pane/src/` resolve to `tui_pane/Cargo.toml` (member check, fine). Edits inside `src/` resolve to root `Cargo.toml`, which is `[package] + [workspace]` — without `--workspace`, only the binary is checked. |
| `post-tool-use-cargo-check.sh:187` `cargo +nightly fmt` (auto-fix) | none | **Workspace.** Switch to `cargo +nightly fmt --all`. | Same root-package-only trap. |

No invocation in this audit needs **Both** (run twice, separately). The `cargo install` case is binary-only by construction and runs once; everything else runs once with `--workspace` / `--all`.

## 3. `feedback_cargo_install.md`

Current entry: "After successful changes, run `cargo install --path .`".

After Phase 1 the root `Cargo.toml` is both `[package]` (cargo-port binary) and `[workspace]`. `cargo install --path .` reads the `[package]` table at the given path, builds it inside the workspace context, and installs the resulting binary. **It still works unchanged.** No memory update required.

If at any point the binary moves out of the root (e.g. into `cargo-port/` as a sibling of `tui_pane/`), the entry would need to become `cargo install --path cargo-port` — but that is a different layout from the one this audit assumes.

## 4. Nightly scripts

`nightly-rust-clean-build.sh` is workspace-safe today: every cargo line either passes `--workspace` (lines 124, 130) or is workspace-aware by default (line 118). The directory walk at line 73 (`for project_dir in "$RUST_DIR"/*/`) finds `~/rust/cargo-port-api-fix/` by checking for the presence of `Cargo.toml` (line 102) — Phase 1 keeps `Cargo.toml` at the root, so the project remains discoverable. **No risk.**

`style-eval-all.sh` and `style-fix-worktrees.sh`:

- Both walk `~/rust/*/` for `Cargo.toml` (`style-eval-all.sh:148`, `style-fix-worktrees.sh:168`). Same as above: root `Cargo.toml` is preserved, so cargo-port keeps appearing as a standalone project, not as a workspace_member entry in `nightly-rust.conf`.
- The fix prompt sets `cargo_scope_flag="--workspace"` for the standalone branch (`style-fix-worktrees.sh:424`). Every cargo command in the agent prompt already gets `--workspace` for a project like cargo-port. **No risk** unless the user later moves cargo-port under `bevy_hana`-style `[workspace_members]`, in which case the scope would switch to `-p cargo-port` and `tui_pane` edits inside the same fix run would not be covered. That is not the current layout.

One latent issue worth naming: `style-eval-all.sh:228–232` walks `$project_root/src`, `examples`, `tests`, and `Cargo.toml` for staleness detection. After conversion, edits to `tui_pane/src/**` are caught only if the script is re-run (the `find` above does not include `tui_pane/`). Low cost: at worst one nightly run uses a stale EVALUATION.md. Flag, don't fix.

## 5. MSRV / `rust-toolchain.toml`

No `rust-toolchain` or `rust-toolchain.toml` at the repo root. CI pins via `dtolnay/rust-toolchain@master` with `toolchain: stable` (`ci.yml:64, 93, 121, 166`) and `nightly` for fmt (`ci.yml:35`). No MSRV declared in `Cargo.toml`.

After conversion: a single root `rust-toolchain.toml` (if added) would apply to every workspace member automatically. Nothing to do unless an MSRV is introduced; if one is, place it at the root and it covers both crates.

## 6. Suggested workflow

Land a CI-only commit **before** Phase 1 that adds `--all` to the rustfmt invocation (`ci.yml:39`) and the auto-fmt step in `post-tool-use-cargo-check.sh:187`, plus `--workspace` on `cargo check` (`post-tool-use-cargo-check.sh:109/113`) and `cargo mend` (`ci.yml:189`). These edits are no-ops on the current single-crate layout (the workspace-by-default flags collapse to the same target), so green CI on `main` proves the flags are wired correctly before the workspace appears. Then ship Phase 1 as a single commit that converts the manifest and adds `tui_pane/Cargo.toml` + `tui_pane/src/lib.rs`; CI is already prepared and the only risk left is the one `docs/workspace.md` Risk 5 already names — `cargo mend`'s workspace behavior at the root manifest. Update `~/rust/CLAUDE.md:17` and the `feedback_cargo_nextest.md` memory entry in the same Phase 1 commit.

## Files referenced

- `/Users/natemccoy/rust/cargo-port-api-fix/.github/workflows/ci.yml`
- `/Users/natemccoy/rust/cargo-port-api-fix/Cargo.toml`
- `/Users/natemccoy/rust/cargo-port-api-fix/taplo.toml`
- `/Users/natemccoy/rust/cargo-port-api-fix/rustfmt.toml`
- `/Users/natemccoy/rust/cargo-port-api-fix/docs/workspace.md`
- `/Users/natemccoy/rust/CLAUDE.md`
- `/Users/natemccoy/.claude/scripts/nightly/nightly-rust-clean-build.sh`
- `/Users/natemccoy/.claude/scripts/nightly/nightly-rust.conf`
- `/Users/natemccoy/.claude/scripts/nightly/style-eval-all.sh`
- `/Users/natemccoy/.claude/scripts/nightly/style-fix-worktrees.sh`
- `/Users/natemccoy/.claude/scripts/hooks/post-tool-use-cargo-check.sh`
- `/Users/natemccoy/.claude/projects/-Users-natemccoy-rust-cargo-port/memory/feedback_cargo_install.md`
- `/Users/natemccoy/.claude/projects/-Users-natemccoy-rust-cargo-port/memory/feedback_cargo_nextest.md`
