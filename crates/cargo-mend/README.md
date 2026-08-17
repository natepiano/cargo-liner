# cargo-mend

[![Crates.io](https://img.shields.io/crates/v/cargo-mend.svg)](https://crates.io/crates/cargo-mend)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/natepiano/cargo-mend#license)
[![Crates.io](https://img.shields.io/crates/d/cargo-mend.svg)](https://crates.io/crates/cargo-mend)
[![CI](https://github.com/natepiano/cargo-mend/workflows/CI/badge.svg)](https://github.com/natepiano/cargo-mend/actions)

**Warning:** This project is pre-1.0 and under active development. Diagnostics, config, and CLI
flags may change between releases. Fix modes modify source files in place; Mend-managed fixes roll
back on `cargo check` failure, but always review the diff before committing.

`cargo-mend` provides the `cargo mend` subcommand for enforcing an opinionated Rust
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
- if an item in a top-level private module is shared across crate-root branches, use `pub(crate)`;
  if it stays inside its own subtree, narrow it further
- if the crate root re-exports the item via `pub use`, the source must be bare `pub` (E0364)
- if an item is only local implementation detail, keep it private
- if an item seems to need a deeply nested visibility like `pub(in crate::feature::subtree)` and
  that path is not the narrowest boundary required by callers, signatures, or a parent facade, the
  module tree is probably wrong; `cargo mend` rejects the form so the structural problem remains
  visible

`cargo mend` flags places where the written visibility is broader, vaguer, or more global than
the code relationship actually is.

## Mend Policy

Hard errors:

- `pub(in ...)` is forbidden unless a crate-rooted path exactly matches the boundary required by a
  parent facade, callers, or signature reach and `pub_in_path` permits it
- `pub mod` requires an explicit allowlist entry, except for a crate-root prelude

Warnings:

- `pub(crate)` when the complete reach analysis proves that a narrower visibility is sufficient;
  it is accepted when the narrowest proven boundary is the crate root
- `pub` that exceeds the reach required by callers or signatures
- dead field visibility and parent facades that deserve review
- import forms that obscure local module relationships
- parent-module `pub use *` re-exports that should be explicit

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
# exact caller, signature, or facade boundaries: forbidden | permitted | required
pub_in_path = "required"
```

Use the allowlists sparingly. The default assumption should be that the code structure is wrong before
the policy is wrong.

`pub_in_path` controls exact crate-rooted `pub(in crate::path)` boundaries on declarations and
fields:

- `forbidden` rejects them
- `permitted` accepts them when they exactly match a caller, signature, or parent-facade boundary
- `required` (the default) also asks `suspicious_pub` to recommend that boundary instead of bare
  `pub`, and `cargo mend --fix` rewrites the declaration or field

Other restricted spellings, such as `pub(in crate)`, relative `pub(in super::...)` paths, and
boundaries that do not match the proven reach remain diagnostics.

A crate-root `pub mod prelude;` is exempt from `review_pub_mod` by default — a prelude is an
intentional public surface, so it does not need an `allow_pub_mod` entry. Nested `pub mod prelude;`
and any other crate-root `pub mod` are still reviewed. Set `allow_prelude_pub_mod = false` under
`[visibility]` to review crate-root preludes too.

### Global config

On first run, cargo-mend writes a global config to your platform config directory:

- Linux: `$XDG_CONFIG_HOME/cargo-mend/config.toml`, falling back to
  `~/.config/cargo-mend/config.toml`
- macOS: `~/Library/Application Support/cargo-mend/config.toml`

It records the on/off default for every diagnostic and the fallback defaults for `[visibility]`:

```toml
[diagnostics]
review_pub_mod = true
# ... one line per diagnostic

[visibility]
# default-on; set false to review crate-root prelude modules too
allow_prelude_pub_mod = true
# required (default) reviews pub; permitted also accepts it; forbidden rejects pub(in ...)
pub_in_path = "required"
```

For `allow_prelude_pub_mod` and `pub_in_path`, a project `mend.toml` value overrides the global
value, and the global value overrides the compiled-in default. The `allow_pub_mod` and
`allow_pub_items` path lists are project-only. `[diagnostics]` entries use project-over-global
precedence. On every run, cargo-mend adds any keys missing from an existing global config
(preserving your comments and values), so the file stays complete as new options are introduced.

## Installation

`cargo-mend` uses `#![feature(rustc_private)]` to access compiler internals for visibility
analysis after macro expansion. This is a permanently unstable feature — it is how tools like
clippy and miri access the compiler, but it means the compiler's internal crates have no
stability guarantee and `cargo-mend` is sensitive to the exact rustc version used to build it.

### Compatibility

`cargo-mend` links against the compiler internals of the toolchain that builds it, so releases are
tied to a specific rustc version. Build and run each release with its matching toolchain:

| `cargo-mend` | rustc |
|--------------|-------|
| 0.17+        | 1.97  |
| 0.16.x       | 1.96  |

Install the `rustc-dev` component, then install `cargo-mend` with the stable toolchain plus
`RUSTC_BOOTSTRAP=1`. Nightly-built binaries can fail against stable projects with `E0514`
because `cargo-mend` links against `rustc_driver`.

```bash
rustup component add rustc-dev
RUSTC_BOOTSTRAP=1 cargo +stable install --path .
RUSTC_BOOTSTRAP=1 cargo +stable install cargo-mend --version <VERSION>
```

## Usage

```bash
cargo mend
cargo mend --fail-on-warn
cargo mend --dry-run
cargo mend --fix
cargo mend --fix-pub-use
cargo mend --fix-compiler
cargo mend --fix-all
cargo mend --json
cargo mend --workspace
cargo mend --package my-crate
cargo mend --manifest-path path/to/Cargo.toml
cargo mend path/to/project
```

Behavior:

- run it at a workspace root to audit all workspace members
- run it in a member crate directory to audit just that package
- pass `--manifest-path` to choose an explicit crate or workspace root
- `--fix` applies Mend's proven import and visibility rewrites
- `--fix-pub-use` repairs stale parent re-exports, while `--fix-compiler` runs `cargo fix`
- `--fix-all` combines all three fix categories
- `--dry-run` previews the requested categories; by itself, it previews all fixes
- warnings leave the exit status successful unless `--fail-on-warn` is set
- if a Mend fix batch would leave the crate failing `cargo check`, cargo-mend restores the original
  files automatically
- if there is nothing fixable, `cargo-mend` says so after the report summary

### Target selection flags are display filters

`--lib`, `--bin <NAME>`, `--example <NAME>`, `--test <NAME>`, `--bench <NAME>`, and
`--all-targets` only narrow **what gets printed**. They do not change what gets analyzed —
mend always compiles every target (lib, bins, tests, examples, benches).

Why: whether a `pub fn` is "really used" depends on the whole crate. If you analyze only the
lib, a function called solely by an integration test or a `#[cfg(test)]` helper looks dead
and mend would suggest narrowing or removing it. That suggestion would break the test build.
By always compiling everything, mend sees the full call graph and gives correct answers.

So `cargo mend --lib` is "show me only the lib-file findings"; the analysis behind those
findings still considered every target.

Caveat: this covers `#[cfg(test)]`, but not code enabled only by non-default features. Cargo-mend
does not currently expose Cargo's feature-selection flags, so those configurations are outside the
analyzed call graph.

## Intended workflow

Use this as a migration aid and CI guard:

1. report visibility forms broader than the proven boundary
2. review suspicious `pub`
3. let `cargo mend --fix` apply the visibility and import rewrites it can prove safe
4. keep repo-specific exceptions small and explicit

The usual review flow is:

1. ask whether the item is truly part of the module's API
2. if all callers are inside the defining module subtree, make it private
3. if callers live in sibling modules, try `pub(super)` in a nested module
4. use `pub(in crate::path)` when callers, signatures, or a parent facade require a wider ancestor
5. use `pub(crate)` when the required common ancestor is the crate root
6. move the item to a better common parent when a restricted annotation would obscure ownership
7. only keep broader visibility when the module structure genuinely requires it

## Diagnostic Reference

<a id="overbroad-pub-crate"></a>
### Overly broad `pub(crate)`

`pub(crate)` lets any module in the crate touch the item, regardless of where the item lives.
In a deep module tree that usually weakens the module boundaries the layout was meant to
enforce.

This rule applies to declarations and struct and union fields. Cargo-mend joins callers and
signature requirements across every selected target and computes their deepest common module. It
accepts `pub(crate)` when that module is the crate root. When the common module is narrower, the
`overbroad_pub_crate` diagnostic reports that the annotation is broader than required.

Otherwise, prefer:

- private items when they are local implementation details
- `pub(super)` when the parent module owns the boundary
- `pub(in crate::path)` when callers, signatures, or a parent facade require a wider ancestor
- `pub(crate)` when uses genuinely span crate-root sibling branches
- moving the item to a better common parent when `pub(super)` is too narrow

`cargo mend --fix` writes each proven result directly: it removes an unnecessary annotation, uses
`pub(super)` for the immediate parent, uses exact `pub(in crate::path)` for a wider ancestor, and
uses `pub(crate)` for the crate root. It applies related declaration changes in one batch before the
validation build, so a trait and its associated type can narrow together without an intermediate
E0446 failure. Field construction, access, destructuring, and `offset_of!` computations contribute
caller evidence for named and positional fields.

When mend gives facade-specific advice, it quotes the facade's `use` spelling only when it can
establish that spelling from the active item graph. A resolved reach alone never licenses quoting a
modifier; otherwise the diagnostic calls it a re-export without naming a modifier.

In this example, `feature` is a parent module and `helpers.rs` exists only to support it. The
question is whether the helper should be available to the whole crate, or just to `feature`.

```rust
// src/feature/mod.rs
mod helpers;

// src/feature/helpers.rs
pub(crate) fn helper() {}
```

When every caller is under `feature`, the crate-wide annotation ignores that module boundary. A
better version is:

```rust
// src/feature/helpers.rs
pub(super) fn helper() {}
```

`helper` is now available to `feature` and nowhere else.

`pub(crate)` remains correct when the item is reached from crate-root sibling branches:

```rust
// src/lib.rs
mod render;
mod text;

// src/render/batching.rs
pub(crate) struct TextRunBatch;
```

If both `render` and `text` reach `TextRunBatch`, their common boundary is the crate root and mend
accepts the annotation.

<a id="forbidden-pub-in-crate"></a>
### Forbidden `pub(in ...)`

Arbitrary `pub(in ...)` annotations are rejected because they can hide an ownership problem behind
a long path. An exact crate-rooted `pub(in crate::path)` is accepted when `path` is the narrowest
boundary required by callers, signatures, or a parent facade. Set `pub_in_path = "permitted"` (the
less strict mode) or `"required"` (the default) to accept that exact boundary.

A parent re-export is one case that requires this form. A `pub(super) use` can put the item's
required reach above the module where it is declared: `pub(super)` at the declaration is then too
narrow to compile, while bare `pub` is wider than required.

```rust
// src/video_plane/plane/camera_panel.rs
pub(in crate::video_plane) fn bind_camera_panel() {}

// src/video_plane/plane/mod.rs
mod camera_panel;
pub(super) use camera_panel::bind_camera_panel;
```

Here the item lives in `video_plane::plane::camera_panel`, while the facade lives in
`video_plane::plane`. The facade's `pub(super) use` must be visible to `video_plane`, but
`pub(super)` on `bind_camera_panel` would only make it visible to `plane`; rustc rejects that wider
re-export with E0364. Bare `pub` compiles, but says more than the relationship requires.
`pub(in crate::video_plane)` states the required reach exactly.

The path is the parent of the module holding the facade, not one level above the item. In this
example it is two levels above `camera_panel`. With chained facades, find the widest facade and use
the parent of the module holding it; the distance can grow beyond two levels. The path names who
can see the item, not who owns it.

Always spell this boundary from `crate::`. `pub(in super::super)` can name the same module, but
forces readers to count levels and changes meaning when the file moves.

Exact-boundary repairs apply to declarations and fields. A `use` line selects its own reach, so
`pub(super)`, `pub(crate)`, and `pub` already express what it needs.

Do not use this form to avoid moving an item. If the required path is long or names a module
unrelated to the item, the item belongs in a different module. It is not a way to widen access:
`pub(in crate::a)` is narrower than `pub(crate)` and `pub`. If it seems necessary to unlock access,
consider whether the intended decision is a `pub(crate) use` facade instead, and state that decision
on the `use` line.

<a id="review-pub-mod"></a>
### Review `pub mod`

`pub mod` is disallowed by default — it publishes the module path as part of the crate's public
API. Override per path via `allow_pub_mod` in `mend.toml` when the public path is intentional
(e.g. macro or codegen constraints).

```rust
// src/lib.rs
pub mod tools;   // module path is now part of the crate's public API
```

<a id="suspicious-pub"></a>
### Suspicious `pub`

A nested private module can declare `pub struct Helper;`, but if any parent module on the path
is private, `Helper` cannot escape the crate — the bare `pub` is broader than the boundary the
file actually participates in.

```rust
// src/lib.rs
mod support;

// src/support/mod.rs
mod helpers;

// src/support/helpers.rs
pub struct Helper;
```

`Helper` is `pub`, but `support` is private, so `Helper` is unreachable from outside the crate.
The declared visibility doesn't match the actual reach.

Resolutions:

- make the item private
- change it to `pub(super)`
- use the exact `pub(in crate::path)` or `pub(crate)` boundary Mend reports when wider access is
  required
- move it to a better common parent if it is genuinely shared across the crate

At logical depth greater than one, `suspicious_pub` examines bare `pub` and accepted crate-rooted
`pub(in crate::...)` declarations. Its parent-facade-boundary suggestion for a bare `pub` appears
only when `pub_in_path = "required"`; accepted `pub(in ...)` declarations are also reviewed for a
boundary that has become stale or no longer matches the facade.

This warning does not fire at a top-level private module — [Narrow `pub` to
`pub(crate)`](#narrow-to-pub-crate) covers that case. At the top level, bare `pub` is only
correct when the crate root re-exports the item via `pub use`; otherwise, narrow it to
`pub(crate)`.

#### Parent-facade exception

When the parent module re-exports the child item, the child `pub` is intentional and the
warning is suppressed:

```rust
// src/private_parent/mod.rs
mod child;
pub use child::Helper;
```

The exception applies whether the parent boundary is a `mod.rs` file or an ordinary file module
like `markdown_file.rs`. If nothing outside the parent subtree uses that re-export, the warning
still fires — and the compiler usually emits a paired `unused import` warning on the parent.
`cargo mend --fix-pub-use` is designed to repair that paired case.

<a id="unused-pub"></a>
### Unused `pub`

When a `pub` item is only used inside its defining module subtree, the modifier grants no useful
access. Private visibility already lets the defining module and its descendants call the item.

```rust
// src/lib.rs
mod renderer;

// src/renderer/mod.rs
mod tests;

pub fn normalize_label(label: &str) -> String {
    label.trim().to_string()
}

// src/renderer/tests.rs
fn example() {
    let _ = super::normalize_label(" title ");
}
```

The item does not need to be visible to the parent or sibling modules:

```rust
fn normalize_label(label: &str) -> String {
    label.trim().to_string()
}
```

This warning does not fire for `pub` items in a library crate root, for items reached from outside
their defining module subtree, or for items structurally exposed through public signatures.

`cargo mend --fix` can remove these `pub` annotations automatically.

<a id="prefer-module-import"></a>
### Prefer module import

This warning detects direct function imports and suggests importing the parent module instead,
then calling the function with module qualification.

Example:

```rust
// Before:
use crate::error::report_to_mcp_error;

fn example() {
    let error = report_to_mcp_error(&err);
}

// After:
use crate::error;

fn example() {
    let error = error::report_to_mcp_error(&err);
}
```

`cargo mend --fix` can rewrite these cases automatically. It rewrites the `use` statement and
qualifies all bare references in the file.

<a id="inline-path-qualified-type"></a>
### Inline path-qualified type

This warning detects types used with inline path qualification — both intra-crate
(`crate::module::MyType`, `super::module::MyType`) and external-crate
(`ratatui::Frame`, `std::collections::BTreeMap`, `notify::WatcherKind::Variant`) —
and suggests adding a `use` import at the top of the file instead. Trait paths in
`impl Trait for Type` are also covered.

Example:

```rust
// Before:
fn example() -> crate::module::MyType {
    crate::module::MyType::new()
}

fn render(frame: &mut ratatui::Frame<'_>) {}

impl crate::pane::Hittable for ToastManager { /* ... */ }

// After:
use crate::module::MyType;
use crate::pane::Hittable;
use ratatui::Frame;

fn example() -> MyType {
    MyType::new()
}

fn render(frame: &mut Frame<'_>) {}

impl Hittable for ToastManager { /* ... */ }
```

`cargo mend --fix` can rewrite these cases automatically. It adds the `use` import and replaces
all inline occurrences with the bare type name. The fix is skipped when adding the import would
shadow a name the file already uses (e.g. it won't add `use io::Result;` if the file relies on
the prelude `Result` via `Result::ok`).

<a id="shorten-local-crate-import"></a>
### Shorten local crate import

A `crate::a::b::c::*` import that crosses no module boundary makes the path look more global
than the relationship is. When the importer and the imported module share a parent, prefer the
local-relative form.

```rust
// src/app_tools/support/process.rs

// flagged — `cargo_detector` is a peer of `process` under `support`
use crate::app_tools::support::cargo_detector::TargetType;

// preferred
use super::cargo_detector::TargetType;
```

`cargo mend --fix` rewrites these cases automatically. It preserves the original `use`
visibility (`use`, `pub use`, `pub(crate) use`, etc.) and rolls the edits back if the follow-up
`cargo check` fails.

<a id="replace-deep-super-import"></a>
### Replace deep `super::` import

`super::super::` and deeper chains force the reader to count hops to figure out where the import
lands. When a single `super::` is not enough, a named `crate::` path is immediately clear.

Example:

```rust
// src/tui/columns/render.rs

// flagged — deep super chain
use super::super::ResolvedWidths;

// preferred — named crate path
use crate::tui::ResolvedWidths;
```

This applies at any depth: `super::super::super::` and beyond are all rewritten to the equivalent
`crate::` path.

`cargo mend --fix` can rewrite these cases automatically.

<a id="wildcard-parent-pub-use"></a>
### Wildcard parent `pub use`

This warning is about parent facade modules that re-export everything from a child with `*`.

That makes the boundary harder to read because the parent module no longer says what it is
actually exporting.

Prefer:

```rust
pub use child::{Helper, OtherHelper};
```

instead of:

```rust
pub use child::*;
```

<a id="internal-parent-pub-use-facade"></a>
### Internal parent `pub use` facade

This warning is about a parent boundary module that is being used as an internal namespace facade
inside its own subtree.

In other words:

- the parent `pub use` is not part of the outward boundary
- but code inside the subtree is still referring to the parent path directly
- that makes the parent boundary part of the implementation structure, not just the facade

Example:

```rust
// src/private_parent/mod.rs
mod child;
pub use child::Helper;

// src/private_parent/sibling.rs
fn use_helper() {
    let _ = std::mem::size_of::<super::Helper>();
}
```

In this example, `super::Helper` is using the parent boundary itself as an internal facade.

That can be intentional, but it is worth review because it usually means one of two things:

- the parent boundary is acting as an internal namespace and should stay that way intentionally
- or the subtree should import the child module directly instead of routing through the parent

`cargo mend --fix-pub-use` can rewrite these cases automatically. It deletes the parent `pub use`,
repoints every reference inside the subtree at the declaring child module — both `use` statements
and paths written inline — and narrows the child declaration to `pub(super)`. The facade is left in
place when the re-export cannot be resolved to a single declaring module, such as a `pub use ... *`
glob or a chain that leaves the crate.

<a id="narrow-to-pub-crate"></a>
### Narrow `pub` to `pub(crate)`

This warning flags bare `pub` items that can't actually escape the crate. Writing `pub(crate)` at
the definition makes the real reach visible at a glance, instead of forcing the reader to walk up
the module tree.

It fires in two situations:

**The crate root doesn't re-export the item.**

```rust
// src/lib.rs
mod helpers;
pub use helpers::exported_fn;

// src/helpers.rs
pub fn exported_fn() {}    // re-exported → must stay `pub`
pub fn internal_fn() {}    // NOT re-exported → should be `pub(crate)`
```

**The parent re-exports the item as `pub(crate) use`.** The `pub(crate) use` already caps reach at
the crate boundary, so the source modifier should match.

```rust
// src/keyboard/mod.rs
mod keys;
pub(crate) use keys::send_keys_handler;

// src/keyboard/keys.rs
pub fn send_keys_handler() {}   // → should be `pub(crate)`
```

Glob re-exports (`pub(crate) use foo::*`) are ignored — they neither trigger nor block this lint.
Items widened by a `pub use` somewhere in the chain are left alone.

Run `cargo mend --fix` to auto-fix these items to `pub(crate)`.

<a id="field-visibility-wider-than-type"></a>
### Field visibility wider than type

This warning flags struct, union, or enum-variant fields with a `pub` or `pub(crate)` annotation
on a **fully private type** (a type with no `pub` annotation of its own). The field annotation
cannot grant any access because the containing type itself isn't visible — the annotation is
dead.

The lint deliberately does **not** fire on `pub` fields of `pub(crate)` or `pub(super)` structs:

```rust
// Allowed — `pub` on fields of a `pub(crate)` struct is idiomatic Rust shorthand
pub(crate) struct GhRun {
    pub id:        u64,
    pub node_id:   String,
}
```

What does get flagged: a `pub` field on a struct that has no visibility annotation at all.

```rust
// inside a private module
struct Hidden {
    pub leaked: u32,   // dead — Hidden is private, `pub` grants nothing
}
```

After `cargo mend --fix`:

```rust
struct Hidden {
    leaked: u32,
}
```

Run `cargo mend --fix` to auto-remove dead field annotations.

<a id="imports-at-top"></a>
### Imports at top of file

This warning flags `use` statements written inside function bodies, closures, and other block
expressions. They should live at the top of the enclosing file or the enclosing inline
`mod { ... }` block instead.

```rust
// before
fn example() {
    use crate::movable::Movable;
    let m = Movable::default();
}
```

```rust
// after
use crate::movable::Movable;

fn example() {
    let m = Movable::default();
}
```

`cargo mend --fix` lifts the `use` to the top of the enclosing file or inline module. The
fix is conservative:

- `use` statements with any attribute (most importantly `#[cfg(...)]`) are left in place
  because lifting them could change what's in scope under a different configuration.
- Glob imports (`use foo::*;`) inside a body are left in place; they may shadow arbitrary
  names at the destination.
- When the bare name the in-body `use` introduces is already bound at the top of the
  destination — by another `use` with a different full path, or by a struct/enum/fn/etc.
  defined at that level — the in-body `use` is left in place to avoid an `E0255` collision.
- When the bare name and full path already match an existing top-level `use`, the in-body
  duplicate is deleted.

Run `cargo mend --fix` to auto-lift `use` statements.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
