# `tui_pane` test infrastructure design

This spec covers the test setup for the `tui_pane` workspace member crate: the private `test_support` module, unit-test placement, integration-test policy, and the boundary with the binary's existing `tui_test_support` module.

Examples in `///` comments are prose or ` ```ignore ` blocks. The public API is verified by `cfg(test)` unit tests inside the crate plus integration tests in `tui_pane/tests/`.

It supplements `docs/tui-pane-lib.md` (section **Test infrastructure**).

---

## 1. Private `test_support` module

Location: `tui_pane/src/test_support/mod.rs` (directory form — content grows past one file's worth quickly). Declared inside `lib.rs` as:

```rust
#[cfg(test)]
mod test_support;
```

Visibility throughout: `pub(crate)`. Nothing in this module is reachable from downstream crates.

### `TestCtx` — the cfg(test) Ctx fixture

The mock panes, dispatchers, and unit-test helpers all parameterize on a single `cfg(test)`-only context type:

```rust
// tui_pane/src/test_support/ctx.rs
use crate::{AppContext, Framework, FocusedPane};

pub(crate) struct TestCtx {
    pub(crate) dispatch_count: u32,
    pub(crate) flag: bool,
    pub(crate) level: i64,
    pub(crate) mode: String,
    pub(crate) framework: Framework<TestCtx>,
}

impl TestCtx {
    pub(crate) fn new() -> Self {
        Self {
            dispatch_count: 0,
            flag: false,
            level: 0,
            mode: String::new(),
            framework: Framework::new(FocusedPane::App(())),
        }
    }
}

impl AppContext for TestCtx {
    type AppPaneId = ();
    fn framework(&self)        -> &Framework<Self>     { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}
```

`type AppPaneId = ()` — the unit type satisfies every bound on `AppPaneId` (`Copy + Eq + Hash + Debug + 'static`) without inventing an enum. Mock panes that need a stable per-pane identity use it.

### Files and contents

```
tui_pane/src/test_support/
├── mod.rs              # re-exports + `cfg(test)` PaneId variant
├── mock_panes.rs       # MockPane, MockTextInputPane, MockStaticPane,
│                       #   MockPairedNavPane
├── builders.rs         # build_minimal_keymap(), build_keymap_with(),
│                       #   noop dispatchers
├── samples.rs          # SAMPLE_TOML_BASIC, SAMPLE_TOML_MULTIBIND,
│                       #   SAMPLE_TOML_DUPLICATE, SAMPLE_TOML_BAD_KEY
├── keys.rs             # plain(KeyCode), shift_char(char),
│                       #   ctrl_char(char), key_event(KeyCode)
└── ctx.rs              # build_test_ctx(), assert_dispatch_count(...)
```

#### `mod.rs`

```rust
// Internal test-only fixtures. NOT public, NOT re-exported from lib.rs.
#![cfg(test)]

pub(crate) mod builders;
pub(crate) mod ctx;
pub(crate) mod keys;
pub(crate) mod mock_panes;
pub(crate) mod samples;

// `TestCtx::AppPaneId = ()` — mock panes use the unit type for
// pane identity. No `cfg(test)` PaneId variant is needed.
```

#### `mock_panes.rs`

```rust
use crate::{
    action_enum, bindings, BarRegion, BarRow, Bindings, TestCtx, InputMode,
    KeyBind, Shortcut, Shortcuts,
};
use crossterm::event::KeyCode;

action_enum! {
    pub(crate) enum MockAction {
        Activate => "activate", "Activate";
        Cancel   => "cancel",   "Cancel";
    }
}

pub(crate) struct MockPane {
    pub(crate) busy: bool,
}

pub(crate) fn dispatch_mock(action: MockAction, ctx: &mut TestCtx) {
    ctx.dispatch_count += 1;
    match action {
        MockAction::Activate => ctx.flag = true,
        MockAction::Cancel   => ctx.flag = false,
    }
}

impl Shortcuts<TestCtx> for MockPane {
    type Action = MockAction;
    const SCOPE_NAME: &'static str = "mock";

    fn defaults() -> Bindings<MockAction> {
        bindings! {
            KeyCode::Enter => MockAction::Activate,
            KeyCode::Esc   => MockAction::Cancel,
        }
    }

    fn shortcut(&self, action: MockAction, _ctx: &TestCtx) -> Option<Shortcut> {
        match (action, self.busy) {
            (MockAction::Activate, true)  => Some(Shortcut::disabled("activate")),
            (MockAction::Activate, false) => Some(Shortcut::enabled("activate")),
            (MockAction::Cancel,   _)     => Some(Shortcut::enabled("cancel")),
        }
    }

    fn dispatcher() -> fn(MockAction, &mut TestCtx) { dispatch_mock }
}

// Variants with non-default input_mode for region-suppression tests.
pub(crate) struct MockTextInputPane;
impl Shortcuts<TestCtx> for MockTextInputPane { /* … input_mode = TextInput */ }

pub(crate) struct MockStaticPane;
impl Shortcuts<TestCtx> for MockStaticPane { /* … input_mode = Static */ }

// Pane that emits paired Nav rows — exercises ProjectList-style layout
// without needing the binary's ProjectListPane.
pub(crate) struct MockPairedNavPane;
impl Shortcuts<TestCtx> for MockPairedNavPane {
    /* … overrides bar_rows to emit Paired rows in BarRegion::Nav */
}
```

#### `builders.rs`

```rust
use crate::{TestCtx, Keymap, VimMode};

pub(crate) fn noop_quit(_ctx: &mut TestCtx)    {}
pub(crate) fn noop_restart(_ctx: &mut TestCtx) {}
pub(crate) fn noop_dismiss(_ctx: &mut TestCtx) {}

/// Smallest valid keymap: no panes, no nav, no globals — just the three
/// required builder positionals. Useful for tests of `BaseGlobals` and
/// scope lookup edge cases.
pub(crate) fn build_minimal_keymap() -> Keymap<TestCtx> {
    Keymap::<TestCtx>::builder(noop_quit, noop_restart, noop_dismiss)
        .vim_mode(VimMode::Disabled)
        .build()
}

/// Keymap with the standard mock pane registered, plus a closure for
/// further customization (TOML loading, vim mode toggle, extra panes).
pub(crate) fn build_keymap_with<F>(customize: F) -> Keymap<TestCtx>
where
    F: FnOnce(crate::KeymapBuilder<TestCtx>) -> crate::KeymapBuilder<TestCtx>,
{ /* … */ }
```

#### `samples.rs`

```rust
// TOML strings used by load.rs unit tests. Each is annotated with what
// it should produce (success outcome or specific error variant).

pub(crate) const SAMPLE_TOML_BASIC: &str = r#"
[mock]
activate = "Enter"
cancel   = "Esc"
"#;

pub(crate) const SAMPLE_TOML_MULTIBIND: &str = r#"
[mock]
activate = ["Enter", "Return"]
"#;

pub(crate) const SAMPLE_TOML_DUPLICATE_IN_ARRAY: &str = r#"
[mock]
activate = ["Enter", "Enter"]
"#;

pub(crate) const SAMPLE_TOML_CROSS_ACTION_DUP: &str = r#"
[mock]
activate = "Enter"
cancel   = "Enter"
"#;

pub(crate) const SAMPLE_TOML_UNKNOWN_SCOPE: &str = r#"
[no_such_scope]
foo = "Enter"
"#;

pub(crate) const SAMPLE_TOML_BAD_KEY_NAME: &str = r#"
[mock]
activate = "NotAKey"
"#;
```

#### `keys.rs`

```rust
use crate::KeyBind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn plain(code: KeyCode) -> KeyBind {
    KeyBind { code, mods: KeyModifiers::NONE }
}

pub(crate) fn shift_char(c: char) -> KeyBind { KeyBind::shift(c) }
pub(crate) fn ctrl_char(c: char)  -> KeyBind { KeyBind::ctrl(c) }

pub(crate) fn key_event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(crate) fn key_event_mods(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}
```

#### `ctx.rs`

```rust
use crate::TestCtx;

pub(crate) fn build_test_ctx() -> TestCtx { TestCtx::new() }

pub(crate) fn assert_dispatch_count(ctx: &TestCtx, expected: u32) {
    assert_eq!(
        ctx.dispatch_count, expected,
        "dispatch_count mismatch: got {}, expected {}",
        ctx.dispatch_count, expected,
    );
}
```

### What does NOT live in `test_support`

- Snapshot baselines — those live alongside the bar tests (per the snapshot crate's convention) under `tui_pane/src/bar/snapshots/`.
- Anything app-specific (no `App`, no cargo-port pane types, no cargo-port action enums).

---

## 2. Unit test placement

**Rule:** unit tests live next to their module via `#[cfg(test)] mod tests` at the bottom of the same file (or a sibling `tests.rs` if the module is a directory). Tests reach for fixtures via `use crate::test_support::…`.

Modules with non-trivial coverage needs (each owns its own `#[cfg(test)] mod tests`):

| Module                          | Coverage focus                                                                                                  |
|---------------------------------|------------------------------------------------------------------------------------------------------------------|
| `keymap/key_bind.rs`            | `From<KeyCode>` / `From<char>` / `shift` / `ctrl`; `display_short` for every `KeyCode` variant (paired-slot rule); round-trip parsing. |
| `keymap/bindings.rs`            | `bindings!` macro for each form (single, list, modifier); insertion order; `bind` vs `bind_many` parity.        |
| `keymap/scope_map.rs`           | `insert` rejects same-key→different-action; `by_key.len() == sum(by_action.values().len())`; `key_for` returns insertion-order primary; `display_keys_for` order. |
| `keymap/load.rs`                | Each `samples.rs` constant produces the documented outcome; replace-not-merge semantics; `config_path` return value per platform. |
| `keymap/vim.rs`                 | Vim extras layered after TOML; arrows remain primary; `vim_mode_conflicts` detection; `is_vim_reserved` reads live `Navigation` scope. |
| `keymap/builder.rs`             | Required positionals (`quit`, `restart`, `dismiss`); registration order; `BuilderError::{NavigationMissing, GlobalsMissing}` raised when those `with_*` calls are skipped. |
| `keymap/base_globals.rs`        | Framework-owned defaults (`q`, `R`, `Tab`, etc.); `Dismiss` calls injected hook; `OpenKeymap`/`OpenSettings` reach the framework panes. |
| `bar/region.rs`                 | `BarRegion::ALL` order matches render walk.                                                                     |
| `bar/shortcut.rs`               | `Shortcut::enabled` / `disabled` constructors; `BarRow` exhaustive match.                                       |
| `bar/support.rs`                | `format_action_keys` joins via `,`; `push_cancel_row` produces the expected row.                                                    |
| `bar/nav_region.rs`             | `Navigable`/`TextInput`/`Static` suppression; pane-cycle row pulled from `GlobalAction::NextPane`.          |
| `bar/pane_action_region.rs`     | Per-action labels read from `Shortcut`; `ShortcutState::Disabled` rendering.                                    |
| `bar/global_region.rs`          | `BaseGlobals` first, `AppGlobals::render_order()` after; `TextInput` suppression.                               |
| `bar/mod.rs`                    | End-to-end snapshot per fixture pane (drives `MockPane`, `MockTextInputPane`, `MockStaticPane`, `MockPairedNavPane`). |
| `panes/keymap_pane.rs`          | Mode transitions Browse → Awaiting → Conflict; `editor_target` reflects current state.                          |
| `panes/settings_pane.rs`        | Mode transitions Browse ↔ Editing; `is_text_input` flips correctly; `add_bool`/`add_enum`/`add_int` round-trips. |
| `panes/toasts.rs`               | Push/pop stack; `Dismiss` reaches the registered framework accessor.                                            |
| `settings.rs`                   | Each value flavor's get/set closure round-trips through `TestCtx`.                                               |
| `framework.rs`                  | `focused()`/`set_focused()` round-trip; `editor_target_path()` reads `self.focused()`; `focused_pane_input_mode(ctx)` falls back to `Navigable` for unregistered app panes. |

The full module set under `tui_pane/src/` matches the layout in `tui-pane-lib.md` lines 19-70; every leaf file gets a `#[cfg(test)] mod tests` block.

---

## 3. Cross-module integration tests

**Create `tui_pane/tests/` — one integration file per cross-cutting scenario.**

`#[cfg(test)] mod tests` covers the inside view (private fields, `pub(crate)` constructors, internal invariants). `tui_pane/tests/` covers the outside view: the same surface a downstream consumer sees.

Each integration file declares its own `IntegCtx` inline — typically a struct with a `Framework<Self>` field and the four-line `AppContext` impl, parallel to `TestCtx` but reachable from outside the crate. The duplication is intentional: integration tests verify the public API without privileged access to `test_support`.

```rust
// e.g. tui_pane/tests/builder_full.rs
use tui_pane::{AppContext, Framework, FocusedPane};

struct IntegCtx { framework: Framework<IntegCtx> }
impl AppContext for IntegCtx {
    type AppPaneId = ();
    fn framework(&self)        -> &Framework<Self>     { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}
```

Files under `tui_pane/tests/`:

| File                         | Scenario                                                                                                       |
|------------------------------|----------------------------------------------------------------------------------------------------------------|
| `tests/builder_full.rs`      | Construct a full `Keymap<IntegCtx>` via the public surface only; register multiple panes; load each `SAMPLE_TOML_*` constant (re-declared in this file or read from a fixtures dir); assert end-to-end resolution. |
| `tests/dispatch_routing.rs`  | Drive a `KeyEvent` through `Keymap::action_for`, then through `Shortcuts::dispatcher`, asserting `IntegCtx` mutations. Pane-scope-wins-over-navigation precedence test (Right → `ProjectListAction::ExpandRow` analog beats `NavigationAction::Right`). |
| `tests/bar_rendering.rs`     | Snapshot the full bar for a fixture pane against each `InputMode`. Keeps the snapshot crate (likely `insta`) confined to one integration file rather than scattered across module tests. |
| `tests/vim_mode.rs`          | Toggle `VimMode::Enabled` on a built keymap; assert `'h'/'j'/'k'/'l'` reach `NavigationAction::*`; assert TOML user-replace doesn't disable vim. |
| `tests/toml_errors.rs`       | Each documented error path (`SAMPLE_TOML_DUPLICATE_IN_ARRAY`, `SAMPLE_TOML_CROSS_ACTION_DUP`, `SAMPLE_TOML_BAD_KEY_NAME`) returns `Err` with the documented variant. |

### What `tests/` does NOT cover

- Exhaustive per-`KeyCode` `display_short` walk — that's a tight loop, lives in `keymap/key_bind.rs::tests`.
- Internal invariants (`by_key.len()` sum) — `tests/` cannot see private fields.
- Macro expansion correctness — covered by `cfg(test)` unit tests on `bindings!` and `action_enum!`.

---

## 4. Boundary with the binary's `tui_test_support`

The binary already owns `src/tui/tui_test_support.rs` (per Phase 9 of `tui-pane-lib.md`). It contains `pub(super) fn make_app` and binary-only test fixtures (`App` constructors, project-tree fixtures, etc.).

- `tui_pane::test_support` is `#[cfg(test)] pub(crate)`. It cannot leak across the workspace boundary — even `dev-dependencies` cannot reach a `cfg(test)`-gated module of an upstream crate. The binary's tests literally cannot `use tui_pane::test_support::…`; the path does not resolve.
- `src/tui/tui_test_support.rs` is `pub(super)` inside the binary crate; `tui_pane` cannot depend on the binary crate (dependency direction is binary → library only), so `tui_pane`'s tests cannot import binary fixtures.
- The two fixture sets are disjoint by language rule, not convention.

What the binary uses from `tui_pane`: all `pub` items — `Keymap`, `KeymapBuilder`, traits, macros, `KeyBind`, etc. — in production code. Binary tests use them through `make_app()` returning a real `App` with a real `Keymap<App>`. End-to-end key-event → pane-state-mutation chains against the real `App` are binary-side; they exercise cargo-port's wiring, not the framework's own correctness.

### Summary of the split

| Layer                     | Test code lives in                                                  | Fixture source                                |
|---------------------------|---------------------------------------------------------------------|-----------------------------------------------|
| Framework module-internal | `tui_pane/src/**/*.rs` `#[cfg(test)] mod tests`                     | `tui_pane::test_support` (`pub(crate)`); `TestCtx` |
| Framework public surface  | `tui_pane/tests/*.rs`                                               | Inline helpers; per-file `IntegCtx`           |
| Binary module-internal    | `src/**/*.rs` `#[cfg(test)] mod tests`                              | `src/tui/tui_test_support.rs` (`pub(super)`)  |
| Binary integration        | `tests/*.rs` (binary crate)                                         | `make_app()` from `tui_test_support`          |
