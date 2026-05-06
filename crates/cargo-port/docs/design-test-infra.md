# `tui_pane` test infrastructure design

This spec covers the concrete test setup for the `tui_pane` workspace member crate: the public doctest fixture, doctest snippets for every documented public item, the private `test_support` module, unit-test placement, integration-test policy, and the boundary with the binary's existing `tui_test_support` module.

It supplements `docs/tui-pane-lib.md` (sections **Doctest policy** and **Test infrastructure**) and does not replace any decision recorded there.

---

## 1. Public fixture `Ctx` type

### Name: `DocCtx`

Picked over `ExampleCtx` / `SampleCtx` / `FixtureCtx`:

- `DocCtx` names the **role** the type plays (it exists so doctests can substitute it for the binary's `App`). Doctests are the only sanctioned caller; "doc" in the name matches that intent and discourages downstream crates from reaching for it as a general-purpose mock.
- `ExampleCtx` is too generic — a downstream crate writing its own examples might also reach for it.
- `SampleCtx` / `FixtureCtx` lean toward unit-test fixture vocabulary; `test_support` already owns that role, and that module is `pub(crate)`, not public.

The doc comment on `DocCtx` directs readers: this type exists for crate-internal doctests; production code substitutes its own `Ctx`.

### Definition

```rust
// tui_pane/src/doc_ctx.rs
//
// Public fixture context used by every doctest in this crate. Production
// callers parameterize the framework over their own context (cargo-port
// uses `App`); the framework imposes no trait bounds on `Ctx` beyond
// `'static` where required by individual signatures.

use std::collections::HashMap;

/// Minimal context type used by `tui_pane`'s own doctests.
///
/// Real applications substitute their own root context (cargo-port uses
/// `App`). `DocCtx` carries just enough state for trait examples,
/// dispatcher fn pointers, settings get/set closures, and a
/// `Framework<DocCtx>` accessor.
///
/// Not intended for downstream test harnesses — depend on it only inside
/// `///` blocks under `tui_pane`.
#[derive(Debug, Default)]
pub struct DocCtx {
    /// Counter mutated by example dispatchers so doctest assertions
    /// can confirm the fn pointer fired.
    pub dispatch_count: u32,

    /// A bool the settings doctest flips through `add_bool`.
    pub flag: bool,

    /// A small int the settings doctest steps through `add_int`.
    pub level: i64,

    /// A string the settings doctest cycles through `add_enum`.
    pub mode: String,

    /// Named flags an example pane reads to decide labels / enabled state.
    pub flags: HashMap<&'static str, bool>,

    /// The framework aggregator. Populated by the keymap builder when a
    /// doctest constructs one; left at `Default` otherwise. Reached via
    /// the framework accessor registered in `KeymapBuilder::with_framework_accessor`.
    pub framework: crate::Framework<DocCtx>,
}

impl DocCtx {
    /// Convenience constructor used inside doctests:
    /// `let mut ctx = DocCtx::new();`
    pub fn new() -> Self { Self::default() }
}
```

`DocCtx` is re-exported from `lib.rs`:

```rust
// tui_pane/src/lib.rs
mod doc_ctx;
pub use doc_ctx::DocCtx;
```

### How a doctest constructs and uses one

```rust
/// ```
/// use tui_pane::DocCtx;
///
/// let mut ctx = DocCtx::new();
/// ctx.flag = true;
/// assert_eq!(ctx.dispatch_count, 0);
/// ```
```

Every doctest in the crate either constructs `DocCtx::new()` directly or — for examples that demonstrate the keymap builder chain — calls a builder helper that takes/returns `DocCtx` (see Section 2). No doctest references `App`, the binary's pane types, or anything outside `tui_pane`.

---

## 2. Doctest patterns

Every snippet below is self-contained: it compiles unmodified as the body of a `///` block on the named item. Each one constructs its own state, performs the operation under test, and asserts an observable outcome.

The `action_enum!` macro is invoked inside doctests (it expands to a `pub enum` plus the `ActionEnum` impl). Where doctests need a concrete pane type, they declare a local zero-sized struct and `impl Shortcuts<DocCtx> for` it inline.

### 2.1 `Shortcuts<Ctx>` trait

On the trait itself:

```rust
/// Per-pane action contract: defaults, per-frame label/state, and a
/// dispatcher fn pointer the framework calls with `&mut Ctx`.
///
/// ```
/// use tui_pane::{
///     action_enum, bindings, BarRow, BarRegion, Bindings, DocCtx,
///     InputMode, KeyBind, Shortcut, Shortcuts,
/// };
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum ExampleAction {
///         Activate => "activate", "Activate the focused row";
///         Cancel   => "cancel",   "Cancel the operation";
///     }
/// }
///
/// pub struct ExamplePane { pub busy: bool }
///
/// fn dispatch_example(action: ExampleAction, ctx: &mut DocCtx) {
///     ctx.dispatch_count += 1;
///     match action {
///         ExampleAction::Activate => ctx.flag = true,
///         ExampleAction::Cancel   => ctx.flag = false,
///     }
/// }
///
/// impl Shortcuts<DocCtx> for ExamplePane {
///     type Action = ExampleAction;
///     const SCOPE_NAME: &'static str = "example";
///
///     fn defaults() -> Bindings<ExampleAction> {
///         bindings! {
///             KeyCode::Enter => ExampleAction::Activate,
///             KeyCode::Esc   => ExampleAction::Cancel,
///         }
///     }
///
///     fn shortcut(&self, action: ExampleAction, _ctx: &DocCtx) -> Option<Shortcut> {
///         match action {
///             ExampleAction::Activate if self.busy => Some(Shortcut::disabled("activate")),
///             ExampleAction::Activate              => Some(Shortcut::enabled("activate")),
///             ExampleAction::Cancel                => Some(Shortcut::enabled("cancel")),
///         }
///     }
///
///     fn dispatcher() -> fn(ExampleAction, &mut DocCtx) { dispatch_example }
/// }
///
/// // Example exercises the dispatcher pointer end-to-end.
/// let mut ctx = DocCtx::new();
/// (<ExamplePane as Shortcuts<DocCtx>>::dispatcher())(ExampleAction::Activate, &mut ctx);
/// assert_eq!(ctx.dispatch_count, 1);
/// assert!(ctx.flag);
/// ```
```

### 2.2 `Navigation<Ctx>` trait

```rust
/// Single-instance app navigation scope. Four required action consts
/// (`UP`/`DOWN`/`LEFT`/`RIGHT`) drive the framework's nav row and vim
/// extras; the dispatcher routes to whichever scrollable owns the focus.
///
/// ```
/// use tui_pane::{
///     action_enum, bindings, Bindings, DocCtx, KeyBind, Navigation, PaneId,
/// };
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum ExampleNav {
///         Up    => "up",    "Move up";
///         Down  => "down",  "Move down";
///         Left  => "left",  "Move left";
///         Right => "right", "Move right";
///     }
/// }
///
/// fn dispatch_nav(action: ExampleNav, _focused: PaneId, ctx: &mut DocCtx) {
///     ctx.dispatch_count += 1;
///     match action {
///         ExampleNav::Up   => ctx.level -= 1,
///         ExampleNav::Down => ctx.level += 1,
///         _ => {}
///     }
/// }
///
/// pub struct ExampleNavScope;
///
/// impl Navigation<DocCtx> for ExampleNavScope {
///     type Action = ExampleNav;
///     const UP:    ExampleNav = ExampleNav::Up;
///     const DOWN:  ExampleNav = ExampleNav::Down;
///     const LEFT:  ExampleNav = ExampleNav::Left;
///     const RIGHT: ExampleNav = ExampleNav::Right;
///
///     fn defaults() -> Bindings<ExampleNav> {
///         bindings! {
///             KeyCode::Up    => ExampleNav::Up,
///             KeyCode::Down  => ExampleNav::Down,
///             KeyCode::Left  => ExampleNav::Left,
///             KeyCode::Right => ExampleNav::Right,
///         }
///     }
///
///     fn dispatcher() -> fn(ExampleNav, PaneId, &mut DocCtx) { dispatch_nav }
/// }
///
/// let mut ctx = DocCtx::new();
/// (<ExampleNavScope as Navigation<DocCtx>>::dispatcher())(
///     ExampleNav::Down, PaneId::FrameworkFixture, &mut ctx,
/// );
/// assert_eq!(ctx.level, 1);
/// ```
```

(`PaneId::FrameworkFixture` is a doctest-only variant added to `PaneId` under `#[cfg(any(test, doc))]` — see Section 3.)

### 2.3 `Globals<Ctx>` trait

```rust
/// Application's extension globals scope (Find, OpenEditor, Rescan, …).
/// Framework's own pane-management globals live in `BaseGlobals`; this
/// trait describes what the *app* adds on top.
///
/// ```
/// use tui_pane::{
///     action_enum, bindings, Bindings, DocCtx, Globals, KeyBind,
/// };
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum ExampleGlobal {
///         Refresh => "refresh", "Reload data";
///         Find    => "find",    "Open finder";
///     }
/// }
///
/// fn dispatch_global(action: ExampleGlobal, ctx: &mut DocCtx) {
///     ctx.dispatch_count += 1;
///     match action {
///         ExampleGlobal::Refresh => ctx.flag = !ctx.flag,
///         ExampleGlobal::Find    => ctx.mode = "find".to_string(),
///     }
/// }
///
/// pub struct ExampleGlobals;
///
/// impl Globals<DocCtx> for ExampleGlobals {
///     type Action = ExampleGlobal;
///
///     fn render_order() -> &'static [ExampleGlobal] {
///         &[ExampleGlobal::Find, ExampleGlobal::Refresh]
///     }
///
///     fn defaults() -> Bindings<ExampleGlobal> {
///         bindings! {
///             KeyBind::ctrl('f') => ExampleGlobal::Find,
///             KeyCode::F(5)      => ExampleGlobal::Refresh,
///         }
///     }
///
///     fn bar_label(action: ExampleGlobal) -> &'static str {
///         match action {
///             ExampleGlobal::Find    => "find",
///             ExampleGlobal::Refresh => "refresh",
///         }
///     }
///
///     fn dispatcher() -> fn(ExampleGlobal, &mut DocCtx) { dispatch_global }
/// }
///
/// assert_eq!(ExampleGlobals::bar_label(ExampleGlobal::Find), "find");
/// let mut ctx = DocCtx::new();
/// (<ExampleGlobals as Globals<DocCtx>>::dispatcher())(ExampleGlobal::Find, &mut ctx);
/// assert_eq!(ctx.mode, "find");
/// ```
```

### 2.4 `bindings!` macro — three forms

On the macro itself, three separate `///` blocks (each is its own doctest):

```rust
/// Single key per action.
///
/// ```
/// use tui_pane::{action_enum, bindings, Bindings, KeyBind};
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum A { Activate => "activate", "Activate"; }
/// }
///
/// let b: Bindings<A> = bindings! {
///     KeyCode::Enter => A::Activate,
/// };
/// assert!(b.contains(KeyBind::from(KeyCode::Enter), A::Activate));
/// ```
```

```rust
/// Multi-key list — every key in the array dispatches the same action.
///
/// ```
/// use tui_pane::{action_enum, bindings, Bindings, KeyBind};
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum N { Up => "up", "Move up"; }
/// }
///
/// let b: Bindings<N> = bindings! {
///     [KeyCode::Up, 'k'] => N::Up,
/// };
/// assert!(b.contains(KeyBind::from(KeyCode::Up), N::Up));
/// assert!(b.contains(KeyBind::from('k'),         N::Up));
/// // First key in the list is the primary (what the bar renders).
/// assert_eq!(b.primary(N::Up), Some(&KeyBind::from(KeyCode::Up)));
/// ```
```

```rust
/// Modifier keys via `KeyBind::shift` / `KeyBind::ctrl`.
///
/// ```
/// use tui_pane::{action_enum, bindings, Bindings, KeyBind};
/// use crossterm::event::KeyCode;
///
/// action_enum! {
///     pub enum G { OpenKeymap => "open_keymap", "Open keymap"; }
/// }
///
/// let b: Bindings<G> = bindings! {
///     KeyBind::ctrl('k')           => G::OpenKeymap,
///     KeyBind::shift(KeyCode::Tab) => G::OpenKeymap,
/// };
/// assert!(b.contains(KeyBind::ctrl('k'),           G::OpenKeymap));
/// assert!(b.contains(KeyBind::shift(KeyCode::Tab), G::OpenKeymap));
/// ```
```

`Bindings<A>::contains` and `Bindings<A>::primary` are inspection helpers added for doctests and unit tests; both are `pub` and trivially implemented over the existing internal map.

### 2.5 `Keymap::<Ctx>::builder(...).register::<P>().build()`

On `KeymapBuilder::build`:

```rust
/// Full builder example — register a single pane, an app navigation scope,
/// an app globals scope, then build the runtime `Keymap<DocCtx>`.
///
/// ```
/// use tui_pane::{
///     action_enum, bindings, BaseGlobalAction, Bindings, DocCtx, Framework,
///     Globals, KeyBind, Keymap, Navigation, PaneId, Shortcut, Shortcuts, VimMode,
/// };
/// use crossterm::event::KeyCode;
///
/// action_enum! { pub enum PaneA { Activate => "activate", "Activate"; } }
/// action_enum! { pub enum NavA  { Up => "up", "Up"; Down => "down", "Down";
///                                Left => "left", "Left"; Right => "right", "Right"; } }
/// action_enum! { pub enum GlobA { Refresh => "refresh", "Refresh"; } }
///
/// pub struct DemoPane;
/// fn dispatch_demo(_a: PaneA, ctx: &mut DocCtx) { ctx.dispatch_count += 1; }
/// impl Shortcuts<DocCtx> for DemoPane {
///     type Action = PaneA;
///     const SCOPE_NAME: &'static str = "demo";
///     fn defaults() -> Bindings<PaneA> { bindings! { KeyCode::Enter => PaneA::Activate, } }
///     fn shortcut(&self, _a: PaneA, _ctx: &DocCtx) -> Option<Shortcut> {
///         Some(Shortcut::enabled("activate"))
///     }
///     fn dispatcher() -> fn(PaneA, &mut DocCtx) { dispatch_demo }
/// }
///
/// pub struct DemoNav;
/// fn dispatch_nav(_a: NavA, _f: PaneId, ctx: &mut DocCtx) { ctx.dispatch_count += 1; }
/// impl Navigation<DocCtx> for DemoNav {
///     type Action = NavA;
///     const UP:    NavA = NavA::Up;
///     const DOWN:  NavA = NavA::Down;
///     const LEFT:  NavA = NavA::Left;
///     const RIGHT: NavA = NavA::Right;
///     fn defaults() -> Bindings<NavA> {
///         bindings! { KeyCode::Up => NavA::Up, KeyCode::Down => NavA::Down,
///                     KeyCode::Left => NavA::Left, KeyCode::Right => NavA::Right, }
///     }
///     fn dispatcher() -> fn(NavA, PaneId, &mut DocCtx) { dispatch_nav }
/// }
///
/// pub struct DemoGlobals;
/// fn dispatch_global(_a: GlobA, ctx: &mut DocCtx) { ctx.dispatch_count += 1; }
/// impl Globals<DocCtx> for DemoGlobals {
///     type Action = GlobA;
///     fn render_order() -> &'static [GlobA] { &[GlobA::Refresh] }
///     fn defaults() -> Bindings<GlobA> { bindings! { KeyCode::F(5) => GlobA::Refresh, } }
///     fn bar_label(_a: GlobA) -> &'static str { "refresh" }
///     fn dispatcher() -> fn(GlobA, &mut DocCtx) { dispatch_global }
/// }
///
/// fn quit(ctx: &mut DocCtx)    { ctx.dispatch_count += 1; }
/// fn restart(ctx: &mut DocCtx) { ctx.dispatch_count += 1; }
/// fn dismiss(ctx: &mut DocCtx) { ctx.dispatch_count += 1; }
///
/// let keymap: Keymap<DocCtx> = Keymap::<DocCtx>::builder(quit, restart, dismiss)
///     .vim_mode(VimMode::Disabled)
///     .with_framework_accessor(|ctx: &mut DocCtx| &mut ctx.framework)
///     .register::<DemoPane>()
///     .with_navigation::<DemoNav>()
///     .with_globals::<DemoGlobals>()
///     .build();
///
/// // The pane scope is registered and its primary key is reachable.
/// assert!(keymap.scope_for::<DemoPane>().key_for(PaneA::Activate).is_some());
/// ```
```

### 2.6 `KeyBind` — `From<KeyCode>`, `From<char>`, `shift`, `ctrl`

```rust
/// Construction from a `KeyCode`.
///
/// ```
/// use tui_pane::KeyBind;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let kb: KeyBind = KeyCode::Enter.into();
/// assert_eq!(kb.code, KeyCode::Enter);
/// assert_eq!(kb.mods, KeyModifiers::NONE);
/// ```
```

```rust
/// Construction from a character.
///
/// ```
/// use tui_pane::KeyBind;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let kb: KeyBind = 'c'.into();
/// assert_eq!(kb.code, KeyCode::Char('c'));
/// assert_eq!(kb.mods, KeyModifiers::NONE);
/// ```
```

```rust
/// Modifier helpers: `shift` and `ctrl`.
///
/// ```
/// use tui_pane::KeyBind;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let s = KeyBind::shift('g');
/// assert_eq!(s.code, KeyCode::Char('g'));
/// assert_eq!(s.mods, KeyModifiers::SHIFT);
///
/// let c = KeyBind::ctrl(KeyCode::Tab);
/// assert_eq!(c.code, KeyCode::Tab);
/// assert_eq!(c.mods, KeyModifiers::CONTROL);
/// ```
```

### 2.7 `BarRow::Single` and `BarRow::Paired`

On the `BarRow` enum:

```rust
/// A single-action row — every key bound to `action` is rendered, joined
/// by `,`, followed by the pane's label.
///
/// ```
/// use tui_pane::{action_enum, BarRow};
///
/// action_enum! { pub enum A { Activate => "activate", "Activate"; } }
///
/// let row: BarRow<A> = BarRow::Single(A::Activate);
/// match row {
///     BarRow::Single(a) => assert_eq!(a, A::Activate),
///     _ => unreachable!(),
/// }
/// ```
```

```rust
/// A paired row — two actions glued with `/`, sharing one label. The
/// framework renders only each action's *primary* key.
///
/// ```
/// use tui_pane::{action_enum, BarRow};
///
/// action_enum! {
///     pub enum N { Up => "up", "Up"; Down => "down", "Down"; }
/// }
///
/// let row: BarRow<N> = BarRow::Paired(N::Up, N::Down, "nav");
/// match row {
///     BarRow::Paired(left, right, label) => {
///         assert_eq!(left, N::Up);
///         assert_eq!(right, N::Down);
///         assert_eq!(label, "nav");
///     }
///     _ => unreachable!(),
/// }
/// ```
```

---

## 3. Private `test_support` module

Location: `tui_pane/src/test_support/mod.rs` (directory form — content grows past one file's worth quickly). Declared inside `lib.rs` as:

```rust
#[cfg(test)]
mod test_support;
```

Visibility throughout: `pub(crate)`. Nothing in this module is reachable from doctests or downstream crates. (Doctests use `DocCtx` from the public surface; they never need `test_support`.)

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
└── ctx.rs              # build_doc_ctx(), assert_dispatch_count(...)
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

// Test-only PaneId variant used by mock panes and by `Navigation`
// doctests that need to name a focused pane without registering one.
// The real `PaneId` enum lives in `framework.rs`; this `cfg(test)`
// block re-opens it via a const alias the test_support code uses.
//
// (If PaneId becomes non-exhaustive, this can move to a real variant
// guarded by `#[cfg(any(test, doc))]` directly on the enum.)
```

#### `mock_panes.rs`

```rust
use crate::{
    action_enum, bindings, BarRegion, BarRow, Bindings, DocCtx, InputMode,
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

pub(crate) fn dispatch_mock(action: MockAction, ctx: &mut DocCtx) {
    ctx.dispatch_count += 1;
    match action {
        MockAction::Activate => ctx.flag = true,
        MockAction::Cancel   => ctx.flag = false,
    }
}

impl Shortcuts<DocCtx> for MockPane {
    type Action = MockAction;
    const SCOPE_NAME: &'static str = "mock";

    fn defaults() -> Bindings<MockAction> {
        bindings! {
            KeyCode::Enter => MockAction::Activate,
            KeyCode::Esc   => MockAction::Cancel,
        }
    }

    fn shortcut(&self, action: MockAction, _ctx: &DocCtx) -> Option<Shortcut> {
        match (action, self.busy) {
            (MockAction::Activate, true)  => Some(Shortcut::disabled("activate")),
            (MockAction::Activate, false) => Some(Shortcut::enabled("activate")),
            (MockAction::Cancel,   _)     => Some(Shortcut::enabled("cancel")),
        }
    }

    fn dispatcher() -> fn(MockAction, &mut DocCtx) { dispatch_mock }
}

// Variants with non-default input_mode for region-suppression tests.
pub(crate) struct MockTextInputPane;
impl Shortcuts<DocCtx> for MockTextInputPane { /* … input_mode = TextInput */ }

pub(crate) struct MockStaticPane;
impl Shortcuts<DocCtx> for MockStaticPane { /* … input_mode = Static */ }

// Pane that emits paired Nav rows — exercises ProjectList-style layout
// without needing the binary's ProjectListPane.
pub(crate) struct MockPairedNavPane;
impl Shortcuts<DocCtx> for MockPairedNavPane {
    /* … overrides bar_rows to emit Paired rows in BarRegion::Nav */
}
```

#### `builders.rs`

```rust
use crate::{DocCtx, Keymap, VimMode};

pub(crate) fn noop_quit(_ctx: &mut DocCtx)    {}
pub(crate) fn noop_restart(_ctx: &mut DocCtx) {}
pub(crate) fn noop_dismiss(_ctx: &mut DocCtx) {}

/// Smallest valid keymap: no panes, no nav, no globals — just the three
/// required builder positionals. Useful for tests of `BaseGlobals` and
/// scope lookup edge cases.
pub(crate) fn build_minimal_keymap() -> Keymap<DocCtx> {
    Keymap::<DocCtx>::builder(noop_quit, noop_restart, noop_dismiss)
        .vim_mode(VimMode::Disabled)
        .with_framework_accessor(|ctx: &mut DocCtx| &mut ctx.framework)
        .build()
}

/// Keymap with the standard mock pane registered, plus a closure for
/// further customization (TOML loading, vim mode toggle, extra panes).
pub(crate) fn build_keymap_with<F>(customize: F) -> Keymap<DocCtx>
where
    F: FnOnce(crate::KeymapBuilder<DocCtx>) -> crate::KeymapBuilder<DocCtx>,
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
use crate::DocCtx;

pub(crate) fn build_doc_ctx() -> DocCtx { DocCtx::new() }

pub(crate) fn assert_dispatch_count(ctx: &DocCtx, expected: u32) {
    assert_eq!(
        ctx.dispatch_count, expected,
        "dispatch_count mismatch: got {}, expected {}",
        ctx.dispatch_count, expected,
    );
}
```

### What does NOT live in `test_support`

- The public fixture `DocCtx` (it lives at `tui_pane/src/doc_ctx.rs` because doctests need it; `test_support` is `cfg(test)` and unreachable from doctests).
- Snapshot baselines — those live alongside the bar tests (per the snapshot crate's convention) under `tui_pane/src/bar/snapshots/`.
- Anything app-specific (no `App`, no cargo-port pane types, no cargo-port action enums).

---

## 4. Unit test placement

**Rule:** unit tests live next to their module via `#[cfg(test)] mod tests` at the bottom of the same file (or a sibling `tests.rs` if the module is a directory). Tests reach for fixtures via `use crate::test_support::…`.

Modules with non-trivial coverage needs (each owns its own `#[cfg(test)] mod tests`):

| Module                          | Coverage focus                                                                                                  |
|---------------------------------|------------------------------------------------------------------------------------------------------------------|
| `keymap/key_bind.rs`            | `From<KeyCode>` / `From<char>` / `shift` / `ctrl`; `display_short` for every `KeyCode` variant (paired-slot rule); round-trip parsing. |
| `keymap/bindings.rs`            | `bindings!` macro for each form (single, list, modifier); insertion order; `bind` vs `bind_many` parity.        |
| `keymap/scope_map.rs`           | `insert` rejects same-key→different-action; `by_key.len() == sum(by_action.values().len())`; `key_for` returns insertion-order primary; `display_keys_for` order. |
| `keymap/load.rs`                | Each `samples.rs` constant produces the documented outcome; replace-not-merge semantics; `config_path` return value per platform. |
| `keymap/vim.rs`                 | Vim extras layered after TOML; arrows remain primary; `vim_mode_conflicts` detection; `is_vim_reserved` reads live `Navigation` scope. |
| `keymap/builder.rs`             | Required positionals (`quit`, `restart`, `dismiss`); registration order; `with_framework_accessor` round-trips. |
| `keymap/base_globals.rs`        | Framework-owned defaults (`q`, `R`, `Tab`, etc.); `Dismiss` calls injected hook; `OpenKeymap`/`OpenSettings` reach the framework panes. |
| `bar/region.rs`                 | `BarRegion::ALL` order matches render walk.                                                                     |
| `bar/shortcut.rs`               | `Shortcut::enabled` / `disabled` constructors; `BarRow` exhaustive match.                                       |
| `bar/support.rs`                | `format_action_keys` joins via `,`; `push_cancel_row` produces the expected row.                                                    |
| `bar/nav_region.rs`             | `Navigable`/`TextInput`/`Static` suppression; pane-cycle row pulled from `BaseGlobalAction::NextPane`.          |
| `bar/pane_action_region.rs`     | Per-action labels read from `Shortcut`; `ShortcutState::Disabled` rendering.                                    |
| `bar/global_region.rs`          | `BaseGlobals` first, `AppGlobals::render_order()` after; `TextInput` suppression.                               |
| `bar/mod.rs`                    | End-to-end snapshot per fixture pane (drives `MockPane`, `MockTextInputPane`, `MockStaticPane`, `MockPairedNavPane`). |
| `panes/keymap_pane.rs`          | Mode transitions Browse → Awaiting → Conflict; `editor_target` reflects current state.                          |
| `panes/settings_pane.rs`        | Mode transitions Browse ↔ Editing; `is_text_input` flips correctly; `add_bool`/`add_enum`/`add_int` round-trips. |
| `panes/toasts.rs`               | Push/pop stack; `Dismiss` reaches the registered framework accessor.                                            |
| `settings.rs`                   | Each value flavor's get/set closure round-trips through `DocCtx`.                                               |
| `framework.rs`                  | `editor_target_path` per `PaneId`; `focused_pane_input_mode` falls back to `Navigable` for unregistered panes.  |
| `doc_ctx.rs`                    | None — the type is too small for unit tests; doctests cover its construction.                                   |

The full module set under `tui_pane/src/` matches the layout in `tui-pane-lib.md` lines 19-70; every leaf file gets a `#[cfg(test)] mod tests` block.

---

## 5. Cross-module integration tests

**Recommendation: yes, create `tui_pane/tests/` — one integration file per cross-cutting scenario.**

`#[cfg(test)] mod tests` covers the inside view (private fields, `pub(crate)` constructors, internal invariants). `tui_pane/tests/` covers the outside view: the same surface a downstream crate sees. Because doctests already exercise the public API, the integration test directory targets scenarios doctests are too small to cover.

Recommended files under `tui_pane/tests/`:

| File                         | Scenario                                                                                                       |
|------------------------------|----------------------------------------------------------------------------------------------------------------|
| `tests/builder_full.rs`      | Construct a full `Keymap<DocCtx>` via the public surface only (no `test_support` reach-through); register multiple panes; load each `SAMPLE_TOML_*` constant (re-declared in this file or read from a fixtures dir); assert end-to-end resolution. |
| `tests/dispatch_routing.rs`  | Drive a `KeyEvent` through `Keymap::action_for`, then through `Shortcuts::dispatcher`, asserting `DocCtx` mutations. Pane-scope-wins-over-navigation precedence test (Right → `ProjectListAction::ExpandRow` analog beats `NavigationAction::Right`). |
| `tests/bar_rendering.rs`     | Snapshot the full bar for a fixture pane against each `InputMode`. Keeps the snapshot crate (likely `insta`) confined to one integration file rather than scattered across module tests. |
| `tests/vim_mode.rs`          | Toggle `VimMode::Enabled` on a built keymap; assert `'h'/'j'/'k'/'l'` reach `NavigationAction::*`; assert TOML user-replace doesn't disable vim. |
| `tests/toml_errors.rs`       | Each documented error path (`SAMPLE_TOML_DUPLICATE_IN_ARRAY`, `SAMPLE_TOML_CROSS_ACTION_DUP`, `SAMPLE_TOML_BAD_KEY_NAME`) returns `Err` with the documented variant. |

Integration tests cannot reach `test_support` (it's `pub(crate)`), so each integration file declares its own minimal helpers — usually just a couple of free fns (`build_keymap()`, `noop_dispatcher`). That separation is intentional: integration tests verify the public API without privileged access.

### What `tests/` does NOT cover

- Exhaustive per-`KeyCode` `display_short` walk — that's a tight loop, lives in `keymap/key_bind.rs::tests`.
- Internal invariants (`by_key.len()` sum) — `tests/` cannot see private fields.
- Macro expansion correctness — covered by doctests on `bindings!` and `action_enum!`.

---

## 6. Boundary with the binary's `tui_test_support`

The binary already owns `src/tui/tui_test_support.rs` (per Phase 10 of `tui-pane-lib.md`, line 1075). It contains `pub(super) fn make_app` and binary-only test fixtures (`App` constructors, project-tree fixtures, etc.).

Confirmation:

- `tui_pane::test_support` is `#[cfg(test)] pub(crate)`. It cannot leak across the workspace boundary — even `dev-dependencies` cannot reach a `cfg(test)`-gated module of an upstream crate. The binary's tests literally cannot `use tui_pane::test_support::…`; the path does not resolve.
- Conversely, `src/tui/tui_test_support.rs` is `pub(super)` inside the binary crate; `tui_pane` cannot depend on the binary crate (the dependency arrow only goes binary → library), so there is no path by which `tui_pane`'s tests could import binary fixtures.
- The two fixture sets are disjoint by language rule, not by convention. No README warning is needed.

What the binary *can* reach from `tui_pane`:

- `tui_pane::DocCtx` — the public fixture. The binary's tests have no reason to construct `DocCtx`; they construct `App`. If a binary test ever does construct `DocCtx`, that's a smell that the test should live in `tui_pane` instead. (Worth flagging in PR review, not enforceable by the compiler.)
- All other `pub` items — `Keymap`, `KeymapBuilder`, traits, macros, `KeyBind`, etc. The binary uses these in production code; binary tests use them via `make_app()` returning a real `App` with a real `Keymap<App>`.

What the binary's tests gain that `tui_pane`'s tests cannot replicate:

- End-to-end key-event → pane-state-mutation chains against the *real* `App`. These are intentionally binary-side: they exercise the wiring between cargo-port's panes and the framework, not the framework's own correctness.

### Summary of the split

| Layer                     | Test code lives in                                                  | Fixture source                                |
|---------------------------|---------------------------------------------------------------------|-----------------------------------------------|
| Framework module-internal | `tui_pane/src/**/*.rs` `#[cfg(test)] mod tests`                     | `tui_pane::test_support` (`pub(crate)`)       |
| Framework public surface  | `tui_pane/tests/*.rs`                                               | Inline helpers + `tui_pane::DocCtx`           |
| Framework doctests        | `///` blocks on every `pub` item in `tui_pane`                      | `tui_pane::DocCtx` only                       |
| Binary module-internal    | `src/**/*.rs` `#[cfg(test)] mod tests`                              | `src/tui/tui_test_support.rs` (`pub(super)`)  |
| Binary integration        | `tests/*.rs` (binary crate)                                         | `make_app()` from `tui_test_support`          |

Five rows, two crates, zero overlap. Each fixture set serves exactly the layer it sits in.
