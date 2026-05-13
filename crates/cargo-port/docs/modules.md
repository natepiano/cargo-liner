# `tui_pane` Module Restructure

**Status:** Implemented
**Source:** Team review in this session — synthesis of 3 expert agents on cohesion, coupling, and root-vs-submodule placement.

## Constraint

Every item lives either at the crate root (single `.rs` file) or inside a directory submodule. No mid-tier orphans.

## Starting layout

```
tui_pane/src/
├── activity.rs       (FrameCycle, Icon, ACTIVITY_SPINNER)
├── app_context.rs    (AppContext, NoToastAction)
├── pane.rs           (Pane trait, Mode enum)
├── pane_id.rs        (FocusedPane, FrameworkFocusId, FrameworkOverlayId)
├── viewport.rs       (Viewport, ViewportOverflow, render_overflow_affordance)
├── lib.rs
├── bar/              (status bar renderer)
├── framework/        (Framework<Ctx>, dispatch, tab order — owns ModeQuery)
├── keymap/           (binding engine, builder/, action enums)
├── panes/            (KeymapPane, SettingsPane, panes/toasts.rs holds ToastsAction)
├── settings_store/   (store, registry, codecs — and toast.rs holds ToastSettings)
└── toasts/           (toast lifecycle, render, format, items, stack)
```

## Three moves

### 1. Move `ToastSettings` family from `settings_store/` into `toasts/`

**What moves.** Entire contents of `tui_pane/src/settings_store/toast.rs`:

- `ToastSettings`
- `ToastWidth`
- `ToastGap`
- `ToastDuration`
- `ToastPlacement`
- `ToastAnimationSettings`
- `MaxVisibleToasts`

**Destination.** `tui_pane/src/toasts/settings.rs`.

**Why.** Every type in `settings_store/toast.rs` is framework-owned toast configuration. It sits in `settings_store/` only because that module also holds the generic TOML codecs that load `[toasts]` from disk. After the move, `settings_store/` is purely the generic mechanism; `toasts/` owns every toast type including its configuration.

**Path change.** Internal `use crate::settings_store::ToastSettings` becomes
`use crate::toasts::ToastSettings` or the stable crate-root
`use crate::ToastSettings`. Crate-root re-exports retarget from
`settings_store::Toast*` to `toasts::Toast*`, but the public caller path stays
stable: `tui_pane::ToastSettings`, `tui_pane::ToastDuration`, etc.

**TOML codec consequence.** `settings_store/registry.rs` / `settings_store/store.rs` parse the `[toasts]` section into `ToastSettings`. The parse code stays in `settings_store/`; only the type definitions move. The store imports `ToastSettings` from `toasts::` rather than from its own subtree.

**Verification.**
- `cargo check --workspace --all-targets`
- `cargo nextest run -p tui_pane settings`
- `cargo nextest run -p tui_pane toasts`
- Grep `pub use settings_store::Toast` in `lib.rs` — must be empty.

### 2. Move `ToastsAction` from `panes/toasts.rs` into `toasts/`

**What moves.** Contents of `tui_pane/src/panes/toasts.rs` (currently 9 lines — just the `ToastsAction` enum and its `Action` impl).

**Destination.** `tui_pane/src/toasts/action.rs`.

**Why.** `toasts/stack.rs:37` imports `crate::panes::ToastsAction` — a sibling module reaching into a "higher" organizational module for one enum. After the move, `ToastsAction` lives where `Toasts` lives. The inverted import vanishes.

**Side effect.** `panes/` becomes coherent: framework overlay panes only (`KeymapPane`, `SettingsPane`). Delete `panes/toasts.rs`.

**Path change.**
- `tui_pane/src/lib.rs:77` — `pub use panes::ToastsAction` → `pub use toasts::ToastsAction`.
- `tui_pane/src/toasts/stack.rs` — drop `use crate::panes::ToastsAction`, replace with `use super::ToastsAction` (or remove if already in scope).
- `tui_pane/src/panes/mod.rs` — drop `mod toasts;` and the `pub use toasts::ToastsAction;` line.

**Verification.**
- `cargo check --workspace --all-targets`
- `rg "panes::toasts" tui_pane/` — must be empty.
- `rg "use crate::panes::ToastsAction" tui_pane/` — must be empty.

### 3. Move `ModeQuery` from `framework/` into `pane.rs`

**What moves.** The `ModeQuery<Ctx>` type alias (currently `pub(crate)` in `tui_pane/src/framework/mod.rs`).

**Destination.** `tui_pane/src/pane.rs`, alongside `Mode<Ctx>` and `Pane<Ctx>`.
The alias stays `pub(crate)`; it is an internal storage helper, not public API.

**Why.** `ModeQuery<Ctx>` is `fn(&Ctx) -> Mode<Ctx>` — a property of `Pane<Ctx>`, not a property of `Framework<Ctx>`. The `keymap/builder/` module currently imports `crate::framework::ModeQuery` to register panes, which creates a soft cycle (keymap → framework). After the move, both `framework/` and `keymap/builder/` import `ModeQuery` from `pane`, which is a leaf.

**Path change.**
- `tui_pane/src/pane.rs` — add `pub(crate) type ModeQuery<Ctx> = fn(&Ctx) -> Mode<Ctx>;`.
- `tui_pane/src/framework/mod.rs` — drop the local definition; replace internal references with `use crate::pane::ModeQuery;`.
- `tui_pane/src/keymap/builder/mod.rs` — change `use crate::framework::ModeQuery` to `use crate::pane::ModeQuery`.

**Verification.**
- `cargo check --workspace --all-targets`
- `rg "framework::ModeQuery" tui_pane/` — must be empty.
- `cargo nextest run -p tui_pane keymap`

## Resulting layout

```
tui_pane/src/
├── activity.rs       (unchanged)
├── app_context.rs    (unchanged)
├── pane.rs           (+ ModeQuery)
├── pane_id.rs        (unchanged)
├── viewport.rs       (unchanged)
├── lib.rs            (re-exports retargeted)
├── bar/              (unchanged)
├── framework/        (− ModeQuery local definition)
├── keymap/           (unchanged — uses ModeQuery via pane.rs)
├── panes/            (− toasts.rs; now KeymapPane + SettingsPane only)
├── settings_store/   (− toast.rs; now purely generic codec/registry mechanism)
└── toasts/           (+ settings.rs from settings_store/; + action.rs from panes/)
```

Five single-file root modules + six directory modules. No mid-tier orphans. Inverted toasts↔panes import gone. Soft keymap↔framework cycle gone.

## What I'm not doing

- Splitting `keymap/` further (premature without a concrete pain point).
- Splitting `panes/` by file size (size is not a cohesion problem).
- Moving `viewport.rs` into `framework/` — cargo-port (the binary) consumes `tui_pane::Viewport` directly for every app pane, so it's shared infrastructure, not framework-internal.
- Abstracting `bar`'s direct `Toasts` import through a framework helper — the one-line `use` costs less than the indirection.

## Order of operations

Do move 3 first (smallest, breaks no public API), then move 1 (retargets the
crate-root re-export source while keeping the public path stable), then move 2
(deletes `panes/toasts.rs`). All three are mechanically independent.

## Verification after all three

- `cargo +nightly fmt --all`
- `cargo mend --workspace --all-targets`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo nextest run --workspace`
- `cargo install --path .`
- `rg "pub use settings_store::Toast" tui_pane/src/lib.rs` — empty
- `rg "pub use panes::ToastsAction" tui_pane/src/lib.rs` — empty
- `rg "framework::ModeQuery" tui_pane/` — empty
