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

`pub_in_path` controls exact crate-rooted `pub(in crate::path)` boundaries on declarations, fields,
and `use` re-exports:

- `forbidden` rejects them
- `permitted` accepts them when they exactly match a caller, signature, or parent-facade boundary
- `required` (the default) also asks `suspicious_pub` to recommend that boundary instead of bare
  `pub`, and `cargo mend --fix` rewrites the declaration or field

A `use` re-export is accepted on the same terms as a declaration, but is never rewritten to a wider
visibility: widening the re-export past the item it names would not compile.

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

| `cargo-mend`  | rustc     |
|---------------|-----------|
| 0.22.2+       | 1.99      |
| 0.17-0.22.1   | 1.97-1.98 |
| 0.16.x        | 1.96      |

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

`pub(crate)` lets any module in the crate touch the item, regardless of where it lives. In a deep
module tree that usually weakens the boundaries the layout was meant to enforce. Cargo-mend joins
callers and signature requirements across every selected target and computes their deepest common
module: a narrower common module makes the crate-wide annotation broader than required.

This applies to declarations and struct and union fields. In the example, `helpers.rs` exists only
to support `feature`, and every caller is under `feature`, so the helper only needs parent access.
See [boundary selection and repair policy](#policy-selecting-a-visibility-boundary) for other reaches,
field caller evidence, and coordinated fixes.

**Before**

```rust
// src/feature/mod.rs
mod helpers;

// src/feature/helpers.rs
pub(crate) fn helper() {}
```

**After**

```rust
// src/feature/mod.rs
mod helpers;

// src/feature/helpers.rs
pub(super) fn helper() {}
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Selecting a visibility boundary

`helper` is now available to `feature` and nowhere else. Prefer:

- private items when they are local implementation details
- `pub(super)` when the parent module owns the boundary
- `pub(in crate::path)` when callers, signatures, or a parent facade require a wider ancestor
- `pub(crate)` when uses genuinely span crate-root sibling branches
- moving the item to a better common parent when `pub(super)` is too narrow

The fixer writes each proven result directly: it removes an unnecessary annotation, uses
`pub(super)` for the immediate parent, uses exact `pub(in crate::path)` for a wider ancestor, and
uses `pub(crate)` for the crate root. It applies related declaration changes in one batch before the
validation build, so a trait and its associated type can narrow together without an intermediate
E0446 failure. Field construction, access, destructuring, and `offset_of!` computations contribute
caller evidence for named and positional fields.

When mend gives facade-specific advice, it quotes the facade's `use` spelling only when it can
establish that spelling from the active item graph. A resolved reach alone never licenses quoting a
modifier; otherwise the diagnostic calls it a re-export without naming a modifier.

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

Arbitrary `pub(in ...)` annotations can hide an ownership problem behind a long path. An exact
crate-rooted `pub(in crate::path)` is accepted when `path` is the narrowest boundary required by
callers, signatures, or a parent facade. Set `pub_in_path = "permitted"` (the less strict mode) or
`"required"` (the default) to accept that exact boundary.

A parent re-export can require this form: its `pub(super) use` reaches above the item's immediate
parent, where a declaration's `pub(super)` would be too narrow. Spell the boundary from `crate::`
instead of making readers count `super::` levels. The
[boundary repair policy](#policy-restricted-visibility-boundaries) explains facade placement, manual
repairs, and when findings become warnings.

**Before**

```rust
// src/video_plane/plane/camera_panel.rs
pub(in super::super) fn bind_camera_panel() {}

// src/video_plane/plane/mod.rs
mod camera_panel;
pub(super) use camera_panel::bind_camera_panel;
```

**After**

```rust
// src/video_plane/plane/camera_panel.rs
pub(in crate::video_plane) fn bind_camera_panel() {}

// src/video_plane/plane/mod.rs
mod camera_panel;
pub(super) use camera_panel::bind_camera_panel;
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Restricted visibility boundaries

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

Exact-boundary repairs apply to declarations, fields, and `pub use` re-exports alike — a re-export
earns the boundary on the same terms a declaration does. mend will not offer to narrow a
re-export's visibility, though: resolved paths name the imported item, not the alias, so it can see
where the item is used but not where the alias is.

##### When no facade covers the boundary

The annotation grants visibility; it does not create a path. `pub(in crate::animation::sequence)`
lets every module under `animation::sequence` *see* the item, but the item is only *nameable* where
its own declaration or `use` line puts it. When those two disagree, mend reports the gap:

```rust
// crates/hana_lagrange/src/animation/sequence/request/mod.rs
mod journey;
pub(in crate::animation::sequence) use journey::RetainedCameraJourneyOrigin;

// crates/hana_lagrange/src/animation/sequence/playback/emission.rs
use crate::animation::sequence::request::RetainedCameraJourneyOrigin;
//                              ^^^^^^^^ still reaching through `request`
```

The permission reaches `animation::sequence`; the path stops at `animation::sequence::request`.
Two costs follow. Callers are coupled to the internals of `request`, so renaming or moving
`journey` breaks every one of them. And nothing at `animation::sequence` records that the item is
shared across the subtree — to find out what the subtree shares with itself, you would have to
search it for annotations.

The repair is a re-export at the module the annotation names. That re-export is what mend calls a
facade:

```rust
// crates/hana_lagrange/src/animation/sequence/mod.rs
pub(in crate::animation::sequence) use request::RetainedCameraJourneyOrigin;

// crates/hana_lagrange/src/animation/sequence/playback/emission.rs
use crate::animation::sequence::RetainedCameraJourneyOrigin;
```

The path now reaches as far as the visibility does, `sequence/mod.rs` lists what the subtree shares
with itself, and `request` can be reorganized without touching its consumers.

Moving the item is the other repair, and sometimes the better one. If the item is not specific to
the module holding it — `journey` above is a `sequence` concept living under `request` — moving it
to `crate::animation::sequence` removes the need for a facade entirely. mend reports the gap and
names the facade; which repair fits is a design decision, which is why this case has no `--fix`.

##### When nothing outside the module uses the item

A third case needs neither repair. If no use of the item was found outside the module that declares
it, the item is already reachable there with no annotation at all, and the annotation is the whole
problem:

```rust
// crates/hana_lagrange/src/animation/sequence/controller_installation.rs
pub(in crate::animation) struct CameraControllerInstallation {
    pub(in crate::animation) identity: CameraControllerInstallationIdentity,
}
```

mend reports this as a warning rather than an error and `--fix` deletes the annotation, because
deleting one resolves no path and so needs no design decision. The repair applies to declarations
and fields; a `mod` line and a `use` re-export keep the error, since a module's visibility caps
every item inside it and a re-export exists to be named elsewhere.

##### When the item needs a wider boundary than it has

A fourth case is repaired by widening. An item can be used only from inside the module its
annotation names and still need a wider boundary, because a `pub(crate)` field or signature carries
it further than its own uses do:

```rust
// crates/hana_lagrange/src/animation/sequence/playback/pose.rs
pub(in crate::animation) struct OrbitCameraPose { /* ... */ }

// crates/hana_lagrange/src/animation/sequence.rs
pub(crate) struct Carrier {
    pub(crate) pose: OrbitCameraPose,
}
```

`Carrier::pose` is readable anywhere in the crate, so `OrbitCameraPose` has to be too. mend reports
this as a warning and `--fix` writes `pub(crate)`, because `pub(in crate::animation)` is contained
in `pub(crate)`: the wider boundary keeps every name that already resolved, so a use the caller scan
did not see still compiles.

Narrowing is the case this excludes. When the resolved boundary is narrower than the annotation —
`pub(in crate::animation)` on an item whose callers all sit in the parent module, where `pub(super)`
would do — the finding stays an error. A caller the scan missed would no longer reach the item, and
that is a build break rather than a warning.

Do not use this form to avoid moving an item. If the required path is long or names a module
unrelated to the item, the item belongs in a different module. It is not a way to widen access:
`pub(in crate::a)` is narrower than `pub(crate)` and `pub`. If it seems necessary to unlock access,
consider whether the intended decision is a `pub(crate) use` facade instead, and state that decision
on the `use` line.

<a id="review-pub-mod"></a>
### `pub mod` requires explicit review or allowlisting

Publishing a module path makes it part of the crate's public API, so `pub mod` is disallowed by
default. Keep implementation modules private; when the public path is intentional, such as for macro
or codegen constraints, use the [explicit allowlist alternative](#policy-intentional-public-module-paths).

**Before**

```rust
// src/lib.rs
pub mod tools;
```

**After**

```rust
// src/lib.rs
mod tools;
```

#### Policy: Intentional public module paths

If `tools` must remain public, keep `pub mod tools;` and allowlist the file containing that declaration:

```toml
# mend.toml
[visibility]
allow_pub_mod = ["src/lib.rs"]
```

<a id="suspicious-pub"></a>
### `pub` is broader than this nested module boundary

Bare `pub` promises more access than a nested private module can provide: if a parent on the path is
private, the item cannot escape the crate. Match the annotation to the callers rather than making
readers walk the module tree. Here only `support` consumes `Helper`, so parent visibility is enough.

Other resolutions are to make the item private, use the exact `pub(in crate::path)` or `pub(crate)`
boundary mend reports for wider access, or move a genuinely shared item to a better common parent.
See the [nested-module and parent-facade policy](#policy-nested-pub-and-parent-facades) for the checks
and the paired cleanup that uses `--fix-pub-use`.

**Before**

```rust
// src/lib.rs
mod support;

// src/support/mod.rs
mod helpers;

fn use_helper() {
    let _ = std::mem::size_of::<helpers::Helper>();
}

// src/support/helpers.rs
pub struct Helper;
```

**After**

```rust
// src/lib.rs
mod support;

// src/support/mod.rs
mod helpers;

fn use_helper() {
    let _ = std::mem::size_of::<helpers::Helper>();
}

// src/support/helpers.rs
pub(super) struct Helper;
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Nested `pub` and parent facades

At logical depth greater than one, `suspicious_pub` examines bare `pub` and accepted crate-rooted
`pub(in crate::...)` declarations. Its parent-facade-boundary suggestion for a bare `pub` appears
only when `pub_in_path = "required"`; accepted `pub(in ...)` declarations are also reviewed for a
boundary that has become stale or no longer matches the facade.

This warning does not fire at a top-level private module — [`pub` exceeds the item's effective
reach](#narrow-to-pub-crate) covers that case. At the top level, bare `pub` is only correct when the
crate root re-exports the item via `pub use`; otherwise, narrow it to `pub(crate)`.

When a parent re-exports a child item for use outside the parent subtree, the child `pub` is
intentional and the warning is suppressed. This applies whether the parent is a `mod.rs` file or an
ordinary file module such as `markdown_file.rs`. If nothing outside the parent subtree uses that
re-export, the warning still fires, and the compiler usually emits a paired `unused import` warning
on the parent. Remove that unused facade and narrow the child together:

**Before**

```rust
// src/private_parent/mod.rs
mod child;
pub use child::Helper;

// src/private_parent/child.rs
pub struct Helper;
```

**After**

```rust
// src/private_parent/mod.rs
mod child;

// src/private_parent/child.rs
pub(super) struct Helper;
```

`cargo mend --fix-pub-use` can rewrite these cases automatically.

<a id="unused-pub"></a>
### `pub` item is not used outside its defining module

A modifier that grants no useful access obscures the module's boundary. Private visibility already
lets the defining module and its descendants call the item, so an item used only in that subtree
does not need to be visible to parents or siblings.

This warning excludes `pub` items in a library crate root, items reached from outside their defining
module subtree, and items structurally exposed through public signatures.

**Before**

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

**After**

```rust
// src/renderer/mod.rs
fn normalize_label(label: &str) -> String {
    label.trim().to_string()
}
```

`cargo mend --fix` can rewrite these cases automatically.

<a id="prefer-module-import"></a>
### Function import should use module-qualified form

Module qualification shows where a function belongs at each call site. Direct function imports hide
that relationship; import the parent module instead. The fixer rewrites the `use` statement and
qualifies its bare references in the file.

**Before**

```rust
// src/tui/error_handler.rs
use crate::error::report_to_mcp_error;

fn handle_error(err: std::io::Error) {
    let error = report_to_mcp_error(&err);
}
```

**After**

```rust
// src/tui/error_handler.rs
use crate::error;

fn handle_error(err: std::io::Error) {
    let error = error::report_to_mcp_error(&err);
}
```

`cargo mend --fix` can rewrite these cases automatically.

<a id="inline-path-qualified-type"></a>
### Inline path-qualified type should use a `use` import

Imports give a type one readable name throughout a file instead of repeating its full path. This
covers intra-crate paths such as `crate::scene::CameraPose` and `super::scene::CameraPose`, external
paths such as `ratatui::Frame`, `std::collections::BTreeMap`, and `notify::WatcherKind::Variant`, and
trait paths in `impl Trait for Type`.

The fixer adds a `use` import at the top of the file and replaces inline occurrences with the bare
type name. See the [import shadowing policy](#policy-inline-type-import-shadowing) for skipped cases.

**Before**

```rust
// src/tui/camera_panel.rs
fn initial_pose() -> crate::scene::CameraPose {
    crate::scene::CameraPose::new()
}

fn render(frame: &mut ratatui::Frame<'_>) {}

impl crate::pane::Hittable for ToastManager { /* ... */ }
```

**After**

```rust
// src/tui/camera_panel.rs
use crate::scene::CameraPose;
use crate::pane::Hittable;
use ratatui::Frame;

fn initial_pose() -> CameraPose {
    CameraPose::new()
}

fn render(frame: &mut Frame<'_>) {}

impl Hittable for ToastManager { /* ... */ }
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Inline type import shadowing

The fix is skipped when adding an import would shadow a name the file already uses. For example, it
will not add `use io::Result;` where the file relies on prelude `Result` through `Result::ok`.

<a id="shorten-local-crate-import"></a>
### Crate-relative import can be shortened to a local-relative import

A crate-rooted import within one local module boundary makes the relationship look more global than
it is. When importer and imported module share a parent, prefer the local-relative form. Here
`cargo_detector` and `process` are peers under `support`.

The fixer preserves the original `use` visibility (`use`, `pub use`, `pub(crate) use`, etc.) and rolls
edits back if the follow-up `cargo check` fails.

**Before**

```rust
// src/app_tools/support/process.rs
use crate::app_tools::support::cargo_detector::TargetType;
```

**After**

```rust
// src/app_tools/support/process.rs
use super::cargo_detector::TargetType;
```

`cargo mend --fix` can rewrite these cases automatically.

<a id="replace-deep-super-import"></a>
### Deep `super::` chain should use a `crate::` path

`super::super::` and deeper chains force readers to count hops to locate an import. When one
`super::` is not enough, a named `crate::` path makes the destination clear. Chains of any depth,
including `super::super::super::` and beyond, are rewritten to the equivalent crate-rooted path.

**Before**

```rust
// src/tui/columns/render.rs
use super::super::ResolvedWidths;
```

**After**

```rust
// src/tui/columns/render.rs
use crate::tui::ResolvedWidths;
```

`cargo mend --fix` can rewrite these cases automatically.

<a id="wildcard-parent-pub-use"></a>
### Parent module `pub use *` should be explicit

A parent facade that re-exports every child item hides what its boundary actually exports. Name the
items explicitly so readers can see that boundary without opening the child module.

**Before**

```rust
// src/private_parent/mod.rs
pub use child::*;
```

**After**

```rust
// src/private_parent/mod.rs
pub use child::{Helper, OtherHelper};
```

<a id="internal-parent-pub-use-facade"></a>
### Parent module re-export is acting as an internal facade

Routing internal references through a parent's outward facade makes that boundary part of the
implementation structure. When the parent `pub use` is not used outside its subtree, its children
can name the defining child directly instead of using the parent as an internal namespace.

That namespace can be intentional and deserves review; otherwise, remove the facade, repoint subtree
references in both `use` statements and inline paths, and narrow the child declaration to
`pub(super)`. See the [facade resolution policy](#policy-internal-facade-resolution) for cases left
in place.

**Before**

```rust
// src/private_parent/mod.rs
mod child;
mod sibling;
pub use child::Helper;

// src/private_parent/child.rs
pub struct Helper;

// src/private_parent/sibling.rs
fn use_helper() {
    let _ = std::mem::size_of::<super::Helper>();
}
```

**After**

```rust
// src/private_parent/mod.rs
mod child;
mod sibling;

// src/private_parent/child.rs
pub(super) struct Helper;

// src/private_parent/sibling.rs
fn use_helper() {
    let _ = std::mem::size_of::<super::child::Helper>();
}
```

`cargo mend --fix-pub-use` can rewrite these cases automatically.

#### Policy: Internal facade resolution

The facade stays in place when its re-export cannot be resolved to a single declaring module, such
as a `pub use ... *` glob or a chain that leaves the crate.

<a id="narrow-to-pub-crate"></a>
### `pub` exceeds the item's effective reach — use `pub(crate)`

Bare `pub` suggests access beyond the crate even when the module tree prevents it. Writing
`pub(crate)` at the definition states that effective reach without making the reader trace every
parent. This applies when the crate root does not re-export the item, or when a parent's
`pub(crate) use` already caps its reach at the crate boundary.

The example keeps the crate-root export public while narrowing the other declarations. See the
[re-export policy](#policy-crate-reach-and-re-exports) for glob and widening exceptions.

**Before**

```rust
// src/lib.rs
mod helpers;
mod keyboard;
pub use helpers::exported_fn;

pub fn entry() {
    helpers::internal_fn();
}

// src/helpers.rs
pub fn exported_fn() {}
pub fn internal_fn() {}

// src/keyboard/mod.rs
mod keys;
pub(crate) use keys::send_keys_handler;

// src/keyboard/keys.rs
pub fn send_keys_handler() {}
```

**After**

```rust
// src/lib.rs
mod helpers;
mod keyboard;
pub use helpers::exported_fn;

pub fn entry() {
    helpers::internal_fn();
}

// src/helpers.rs
pub fn exported_fn() {}
pub(crate) fn internal_fn() {}

// src/keyboard/mod.rs
mod keys;
pub(crate) use keys::send_keys_handler;

// src/keyboard/keys.rs
pub(crate) fn send_keys_handler() {}
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Crate reach and re-exports

Glob re-exports such as `pub(crate) use keys::*` are ignored: they neither trigger nor block this
lint. Items widened by a `pub use` somewhere in the chain are left alone.

<a id="field-visibility-wider-than-type"></a>
### Field visibility is wider than its containing type

A field annotation cannot grant access beyond a fully private containing type, so that annotation
is dead. This warning covers struct, union, or enum-variant fields with `pub` or `pub(crate)` on a
type with no `pub` annotation of its own; the fixer removes the dead field annotation.

The [annotated-type policy](#policy-fields-of-annotated-types) explains why `pub` fields on
`pub(crate)` or `pub(super)` types are allowed.

**Before**

```rust
// src/cache/entry.rs
struct Hidden {
    pub leaked: u32,
}
```

**After**

```rust
// src/cache/entry.rs
struct Hidden {
    leaked: u32,
}
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Fields of annotated types

The lint deliberately does not fire on `pub` fields of `pub(crate)` or `pub(super)` structs. That
combination is idiomatic Rust shorthand for fields as visible as their type:

```rust
// src/ci/run.rs
pub(crate) struct GhRun {
    pub id:        u64,
    pub node_id:   String,
}
```

<a id="imports-at-top"></a>
### `use` statement should live at the top of the file or inline module

Imports at the enclosing file or inline module's top give readers one place to see its dependencies.
A `use` inside a function, closure, or other block expression hides that dependency until the reader
reaches the body. Move it to the top of the file or enclosing `mod { ... }` block.

The fixer lifts supported imports and removes duplicates without introducing name collisions. See
the [lifting policy](#policy-lifting-imports-safely) for attributes, globs, and existing bindings.

**Before**

```rust
// src/editor/drag.rs
fn example() {
    use crate::movable::Movable;
    let m = Movable::default();
}
```

**After**

```rust
// src/editor/drag.rs
use crate::movable::Movable;

fn example() {
    let m = Movable::default();
}
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Lifting imports safely

The fix is conservative because lifting an import can change scope under a different configuration
or shadow a name at its destination:

- Non-conditional attributes on a `use`, such as `#[allow(...)]` or doc comments, leave it in place.
  `#[cfg(...)]` and `#[cfg_attr(...)]` gates on the import or its enclosing constructs travel with
  a moved import so it stays conditionally compiled.
- Glob imports inside a body are left in place because they may shadow arbitrary names.
- When the bare name is already bound at the destination by an import of a different full path or
  a struct, enum, function, or other item, the in-body import stays in place to avoid an `E0255`
  collision. A gated import stays in place if any binding already has that bare name.
- When an unconditional in-body import's bare name and full path match an existing unconditional
  top-level `use`, the duplicate is deleted.

<a id="pub-use-outside-subtree"></a>
### Re-export reaches outside this module's subtree

A facade belongs to a module whose subtree holds the item. Re-exporting from elsewhere turns one
module into a back door into another, so callers reach an item through a module that does not own
it. Re-export from the owning module, usually the parent `mod.rs`, or move the definition to the
module that owns the concept and point callers there.

This covers re-exports through `super::sibling::item`, `crate::elsewhere::item`, or `super::Item` of
the parent's own item. A private sibling `use` is fine. A `prelude` module re-exporting its parent's
items and re-exports from another crate are excluded.

The fixer removes the facade, retaining a private `use` if its module still names the item, and
redirects crate callers to the owning path. If callers outside the item's scope need one, it adds a
re-export at the nearest module containing both modules, copying the original `#[cfg]` attributes.
Here the private `reset_scene` module limits direct access to `tool`, so the crate-root caller needs
the new facade. Fixable findings are warnings; see
[unsupported repairs](#policy-subtree-re-export-repair-limits) for cases that stay errors.

**Before**

```rust
// src/lib.rs
mod tool;

pub fn reset_tools() {
    tool::tool_staging::build_device_recipe();
}

// src/tool/mod.rs
mod reset_scene;
pub(crate) mod tool_staging;

// src/tool/reset_scene.rs
pub(crate) fn build_device_recipe() {}

// src/tool/tool_staging/mod.rs
pub(crate) use super::reset_scene::build_device_recipe;
```

**After**

```rust
// src/lib.rs
mod tool;

pub fn reset_tools() {
    tool::build_device_recipe();
}

// src/tool/mod.rs
pub(crate) use reset_scene::build_device_recipe;
mod reset_scene;
pub(crate) mod tool_staging;

// src/tool/reset_scene.rs
pub(crate) fn build_device_recipe() {}

// src/tool/tool_staging/mod.rs
// The sideways re-export has been removed.
```

`cargo mend --fix` can rewrite these cases automatically.

#### Policy: Subtree re-export repair limits

These cases stay errors with no automatic fix, and the finding's help says which one applies:

- the re-export renames the item with `as`
- the re-export is a glob
- the path passes through an enum or trait before its last segment
- the re-exporting module is reachable from outside the crate, so removing the re-export would
  change a public path
- the common ancestor already has a different item under that name, or imports it with narrower
  visibility
- the rewrite would need a `pub(in …)` boundary
- the re-exported item does not resolve to one target

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
