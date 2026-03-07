# cargo-vischeck

`cargo-vischeck` provides the `cargo vischeck` subcommand for enforcing a stricter Rust
visibility style across a crate or workspace.

The tool is meant for codebases that want visibility to describe real module boundaries.

In practice, that usually means:

- if an item is only meant for its parent module, use `pub(super)`
- if an item is only local implementation detail, keep it private
- if an item seems to need a deeply nested visibility like `pub(in crate::feature::subtree)`,
  the module tree may be wrong
- if an item is marked `pub` but cannot actually be reached from the crate's public API, that is
  probably a design smell

## V1 policy

Hard errors:

- `pub(crate)` is forbidden
- `pub(in crate::...)` is forbidden
- `pub mod` requires an explicit allowlist entry

Warnings:

- bare `pub` in a nested child file where compiler analysis shows the item's effective public API
  is narrower than `pub`

If you are new to Rust visibility, the important idea is this:

- a bare `pub` does not automatically make an item part of the crate's real public API
- every parent module on the path also has to be visible
- if a parent module is private, a child item can be written as `pub` and still not actually be
  reachable from outside the crate

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

Use the allowlists sparingly. The default assumption should be that the code shape is wrong before
the policy is wrong.

## Usage

```bash
cargo vischeck
cargo vischeck --fail-on-warn
cargo vischeck --fix
cargo vischeck --json
cargo vischeck --manifest-path path/to/Cargo.toml
```

Behavior:

- run it at a workspace root to audit all workspace members
- run it in a member crate directory to audit just that package
- pass `--manifest-path` to choose an explicit crate or workspace root

## Toolchain Compatibility

| rustc  | cargo-vischeck |
|--------|----------------|
| 1.93.1 | 0.1.0          |

- `cargo vischeck` runs through a rustc workspace wrapper
- visibility checks use compiler data after macro expansion and analysis
- after a Rust toolchain update, rerun `cargo vischeck` on a known repo and check this section
  first if results regress

## Intended workflow

Use this as a migration aid and CI guard:

1. fail immediately on forbidden visibility forms
2. review suspicious bare `pub`
3. let `cargo vischeck --fix` rewrite straightforward local-import paths
4. keep repo-specific exceptions small and explicit

The usual review flow is:

1. ask whether the item is truly part of the module's API
2. if not, try private or `pub(super)`
3. if `pub(super)` is too narrow, move the item to a better common parent
4. only keep broader visibility when the module structure genuinely requires it

## Diagnostic Reference

<a id="forbidden-pub-crate"></a>
### Forbidden `pub(crate)`

`pub(crate)` is broad enough to be easy to reach for, but in many codebases it weakens module
boundaries more than intended.

This tool treats it as forbidden by default.

Prefer:
- private items when they are local implementation details
- `pub(super)` when the parent module owns the boundary
- moving the item to a better common parent when `pub(super)` is too narrow

Example:

```rust
// src/feature/mod.rs
mod helpers;

// src/feature/helpers.rs
pub(crate) fn helper() {}
```

At first glance, `helper` looks reasonable: the whole crate can use it.

But that is exactly the problem. The helper now ignores the `feature` module boundary.

A better shape is:

```rust
// src/feature/helpers.rs
pub(super) fn helper() {}
```

Now `helper` is available to `feature`, but not to unrelated parts of the crate.

<a id="forbidden-pub-in-crate"></a>
### Forbidden `pub(in crate::...)`

`pub(in crate::...)` often means the item lives too deep in the module tree.

This tool treats it as a design-review signal, not a normal visibility tool.

Prefer:
- `pub(super)` when the current module shape is already correct
- moving the item to the nearest common parent as its own file

Example:

```rust
// src/feature/deep/helper.rs
pub(in crate::feature::subtree) fn helper() {}
```

This tells you the visibility boundary is somewhere far away from the item.

That usually means one of two things:

- the item should just be `pub(super)`
- the item should move upward so the right boundary is local and obvious

A better shape is usually either:

```rust
// src/feature/deep/helper.rs
pub(super) fn helper() {}
```

or:

```rust
// src/feature/helper.rs
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
// src/lib.rs
pub mod tools;
```

`pub mod` does two things at once:

- it declares a child module
- it makes that module path part of the public API

That is sometimes exactly what you want. It is also easy to do by accident.

This tool asks you to review that choice explicitly instead of letting it slip in unnoticed.

<a id="suspicious-bare-pub"></a>
### Suspicious bare `pub`

This warning is about a Rust visibility trap:

- an item can be written as `pub`
- but still fail to be part of the crate's real public API

That happens when one of its parent modules is private.

Example:

```rust
// src/lib.rs
mod support;

// src/support/mod.rs
mod helpers;

// src/support/helpers.rs
pub struct Helper;
```

If you are new to Rust, it is easy to read `pub struct Helper;` and think:

- "`Helper` is public, so other crates can use it"

But Rust does not work that way. The full path must be public too.

In this example:

- `Helper` is marked `pub`
- but `support` is private
- so `Helper` is not reachable from outside the crate

That is why this tool warns here. The declared visibility (`pub`) is broader than the item's real
reachable API.

Possible resolutions:
- make the item private
- change it to `pub(super)`
- move it to a better common parent if it is truly shared

For example:

```rust
// src/support/helpers.rs
pub(super) struct Helper;
```

Now the code says what it actually means: `Helper` is shared with its parent module, not with the
outside world.

<a id="shorten-local-crate-import"></a>
### Shorten local crate import

This warning is about import paths that are technically correct, but more global than the code
relationship actually is.

Example:

```rust
// src/app_tools/support/process.rs
use crate::app_tools::support::cargo_detector::TargetType;
```

If you are reading `process.rs`, that path makes `TargetType` look global.

But the real relationship is local:

- `process.rs` and `cargo_detector.rs` are peers under `support`
- the import is not crossing to a different domain
- the shorter local-relative path is clearer

A better import is:

```rust
use super::cargo_detector::TargetType;
```

`cargo vischeck --fix` can rewrite these straightforward cases automatically.
