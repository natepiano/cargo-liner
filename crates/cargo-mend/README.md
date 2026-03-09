# cargo-mend

`cargo-mend` provides the `cargo mend` subcommand for enforcing a stricter Rust
visibility style across a crate or workspace.

The tool is meant for codebases that want visibility to describe real module boundaries.

## Guiding Principle

The goal is that you should be able to read a Rust file in place and understand what each item's
visibility is trying to say.

In practice, that means:

- if you see `pub` in a leaf module, it should suggest that the item is part of that module's
  intended API surface
- if an item is only meant for its parent module or peer modules under the same parent,
  `pub(super)` should say that directly
- if you are in a top-level private module, plain `pub` can still be the right way to mark that
  module's crate-internal boundary API
- if an item is only local implementation detail, it should stay private

The more the code says this directly, the less a reader has to reconstruct the real boundary by
mentally walking the whole module tree.

That is the design pressure behind this tool. It tries to catch places where the written
visibility is broader, vaguer, or more global than the code relationship really is.

In practice, that usually means:

- if an item is only meant for its parent module in a nested private subtree, use `pub(super)`
- if an item lives in a top-level private module and is part of that module's crate-internal API,
  plain `pub` may be correct
- if an item is only local implementation detail, keep it private
- if an item seems to need a deeply nested visibility like `pub(in crate::feature::subtree)`,
  the module tree may be wrong
- if an item is marked `pub` but is not actually used outside its intended module boundary, that
  is probably a design smell

## V1 policy

Hard errors:

- `pub(crate)` is forbidden in binaries and in nested modules
- library crates may use `pub(crate)` at the crate root when the intent is to keep an item
  crate-internal rather than part of the external library API
- top-level private modules in library crates may also use `pub(crate)` when the intent is to keep
  an item crate-internal and prevent accidental exposure through the public library boundary
- `pub(in crate::...)` is forbidden
- `pub mod` requires an explicit allowlist entry

Warnings:

- `pub` in a nested child file where compiler analysis shows the item should probably be
  narrower than `pub`
- parent module `pub use *` re-exports that should be explicit

If you are new to Rust visibility, the important idea is this:

- `pub` does not automatically make an item part of the crate's real outward API
- every parent module on the path also has to be visible
- if a parent module is private, a child item can be written as `pub` and still not actually be
  reachable from outside the crate

## Config

The tool looks for `mend.toml` at the target root.

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
cargo mend
cargo mend --fail-on-warn
cargo mend --fix
cargo mend --json
cargo mend --manifest-path path/to/Cargo.toml
```

Behavior:

- run it at a workspace root to audit all workspace members
- run it in a member crate directory to audit just that package
- pass `--manifest-path` to choose an explicit crate or workspace root
- `--fix` only rewrites the import-shortening cases that `cargo-mend` can prove are safe
- if a `--fix` run would leave the crate failing `cargo check`, `cargo-mend` restores the
  original files automatically
- if there is nothing fixable, `cargo-mend` says so after the report summary

## Toolchain Compatibility

| rustc  | cargo-mend |
|--------|----------------|
| 1.93.1 | 0.1.0          |

- `cargo mend` runs through a rustc workspace wrapper
- visibility checks use compiler data after macro expansion and analysis
- after a Rust toolchain update, rerun `cargo mend` on a known repo and check this section
  first if results regress

## Intended workflow

Use this as a migration aid and CI guard:

1. fail immediately on forbidden visibility forms
2. review suspicious `pub`
3. let `cargo mend --fix` rewrite the straightforward local-import paths it knows how to fix
4. keep repo-specific exceptions small and explicit

The usual review flow is:

1. ask whether the item is truly part of the module's API
2. if not, try private or `pub(super)` in a nested module
3. if the item lives in a top-level private module, plain `pub` may already be the correct
   crate-internal boundary
4. if `pub(super)` is too narrow, move the item to a better common parent
5. only keep broader visibility when the module structure genuinely requires it

## Diagnostic Reference

<a id="forbidden-pub-crate"></a>
### Forbidden `pub(crate)`

`pub(crate)` is broad enough to be easy to reach for, but in many codebases it weakens module
boundaries more than intended.

This tool treats it as forbidden in binaries and in nested modules.

There is one narrow exception:

- at the crate root of a library crate, when the item should stay crate-internal and not become
  part of the external library API
- in a library crate
- inside a top-level private module
- when the point is to keep something crate-internal and prevent accidental leakage through the
  public library boundary

Prefer:
- private items when they are local implementation details
- `pub(super)` when the parent module owns the boundary
- moving the item to a better common parent when `pub(super)` is too narrow

In this example, there is a parent module named `feature`, and `helpers.rs` exists only to support
that parent module.

The question is whether the helper should be available to the whole crate, or just to `feature`.

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

One exception is the crate root of a library crate, for example:

```rust
// src/lib.rs
pub(crate) type InternalDrawPhase = ();
```

That can be acceptable when the intent is:

- usable anywhere inside the crate
- but not part of the external library API

Another exception is a library crate with a top-level private module, for example:

```rust
// src/lib.rs
mod internals;

// src/internals.rs
pub(crate) fn helper() {}
```

That can be acceptable when the intent is:

- usable anywhere inside the crate
- but never part of the external library API

<a id="forbidden-pub-in-crate"></a>
### Forbidden `pub(in crate::...)`

`pub(in crate::...)` often means the item lives too deep in the module tree.

This tool treats it as a design-review signal, not a normal visibility tool.

Prefer:
- `pub(super)` when the current module shape is already correct
- moving the item to the nearest common parent as its own file

In this example, a helper lives under `src/feature/deep/`, but the desired sharing boundary is
somewhere higher up than that file.

The example is showing what it looks like when the visibility path has to reach outward to describe
the real boundary.

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

In this example, the code is at the crate root.

The important thing to notice is that `pub mod` does not just declare a child module. It also
publishes that module path as part of the crate API.

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

<a id="suspicious-pub"></a>
### Suspicious `pub`

This warning is about a Rust visibility trap in nested private modules:

- an item can be written as `pub`
- but still be broader than the boundary that file actually lives under

That happens when one of its parent modules is private and the file is not itself sitting at the
top-level private boundary.

In this example, there is a private parent module named `support`, and `helpers.rs` lives under
that private boundary.

The code in `helpers.rs` marks `Helper` as `pub`, but the example is specifically showing a case
where that still does not make `Helper` part of the crate's public API.

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

That is why this tool warns here. In a nested private module like `support/helpers.rs`, the
declared visibility (`pub`) is broader than the boundary that file is actually participating in.

Possible resolutions:
- make the item private
- change it to `pub(super)`
- move it to a better common parent if it is truly shared

There is one important allowed case.

If the parent boundary module is intentionally acting as a facade, it may re-export the child item.
That boundary can be either:

- a `mod.rs` file
- or an ordinary file module like `markdown_file.rs`

For example:

```rust
// src/private_parent/mod.rs
mod child;
pub use child::Helper;
```

If code outside `private_parent` actually uses `private_parent::Helper`, then keeping `Helper`
as `pub` in `child.rs` is intentional and this warning should not fire.

If the parent boundary module re-exports `Helper` but nothing outside the parent subtree ever uses that
re-export, then the child `pub` is still broader than the boundary the code is actually using.
In that case this warning should still appear.

In practice, Rust itself will often warn on the parent `mod.rs` too:

- `warning: unused import: ...`

`cargo-mend` does not duplicate that parent warning. Instead, it warns on the child item and points
back to the compiler's `unused import` warning so you can see the pair together:

- the compiler warns that the parent `pub use` is stale
- `cargo-mend` warns that the child item is still broader than needed

That is also the case that `cargo mend --fix-pub-use` is designed to repair.

For example:

```rust
// src/support/helpers.rs
pub(super) struct Helper;
```

Now the code says what it actually means: `Helper` is shared with its parent module, not with the
outside world.

This warning does not apply the same way to a top-level private module. At the top level, plain
`pub` can still be the right way to say "this belongs to this module's crate-internal API."

Parent facade re-exports should also be explicit.

If a parent boundary module does this:

```rust
pub use child::*;
```

`cargo-mend` treats that as a separate problem. Use explicit re-exports instead so the parent
facade states exactly which child items it is exporting.

<a id="wildcard-parent-pub-use"></a>
### Wildcard parent `pub use`

This warning is about parent facade modules that re-export everything from a child with `*`.

That shape makes the boundary harder to read because the parent module no longer says what it is
actually exporting.

Prefer:

```rust
pub use child::{Helper, OtherHelper};
```

instead of:

```rust
pub use child::*;
```

<a id="shorten-local-crate-import"></a>
### Shorten local crate import

This warning is about import paths that are technically correct, but more global than the code
relationship actually is.

In this example, there are two peer modules under the same private parent module:

- `cargo_detector.rs`
- `process.rs`

The code in `process.rs` wants to import `TargetType` from its peer module `cargo_detector.rs`.

Example:

```rust
// src/app_tools/support/process.rs
use crate::app_tools::support::cargo_detector::TargetType;
```

If you are reading `process.rs`, that import path makes `TargetType` look more global than it
really is.

But the real relationship is local:

- `process.rs` and `cargo_detector.rs` are peers under `support`
- the import is not crossing to a different domain
- the shorter local-relative path is clearer

A better import is:

```rust
use super::cargo_detector::TargetType;
```

`cargo mend --fix` can rewrite these straightforward cases automatically.

Today, that auto-fix mode is intentionally narrow:

- it only rewrites local import-shortening cases
- it preserves the original import visibility (`use`, `pub use`, `pub(crate) use`, and so on)
- it rolls the edits back automatically if the follow-up `cargo check` fails
