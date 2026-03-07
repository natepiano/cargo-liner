# cargo-vischeck

`cargo-vischeck` provides the `cargo vischeck` subcommand for enforcing a stricter Rust visibility style across a crate or workspace.

## V1 policy

Hard errors:

- `pub(crate)` is forbidden
- `pub(in crate::...)` is forbidden
- `pub mod` requires an explicit allowlist entry

Warnings:

- bare `pub` in a nested child file where the parent module is private and does not publicly
  re-export the item

This is intentionally a heuristic tool, not a full compiler-resolved truth engine.

## Config

The tool looks for `vischeck.toml` at the target root.

```toml
[visibility]
allow_pub_mod = [
  "mcp/src/brp_tools/tools/mod.rs",
]
allow_pub_items = [
  "src/example/private_child.rs::SomeIntentionalFacadeItem",
]
```

`visibility_audit.toml` and `.visibility-audit.toml` are also accepted as legacy fallback names.

## Usage

```bash
cargo vischeck
cargo vischeck --fail-on-warn
cargo vischeck --json
cargo vischeck --manifest-path path/to/Cargo.toml
```

Behavior:

- run at a workspace root: audit all workspace members
- run in a member crate directory: audit just that package
- pass `--manifest-path` to choose an explicit crate/workspace root

## Intended workflow

Use this as a migration aid and CI guard:

1. fail immediately on forbidden visibility forms
2. review suspicious bare `pub`
3. compare heuristic findings against manual review
4. keep repo-specific exceptions small and explicit

## Diagnostic Reference

<a id="forbidden-pub-crate"></a>
### Forbidden `pub(crate)`

`pub(crate)` is forbidden by this tool's default policy.

Use it when:
- never by default

Prefer:
- private items when they are local implementation details
- `pub(super)` when the parent module owns the boundary
- moving the item to a better common parent when `pub(super)` is too narrow

Example:

```rust
// Bad
pub(crate) fn helper() {}

// Better
pub(super) fn helper() {}
```

<a id="forbidden-pub-in-crate"></a>
### Forbidden `pub(in crate::...)`

`pub(in crate::...)` is treated as a design-review signal, not a normal visibility tool.

Prefer:
- `pub(super)` when the current module shape is already correct
- moving the item to the nearest common parent as its own file

Example:

```rust
// Bad
pub(in crate::feature::subtree) fn helper() {}

// Better
pub(super) fn helper() {}
```

<a id="review-pub-mod"></a>
### Review `pub mod`

`pub mod` requires explicit review or allowlisting.

Keep it only when:
- the module path itself is intentionally part of the API
- macro or code-generation constraints make it a deliberate exception

Example:

```rust
// Review required
pub mod tools;
```

<a id="suspicious-bare-pub"></a>
### Suspicious bare `pub`

This warning means:
- the item is `pub`
- it lives in a non-root child module
- its parent module is private
- the parent does not publicly re-export it
- it also appears unused outside its defining file

This is heuristic, not proof. It is meant to surface likely overexposed items for review.

Example:

```rust
// private parent module
mod support;

// support/helpers.rs
pub struct Helper;
```

Possible resolutions:
- make the item private
- change it to `pub(super)`
- re-export it intentionally from the parent facade
- move it to a better common parent if it is truly shared
