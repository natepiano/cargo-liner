# `tui_pane` — Formal Public API Specification

This file consolidates the public surface of the `tui_pane` crate as it is sketched throughout `docs/tui-pane-lib.md`. Every section below is the canonical signature for the named type or trait. Where the design doc left a choice open, this spec makes the call inline (one sentence) and proceeds.

The crate is generic over a single app context type `Ctx` (the binary supplies `type Ctx = App;`). Every callback the framework invokes while it holds a live `&mut Ctx` borrow is a free function pointer. Settings get/set are closures because the closure itself *is* the borrow holder.

---

## 1. `KeyBind` and friends

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use thiserror::Error;

/// A single keystroke: a `KeyCode` plus its modifier flags.
///
/// Values are constructed with `From<KeyCode>` / `From<char>` /
/// `From<KeyEvent>` for the conversion cases, and the `shift` / `ctrl`
/// constructors when modifiers matter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyBind {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl From<KeyCode> for KeyBind {
    /// `KeyCode::Enter` becomes `KeyBind { code: Enter, mods: NONE }`.
    fn from(code: KeyCode) -> Self {
        Self { code, mods: KeyModifiers::NONE }
    }
}

impl From<char> for KeyBind {
    /// `'c'` becomes `KeyBind { code: Char('c'), mods: NONE }`.
    fn from(c: char) -> Self {
        Self { code: KeyCode::Char(c), mods: KeyModifiers::NONE }
    }
}

/// Kind-tagged keyboard event. The framework's event-loop entry converts
/// each crossterm `KeyEvent` into a `KeyInput`; downstream dispatch
/// pattern-matches on the variant. Keymap dispatch only handles
/// `KeyInput::Press`. `state` (CapsLock / NumLock) is dropped.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KeyInput {
    Press(KeyBind),
    Release(KeyBind),
    Repeat(KeyBind),
}

impl KeyInput {
    pub const fn from_event(event: KeyEvent) -> Self { /* see source */ }
    pub const fn bind(&self)  -> &KeyBind          { /* … */ }
    pub const fn press(&self) -> Option<&KeyBind>  { /* Some(b) only on Press */ }
}

impl KeyBind {
    /// Shift-modified bind. `KeyBind::shift('g')` → `KeyBind { Char('g'), SHIFT }`.
    /// Accepts anything convertible to `KeyBind` (`KeyCode` or `char`);
    /// composes with `ctrl` (`KeyBind::ctrl(KeyBind::shift('g'))` → `Char('g') + CONTROL | SHIFT`).
    #[must_use]
    pub fn shift(into: impl Into<Self>) -> Self {
        let kb = into.into();
        Self { code: kb.code, mods: kb.mods | KeyModifiers::SHIFT }
    }

    /// Control-modified bind. `KeyBind::ctrl('k')` → `KeyBind { Char('k'), CONTROL }`.
    /// Same input/composition rules as `shift`.
    #[must_use]
    pub fn ctrl(into: impl Into<Self>) -> Self {
        let kb = into.into();
        Self { code: kb.code, mods: kb.mods | KeyModifiers::CONTROL }
    }

    /// Full display name, e.g. `"Up"`, `"Enter"`, `"Esc"`, `"Ctrl+K"`,
    /// `"Shift+Tab"`. Modifier order is canonical `Ctrl+Alt+Shift+key`.
    /// Used by the keymap-overlay help screen.
    #[must_use]
    pub fn display(&self) -> String { /* see tui_pane/src/keymap/key_bind.rs */ }

    /// Compact display: arrow keys render as glyphs (`↑`, `↓`, `←`, `→`),
    /// every other key delegates to `display`. Used by the status bar.
    /// Must not produce a string containing `,` or `/` for any key the
    /// parser can produce — `bar/` `Paired` row `debug_assert!`s this.
    #[must_use]
    pub fn display_short(&self) -> String { /* see source */ }

    /// Parse a TOML-style key string (e.g. `"Enter"`, `"Ctrl+K"`,
    /// `"Shift+Tab"`, `"+"`, `"="`). The pre-refactor `+`/`=` collapse
    /// is dropped — `"="` parses to `KeyCode::Char('=')` and `"+"` to
    /// `KeyCode::Char('+')`. Accepts `"Control"` as a synonym for `"Ctrl"`,
    /// and `"Space"` for `KeyCode::Char(' ')`. Letter case is preserved
    /// verbatim — `"Ctrl+K"` parses to `Char('K')`, not `Char('k')`. Any
    /// case-insensitive lookup is a keymap-layer concern (Phase 9 loader),
    /// not a parser concern.
    pub fn parse(s: &str) -> Result<Self, KeyParseError> { /* see source */ }
}

/// Error returned by `KeyBind::parse`.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum KeyParseError {
    /// Input was the empty string.
    #[error("empty key string")]
    Empty,
    /// Input contained a key token that was neither a recognized name
    /// nor a single character.
    #[error("unknown key: {0:?}")]
    UnknownKey(String),
    /// Input contained a modifier token that was not `Ctrl` / `Control`
    /// / `Shift` / `Alt`.
    #[error("unknown modifier: {0:?}")]
    UnknownModifier(String),
}
```

**Tradeoff.** `shift` / `ctrl` take `impl Into<Self>` rather than `impl Into<KeyCode>` because crossterm's `KeyCode` does not implement `From<char>` (and orphan rules block adding it). Routing through `Into<KeyBind>` reuses the two `From` impls and makes the constructors composable. `KeyParseError` derives `thiserror::Error` so downstream wrappers (e.g. `KeymapError`) get automatic `#[source]` chaining via `#[from]`.

**Press/Release/Repeat split.** Crossterm hands us a single `KeyEvent` with a `kind: KeyEventKind` discriminant. Rather than a lossy `From<KeyEvent> for KeyBind` (which silently drops `kind` and lets `Release` events fire keymap actions), the framework uses a separate kind-tagged enum `KeyInput` that the event loop produces. Keymap dispatch sites pattern-match `KeyInput::Press(bind)` (or call `.press()`) — Release / Repeat events flow through unchanged for handlers that opt in. Modeled after Zed/GPUI (which type-splits `KeyDownEvent` / `KeyUpEvent`) and bevy_enhanced_input (which lifts the press-state lifecycle into the action API). Crossterm doesn't give us either affordance directly, so we add the type discipline at our boundary.

---

## 2. `Bindings<A>` builder

```rust
/// Default-key declaration for a single scope. Built from a pane's
/// `Shortcuts::defaults` impl (typically via the `bindings!` macro),
/// then folded into the keymap during `KeymapBuilder::build()`.
///
/// Insertion order is significant: the **first** key bound to an action
/// is the action's primary key (what the bar shows when only one key
/// fits). Tests lock this for arrow-vs-vim conflicts.
pub struct Bindings<A> {
    entries: Vec<(KeyBind, A)>,
}

impl<A> Bindings<A> {
    /// Empty builder.
    pub fn new() -> Self { Self { entries: Vec::new() } }

    /// Bind one key to one action. Multiple `bind` calls with the same
    /// action append additional keys; the first call's key is primary.
    pub fn bind(&mut self, key: impl Into<KeyBind>, action: A) -> &mut Self {
        self.entries.push((key.into(), action));
        self
    }

    /// Bind every key in the iterator to the same action, in order.
    /// The first key in the iterator becomes the primary if the action
    /// has no prior binding.
    pub fn bind_many(
        &mut self,
        keys: impl IntoIterator<Item = KeyBind>,
        action: A,
    ) -> &mut Self {
        for k in keys { self.entries.push((k, action.clone())); }
        self
    }

    /// Consume the builder and return the fully indexed `ScopeMap<A>`.
    /// Panics on cross-action collision under `debug_assertions`
    /// (defaults are author-controlled; collisions are bugs, not user
    /// input). User-supplied TOML goes through `load.rs`, which returns
    /// `Result` for the same condition.
    pub fn into_scope_map(self) -> ScopeMap<A>
    where A: Copy + Eq + std::hash::Hash {
        let mut map = ScopeMap::new();
        for (k, a) in self.entries { map.insert(k, a); }
        map
    }
}

impl<A> Default for Bindings<A> {
    fn default() -> Self { Self::new() }
}

impl<A: Clone> Clone for Bindings<A> {
    fn clone(&self) -> Self { Self { entries: self.entries.clone() } }
}
```

`Bindings<A>` is `Default` (free `new()` mirror, idiomatic) and `Clone` when `A: Clone` (lets a pane reuse a defaults table as a starting point for tests). It is **not** `Copy`: the inner `Vec` precludes it, and the framework never needs `Copy` since `into_scope_map` consumes.

### `bindings!` macro

The macro accepts the syntax `KEY => ACTION` (single key) and `[KEY, KEY, …] => ACTION` (multi-key list). Each `KEY` is any `impl Into<KeyBind>` expression — `KeyCode::Enter`, `'c'`, `KeyBind::shift('g')`, `KeyBind::ctrl('k')`. The macro expands to:

```rust
// bindings! { KeyCode::Enter => PackageAction::Activate, ['=', '+'] => ProjectListAction::ExpandAll, … }
// expands to:
{
    let mut __b: Bindings<_> = Bindings::new();
    __b.bind(KeyCode::Enter, PackageAction::Activate);
    __b.bind_many(
        [KeyBind::from('='), KeyBind::from('+')],
        ProjectListAction::ExpandAll,
    );
    // …
    __b
}
```

**Tradeoff.** Macro returns `Bindings<A>` (not `ScopeMap<A>`) so a pane impl can layer extra binds before yielding to the framework. The framework calls `into_scope_map` itself during `build()`.

---

## 3. `ScopeMap<A>`

```rust
use std::collections::HashMap;
use std::hash::Hash;

/// Resolved binding table for a single scope.
///
/// Two indexes:
///   - `by_key`:    1-to-1 within a scope. Used by the dispatcher.
///   - `by_action`: 1-to-many. Insertion order preserved per action;
///                  the first entry in each `Vec<KeyBind>` is the
///                  action's primary key (used by the bar).
///
/// Invariant locked by tests:
///     by_key.len() == by_action.values().map(Vec::len).sum::<usize>()
/// — every key in `by_key` appears exactly once across all `by_action`
/// vectors. No orphans, no double-counts.
pub struct ScopeMap<A: Copy + Eq + Hash> {
    by_key:    HashMap<KeyBind, A>,
    by_action: HashMap<A, Vec<KeyBind>>,
}

impl<A: Copy + Eq + Hash> ScopeMap<A> {
    /// Empty map. `pub(super)` — only `Bindings::into_scope_map` and the
    /// TOML loader (sibling modules in `keymap/`) construct one;
    /// consumers always receive a built map from the `Keymap`.
    pub(super) fn new() -> Self {
        Self { by_key: HashMap::new(), by_action: HashMap::new() }
    }

    /// Insert one (key, action) pair.
    ///
    /// `pub(super)` — same reason as `new`.
    ///
    /// `debug_assert!`s that `key` is either unbound or already bound
    /// to the same `action`. Cross-action collisions inside one scope
    /// are bugs in `defaults()`; the TOML loader catches the same
    /// condition for user input and returns `Err` instead of panicking.
    pub(super) fn insert(&mut self, key: KeyBind, action: A) {
        debug_assert!(
            self.by_key.get(&key).is_none_or(|&existing| existing == action),
            "ScopeMap::insert: key {key:?} already maps to a different action",
        );
        if self.by_key.insert(key, action).is_none() {
            self.by_action.entry(action).or_default().push(key);
        }
    }

    /// Dispatcher lookup. `pub` — every input handler calls this.
    pub fn action_for(&self, key: &KeyBind) -> Option<A> {
        self.by_key.get(key).copied()
    }

    /// Primary-key lookup (the first key bound to `action`). `pub` —
    /// the bar reads this when rendering a `BarSlot::Paired` slot.
    pub fn key_for(&self, action: A) -> Option<&KeyBind> {
        self.by_action.get(&action).and_then(|v| v.first())
    }

    /// Display the primary key, full name (`"Up"`, `"Ctrl+K"`).
    /// `pub` — keymap-overlay help screen renders these.
    pub fn display_key_for(&self, action: A) -> String {
        self.key_for(action).map(KeyBind::display).unwrap_or_default()
    }

    /// All keys bound to `action`, insertion order. `pub` — the bar's
    /// `Single` row joins these with `,` after `display_short`.
    /// Returns an empty slice for unbound actions.
    pub fn display_keys_for(&self, action: A) -> &[KeyBind] {
        self.by_action.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }
}
```

**Tradeoff.** `new` and `insert` are `pub(super)` — the framework owns construction. App code only ever receives a built `&ScopeMap<A>` via `Keymap::scope_for`, so a public mutable surface would be a hazard with no caller. `pub(super)` (not `pub(crate)`) per Phase 5+ standing rule 3: in nested modules, narrow to siblings. `action_for` / `key_for` / `display_key_for` / `display_keys_for` are all `pub` because the bar code (in `tui_pane`) and the input handlers (in the binary) both call them.

---

## 4. `Shortcuts<Ctx>`, `Navigation<Ctx>`, `Globals<Ctx>` traits

```rust
/// Marker trait every action enum implements (typically via the
/// `action_enum!` macro). Provides a static `ALL` slice for
/// declaration-order rendering and the per-variant TOML key + label.
///
/// Super-trait set (matches Phase 3 shipped code + `macros.md`
/// §2.3): `Copy + Eq + Hash + Debug + Display + 'static`. The macro
/// generates the `Display` impl (delegating to `description()`); hand-
/// rolled impls (e.g. `GlobalAction`) must do the same.
pub trait Action:
    Copy + Eq + std::hash::Hash + std::fmt::Debug + std::fmt::Display + 'static
{
    /// Every variant in declaration order.
    const ALL: &'static [Self];

    /// TOML key for this variant (e.g. `"activate"`).
    fn toml_key(self) -> &'static str;

    /// Default short label rendered in the bar (e.g. `"activate"`,
    /// `"clean"`). The bar renderer reads this directly through
    /// default; overrides only fire when the label is state-dependent.
    fn bar_label(self) -> &'static str;

    /// Human-readable description (used by the keymap-overlay help).
    fn description(self) -> &'static str;

    /// Inverse of `toml_key`. Returns `None` for unknown identifiers;
    /// the TOML loader attaches scope context and surfaces a
    /// `KeymapError::UnknownAction`.
    fn from_toml_key(key: &str) -> Option<Self>;
}

/// Pane scope: state-bearing, one impl per pane type.
///
/// `'static` super-trait bound is required because the framework keys
/// the registry on `TypeId<P>` and stores `fn` pointers — both demand
/// `'static`. Pane *instances* live on the binary's `App`; the trait
/// impl itself never holds borrowed data.
///
/// `Pane<Ctx>` carries the per-pane identity and the per-frame mode
/// query. `Shortcuts<Ctx>: Pane<Ctx>` adds the shortcut-config surface.
/// Splitting them lets a future per-pane trait (e.g. `MouseInput<Ctx>:
/// Pane<Ctx>`) attach without bloating `Shortcuts`, and lets a pane
/// without shortcuts (e.g. a pure text-input overlay) impl just `Pane`.
pub trait Pane<Ctx: AppContext>: 'static {
    /// Stable per-pane identity used by the framework's per-pane
    /// query registry (populated through `Framework::register_app_pane`).
    /// The trait covers app panes only — framework panes (Keymap,
    /// Settings, Toasts) are special-cased — so the variant is always
    /// an `AppPaneId`.
    const APP_PANE_ID: Ctx::AppPaneId;

    /// Per-frame mode. Drives bar-region suppression, structural Esc
    /// gate, and key dispatch. The `TextInput(handler)` variant carries
    /// the handler inline so "TextInput without a handler" is
    /// unrepresentable.
    ///
    /// Returns a `fn(&Ctx) -> Mode<Ctx>` so the framework can store
    /// the pointer in `Framework<Ctx>::mode_queries`, keyed by
    /// `AppPaneId`, populated through `Framework::register_app_pane`
    /// at `KeymapBuilder::build_into` time. The framework holds `&Ctx`
    /// and an `AppPaneId` at query time, never a typed `&PaneStruct`,
    /// so the closure does the navigation from `Ctx` to whatever state
    /// determines the mode.
    ///
    /// Default returns `Mode::Navigable`. Panes whose mode varies with
    /// `Ctx` state (Finder, Output, Settings) override.
    fn mode() -> fn(&Ctx) -> Mode<Ctx> {
        |_ctx| Mode::Navigable
    }
}

pub trait Shortcuts<Ctx: AppContext>: Pane<Ctx> {
    /// The pane's action enum.
    type Actions: Action;

    /// TOML table name. Survives type renames; one-line cost. Required.
    const SCOPE_NAME: &'static str;

    /// Default keybindings. No framework default — every pane declares
    /// its own keys.
    fn defaults() -> Bindings<Self::Actions>;

    /// Per-action visibility. `Visible` (default) renders the slot;
    /// `Hidden` removes it. The bar **label** is always
    /// `Action::bar_label()` declared in `action_enum!` — no per-frame
    /// label override on the trait. Override `visibility` when the
    /// slot's presence is state-dependent (e.g. `CiRunsAction::Activate`
    /// `Hidden` at EOL).
    fn visibility(&self, _action: Self::Actions, _ctx: &Ctx) -> Visibility {
        Visibility::Visible
    }

    /// Per-action enabled / disabled status. Default `Enabled`.
    /// Override when the action is visible but inert (e.g.
    /// `PackageAction::Clean` grayed out when no target dir exists).
    fn state(&self, _action: Self::Actions, _ctx: &Ctx) -> ShortcutState {
        ShortcutState::Enabled
    }

    /// Bar slot layout. Default: one `(PaneAction, Single(action))` per
    /// `Self::Actions::ALL` in declaration order. Override to introduce
    /// `Paired` slots, route into `BarRegion::Nav`, or omit
    /// data-dependent slots.
    fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        Self::Actions::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Optional vim-extras: pane actions that gain a vim binding when
    /// `VimMode::Enabled`. Default empty. Used by
    /// `ProjectListAction::ExpandRow` (`'l'`) and `CollapseRow` (`'h'`).
    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &[] }

    /// Free-function dispatcher. Framework calls
    /// `Self::dispatcher()(action, ctx)` while holding `&mut Ctx`.
    /// Implementations navigate from the `Ctx` root rather than
    /// holding a `&mut self` borrow (split-borrow rule).
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
}

/// Navigation scope. One impl per app (the binary defines a single
/// `AppNavigation` zero-sized type and impls this trait for it).
///
/// `'static` is implied by the `Action` bound on `Actions`.
pub trait Navigation<Ctx: AppContext>: 'static {
    type Actions: Action;

    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Actions;
    const DOWN:  Self::Actions;
    const LEFT:  Self::Actions;
    const RIGHT: Self::Actions;
    const HOME:  Self::Actions;
    const END:   Self::Actions;

    fn defaults() -> Bindings<Self::Actions>;

    /// Translate a resolved navigation action into framework-owned
    /// `ListNavigation`. Default impl matches against the trait's
    /// `UP`/`DOWN`/`HOME`/`END` constants; returns `None` for
    /// `LEFT`/`RIGHT` and any app-specific variants. Phase 12 dispatch
    /// calls this when focus is
    /// `FocusedPane::Framework(FrameworkFocusId::Toasts)` to translate
    /// the binary's `Self::Actions` into the framework's
    /// `ListNavigation` before calling `Toasts::on_navigation`.
    fn list_navigation(action: Self::Actions) -> Option<ListNavigation> {
        if action == Self::UP { Some(ListNavigation::Up) }
        else if action == Self::DOWN { Some(ListNavigation::Down) }
        else if action == Self::HOME { Some(ListNavigation::Home) }
        else if action == Self::END { Some(ListNavigation::End) }
        else { None }
    }

    /// Free fn the framework calls when any navigation action fires.
    /// `focused` lets the dispatcher pick the right scrollable surface;
    /// callers read `ctx.framework().focused()` and pass it through.
    fn dispatcher() -> fn(Self::Actions, focused: FocusedPane<Ctx::AppPaneId>, ctx: &mut Ctx);
}

/// App-extension globals scope. One impl per app. The framework's own
/// pane-management/lifecycle globals live in `GlobalAction` and are
/// not part of this scope.
pub trait Globals<Ctx: AppContext>: 'static {
    type Actions: Action;

    const SCOPE_NAME: &'static str = "global";

    fn render_order() -> &'static [Self::Actions];
    fn defaults() -> Bindings<Self::Actions>;
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
}
```

**Tradeoff.** All four traits are `'static` (`Pane<Ctx>: 'static` is the supertrait; `Shortcuts<Ctx>` inherits it). The framework stores `fn` pointers and TypeId-keyed lookups; both require `'static`. App pane structs are owned by `App` and are themselves `'static`, so the bound costs nothing in practice.

`Pane::mode` returns a `fn(&Ctx) -> Mode<Ctx>`. The framework's structural-Esc gate, bar-region suppression, and key dispatch all run with `&Ctx` and an `AppPaneId`, never with a typed `&PaneStruct`, so it stores the returned pointer in `Framework<Ctx>::mode_queries`, populated through `Framework::register_app_pane` at `KeymapBuilder::build_into` time. Pane-internal callers write `Self::mode()(ctx)`. Default returns `Mode::Navigable`; panes whose mode varies with `Ctx` state (Finder, Output, Settings) override. The `Mode::TextInput(handler)` variant carries the handler inline — when the focused pane returns `TextInput(h)`, the framework calls `h(key, ctx)` for every keystroke; the handler is the sole authority for keys while the pane is in TextInput. To exit, the handler mutates `ctx` so `mode()` next frame returns `Navigable`/`Static`.

---

## 5. `ShortcutState` / `Visibility` / `BarSlot<A>` / `BarRegion` / `Mode<Ctx>`

```rust
/// Per-action enabled/disabled flag, returned by `Shortcuts::state`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutState { Enabled, Disabled }

/// Per-action visibility, returned by `Shortcuts::visibility`.
/// Variant names match Bevy's `bevy::render::view::Visibility` for the
/// variants we need (`Inherited` is not modeled — bar slots have no
/// visibility hierarchy to inherit from).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Visibility { Visible, Hidden }

/// One slot in the bar. `Single` shows every key bound to one action,
/// joined by `,`. `Paired` glues two actions with `/` under one label,
/// using **primary keys only** — alternative bindings for paired
/// actions never appear in paired slots. If either half of a `Paired`
/// is `Hidden` per `Shortcuts::visibility`, the whole slot hides
/// (all-or-nothing — paired slots represent one conceptual operation).
#[derive(Clone, Copy, Debug)]
pub enum BarSlot<A> {
    Single(A),
    Paired(A, A, &'static str),
}

/// Three regions of the bar, left-to-right.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarRegion {
    /// Paired-key rows: nav (↑/↓), expand (←/→), all (+/-), pane (Tab).
    Nav,
    /// Per-action rows from the focused pane.
    PaneAction,
    /// `GlobalAction` strip + `Globals::render_order()`.
    Global,
}

impl BarRegion {
    pub const ALL: &'static [BarRegion] =
        &[BarRegion::Nav, BarRegion::PaneAction, BarRegion::Global];
}

/// Pane's current typing/scrolling mode. Drives bar-region suppression,
/// structural Esc gate, and key dispatch.
///
/// The `TextInput` variant carries its key handler inline so the
/// invariant "TextInput-without-handler" is unrepresentable.
pub enum Mode<Ctx> {
    /// List/cursor pane. The app's `Navigation` scope drives it
    /// (the framework reads keys through the `Navigation` trait's
    /// accessors). `Nav` region emitted; `Global` emitted;
    /// structural Esc enabled.
    Navigable,
    /// No scrolling, no typed input (Output, KeymapPane Conflict).
    /// `Nav` suppressed; `Global` emitted.
    Static,
    /// Pane consumes typed characters (Finder, Settings Editing,
    /// KeymapPane Awaiting). `Nav` and `Global` suppressed; structural
    /// Esc suppressed. The handler is the sole authority for keys while
    /// the pane is in this mode — there is no fall-through to global
    /// dispatch. To exit, the handler mutates `ctx` so `mode()` next
    /// frame returns `Navigable`/`Static`. To honor any global key
    /// (Ctrl+Q, etc.), the handler implements it itself.
    TextInput(fn(KeyBind, &mut Ctx)),
}
```

**Tradeoff.** `Visibility` is a separate axis from `ShortcutState` (visible-but-grayed). `Hidden` removes the slot entirely; `Disabled` keeps it visible-but-inert. Together they cover all per-frame slot presentations without growing the bar-label surface — the label itself is always `Action::bar_label()` declared once in `action_enum!`.

---

## 6. `Keymap<Ctx>`

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Built-once, read-every-frame container. `AppPaneId`-keyed per
/// app-pane scope. Internal storage uses a crate-private
/// `RuntimeScope<Ctx>` trait object per registered pane; public
/// callers reach pane operations through the convenience methods on
/// `Keymap<Ctx>` and never name the trait.
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:      HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    /// Phase 10: `Option<ScopeMap<N::Actions>>` for navigation,
    /// `Option<ScopeMap<G::Actions>>` for app globals (typed
    /// singletons; downcast at the getter), and the framework's
    /// own `ScopeMap<GlobalAction>` for `Quit`/`Restart`/etc.
    config_path: Option<PathBuf>,
}

impl<Ctx: AppContext + 'static> Keymap<Ctx> {
    /// Entry point. Returns the builder in `Configuring` state — the
    /// settings phase. The first `register::<P>` call transitions the
    /// type to `KeymapBuilder<Ctx, Registering>`, after which settings
    /// methods (`config_path`, Phase 10's `load_toml` / `vim_mode` /
    /// etc.) drop off the type. Compile-time enforced ordering.
    pub fn builder() -> KeymapBuilder<Ctx, Configuring> { KeymapBuilder::new() }

    /// Path to the config file the loader will read (Phase 10) — set
    /// via `KeymapBuilder::config_path`. `None` when no path was
    /// configured.
    pub fn config_path(&self) -> Option<&Path> { self.config_path.as_deref() }

    /// Resolve `bind` to an action in the scope registered for `id`
    /// and call its dispatcher. Returns `KeyOutcome::Unhandled` if no
    /// scope is registered for `id` or no binding matches; the caller
    /// continues its dispatch chain (globals, dismiss, fallback) on
    /// `Unhandled`. Public — the framework input dispatcher calls this
    /// when it walks `FocusedPane::App(id)`.
    pub fn dispatch_app_pane(
        &self,
        id: Ctx::AppPaneId,
        bind: &KeyBind,
        ctx: &mut Ctx,
    ) -> KeyOutcome { /* … */ }

    /// Bar slots for the scope registered for `id`, fully resolved to
    /// `RenderedSlot { region, label, key, state, visibility }`.
    /// Returns an empty `Vec` if no scope is registered. Public — the
    /// bar renderer calls this when it walks `FocusedPane::App(id)`.
    pub fn render_app_pane_bar_slots(
        &self,
        id: Ctx::AppPaneId,
        ctx: &Ctx,
    ) -> Vec<RenderedSlot> { /* … */ }

    /// Reverse lookup: TOML action key string → currently bound
    /// `KeyBind` in the scope registered for `id`. Returns `None` if
    /// `id` is unregistered, the action name is not recognized, or the
    /// named action has no binding. Used by display code that needs to
    /// show the user's currently bound key for a named action.
    pub fn key_for_toml_key(
        &self,
        id: Ctx::AppPaneId,
        action: &str,
    ) -> Option<KeyBind> { /* … */ }

    // Phase 10 additions (typed singleton getters):
    //   pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Actions>
    //   pub fn globals<G: Globals<Ctx>>(&self)       -> &ScopeMap<G::Actions>
    // Both downcast at the getter from a typed `Option<Box<dyn Any>>`
    // singleton slot populated at `KeymapBuilder::register_navigation`
    // / `register_globals` time. Pane scopes stay erased
    // (heterogeneous); singletons stay typed (one impl each per
    // binary).
    //
    //   pub(crate) fn framework_globals(&self) -> &ScopeMap<GlobalAction>
    // Used only by the framework's built-in `GlobalAction` dispatcher
    // and the bar's `Global` region; the binary never names it.
    //
    //   pub fn config_path_for(name: &str) -> PathBuf
    // Returns `{dirs::config_dir()}/<name>/keymap.toml`. Free fn
    // helper for the binary to compute the default config path.
}
```

**Tradeoff.** `RuntimeScope<Ctx>` is crate-private, so the public surface is three concrete methods rather than a `scope_for_app_pane(id) -> Option<&dyn RuntimeScope<Ctx>>` getter. External callers never name the trait. Phase-10 typed singletons (`navigation` / `globals`) downcast at the getter; pane scopes stay erased because they're heterogeneous (one per pane, typed differently).

---

## 7. `KeymapBuilder<Ctx, State>`

```rust
use core::marker::PhantomData;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Settings phase: state when `KeymapBuilder` accepts settings methods
/// (`config_path`, `load_toml`, `vim_mode`, `with_settings`,
/// `on_quit`, `on_restart`, `dismiss_fallback`,
/// `with_framework_toast_settings`). The first `register::<P>` call
/// transitions to `Registering`.
pub struct Configuring;

/// Panes phase: state after the first `register::<P>` call. Settings
/// methods are no longer reachable on the type — the compiler enforces
/// "settings before panes."
pub struct Registering;

/// Builder for `Keymap<Ctx>`. Two-state typestate.
///
/// The `State` parameter defaults to `Configuring`; consumers rarely
/// name it explicitly. The first `register::<P>` call returns
/// `KeymapBuilder<Ctx, Registering>`, after which only `register` and
/// `build` (and Phase 10's `build_into`) are reachable.
pub struct KeymapBuilder<Ctx: AppContext + 'static, State = Configuring> {
    scopes:      HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    config_path: Option<PathBuf>,
    // Phase 10 fields (settings collected during Configuring,
    // consumed eagerly by each register::<P> call):
    //   toml:        Option<TomlTable>      // result of load_toml(path)
    //   vim_mode:    VimMode                // default VimMode::Disabled
    //   settings:    SettingsRegistry<Ctx>
    //   on_quit:     Option<fn(&mut Ctx)>
    //   on_restart:  Option<fn(&mut Ctx)>
    //   dismiss_fallback:    Option<fn(&mut Ctx) -> bool>
    //   toast_settings_binding:
    //                Option<ToastSettingsBinding<Ctx>>  // Phase 21
    //   nav, globals: typed singleton slots populated by
    //                 register_navigation::<N>() / register_globals::<G>()
    _state: PhantomData<State>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Configuring> {
    /// Override the config path the loader will read.
    pub fn config_path(self, path: PathBuf) -> Self;

    /// Register a `Shortcuts<Ctx>` impl. Eagerly collapses
    /// `P::defaults()` (after applying any TOML overlay and vim
    /// extras the builder has accumulated) into a
    /// `ScopeMap<P::Actions>` and stores the typed pane behind a
    /// crate-private `RuntimeScope` trait object keyed on
    /// `P::APP_PANE_ID`. Transitions to `Registering`.
    pub fn register<P: Shortcuts<Ctx>>(self, pane: P) -> KeymapBuilder<Ctx, Registering>;

    /// Finalize an empty keymap (no panes). Returns `Result` for
    /// uniformity with the `Registering` form; Phase 10 may surface
    /// `KeymapError::NavigationMissing` / `GlobalsMissing` here once
    /// those singletons become required.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError>;

    // Phase 10 additions (Configuring-state only):
    //
    //   pub fn load_toml(self, path: impl AsRef<Path>) -> Self
    //     Reads the TOML config file. Returns Self (not Result<Self>)
    //     so the chain stays fluent; errors surface from build().
    //
    //   pub fn vim_mode(self, mode: VimMode) -> Self
    //   pub fn with_settings(self, f: impl FnOnce(&mut SettingsRegistry<Ctx>)) -> Self
    //   pub fn register_navigation<N: Navigation<Ctx>>(self, nav: N) -> Self
    //   pub fn register_globals<G: Globals<Ctx>>(self, globals: G) -> Self
    //   pub fn on_quit(self, f: fn(&mut Ctx)) -> Self
    //   pub fn on_restart(self, f: fn(&mut Ctx)) -> Self
    //   pub fn dismiss_fallback(self, f: fn(&mut Ctx) -> bool) -> Self
    //
    // Phase 21 addition (Configuring-state only — framework-owned
    // toast settings):
    //
    //   pub fn with_framework_toast_settings(
    //       self,
    //       binding: ToastSettingsBinding<Ctx>,
    //   ) -> Self
    //
    // build_into() calls (binding.load)(ctx) and pre-populates the
    // settings registry with one entry per ToastSettings field under
    // SettingsSection::Framework("toasts"). The settings-pane editor
    // mutates framework.toast_settings_mut() directly; on commit
    // dispatch calls (binding.save)(ctx, framework.toast_settings())
    // after the framework borrow is dropped. There is no app-side
    // mutable copy — Framework<Ctx> is the sole mutable owner.
    // build() (no Framework arg) silently drops the binding.
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Registering> {
    /// Register an additional `Shortcuts<Ctx>` impl. Same body as the
    /// `Configuring`-state form, but stays in `Registering`.
    pub fn register<P: Shortcuts<Ctx>>(self, pane: P) -> Self;

    /// Finalize the builder. Returns the built `Keymap<Ctx>` or the
    /// first loader / validation error encountered.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError>;

    // Phase 10 addition (Registering-state only):
    //
    //   pub fn build_into(
    //       self,
    //       framework: &mut Framework<Ctx>,
    //   ) -> Result<Keymap<Ctx>, KeymapError>
    //
    // Same as build(), but also populates the framework's per-AppPaneId
    // mode_queries registry by calling
    // `framework.register_app_pane(P::APP_PANE_ID, P::mode())` for each
    // `P` registered on the builder. This keeps `Framework<Ctx>` and
    // `Keymap<Ctx>` independently constructible.
}
```

**Tradeoffs.**

- **Typestate over flat builder + runtime check.** The earlier draft used a flat builder that returned `Err(BuilderError::NavigationMissing)` from `build()` when the binary forgot a required step. The typestate gates the ordering rule (`load_toml` / `vim_mode` must come before `register::<P>`) at compile time, so the compiler catches the misuse with a "method not found" error. Phase 10 will still need *runtime* checks for `NavigationMissing` / `GlobalsMissing` because those singletons can be omitted entirely; those land as `KeymapError` variants surfaced from `build` / `build_into`.
- **Eager collapse over deferred `PendingScope`.** `register::<P>` does the typed work inline (TOML lookup via `P::Actions::from_toml_key`, vim extras via `P::vim_extras()`, then `Bindings::into_scope_map`). No deferred storage, no module-private `PendingEntry` trait, no `Box<dyn Any>`. The cost: ordering matters (settings before panes); the typestate already enforces it.
- **Single error type (`KeymapError`).** Earlier drafts split into `KeymapError` (loader) and `BuilderError` (builder validation). The shipped form keeps one type — the binary's startup path renders both the same way. Phase 10 adds `NavigationMissing`, `GlobalsMissing`, `DuplicateScope { type_name: &'static str }` as new `KeymapError` variants.

`load_toml` defers errors to `build()` so the chain reads top-to-bottom without a `?` mid-chain. Framework panes (Toasts, SettingsPane, KeymapPane) reach `&mut Framework<Ctx>` from their free-fn dispatchers via `ctx.framework_mut()` on the `AppContext` trait — no separate accessor hook is needed.

---

## 8. `Framework<Ctx>`

```rust
/// Aggregator owned by the binary's `App`. Holds the three framework
/// panes, the per-AppPaneId input-mode query registry, the registered
/// pane order, lifecycle flags (quit/restart), and the currently
/// focused pane. Focus is owned here (not on the `AppContext` trait)
/// so framework code reads `self.focused` instead of calling back
/// through `Ctx`.
///
/// The three pane fields are `pub` so the binary can pass references
/// directly to its bar-render dispatch (`bar::render(&app.framework.toasts, …)`)
/// without going through accessor methods. Mutation goes through each
/// pane's typed methods, not field assignment.
pub struct Framework<Ctx: AppContext> {
    /// Currently focused pane. The binary's `Focus` subsystem is a
    /// layer above this field — it writes here as part of every
    /// transition. Private; read via `focused()`, written via
    /// `set_focused`.
    focused:           FocusedPane<Ctx::AppPaneId>,
    /// `GlobalAction::Quit` lifecycle flag. Polled by the binary's
    /// main loop. Written by `request_quit()` (`pub(super)`).
    quit_requested:    bool,
    /// `GlobalAction::Restart` lifecycle flag. Polled by the binary's
    /// main loop. Written by `request_restart()` (`pub(super)`).
    restart_requested: bool,
    /// Per-AppPaneId input-mode queries, populated by
    /// `KeymapBuilder::build_into(&mut framework)` through
    /// `register_app_pane`. Framework panes (Keymap, Settings, Toasts)
    /// are special-cased in `focused_pane_mode` and do not appear
    /// here. Private; written only through `pub(super) fn register_app_pane`.
    mode_queries:      HashMap<Ctx::AppPaneId, fn(&Ctx) -> Mode<Ctx>>,
    /// Registered AppPaneIds in registration order. Drives
    /// `NextPane`/`PrevPane` walks and the bar renderer's pane-cycle
    /// row (Phase 12). Private; written by `register_app_pane`, read
    /// publicly via `pane_order()`.
    pane_order:        Vec<Ctx::AppPaneId>,
    /// The framework overlay currently open over the focused pane,
    /// if any. Orthogonal to `focused`: the underlying focus stays put
    /// while the overlay is `Some(_)`. Written only by
    /// `open_overlay`/`close_overlay` (`pub(super)`); read publicly via
    /// `overlay()`. Phase 11 ships this as `Option<FrameworkPaneId>`;
    /// Phase 12 narrows the type to `Option<FrameworkOverlayId>` so
    /// `Some(Toasts)` becomes unrepresentable.
    overlay:           Option<FrameworkOverlayId>,
    pub keymap_pane:   KeymapPane<Ctx>,
    pub settings_pane: SettingsPane<Ctx>,
    pub toasts:        Toasts<Ctx>,
    /// Live framework toast settings. Sole mutable owner —
    /// there is no app-side mutable copy.
    ///
    /// The field is introduced on `Framework<Ctx>` in **Phase 11**,
    /// defaulted via `ToastSettings::default()` at `Framework::new`
    /// (so builds that never register a binding still work).
    /// **Phase 21** adds the `ToastSettingsBinding` wiring:
    /// `KeymapBuilder::build_into` calls `ToastSettingsBinding::load`
    /// when the binary registers a binding through
    /// `with_framework_toast_settings`, replacing the default if the
    /// load hook returns `Some(settings)`. The `SettingsPane` editor
    /// mutates this field directly through `toast_settings_mut()`;
    /// dispatch calls `ToastSettingsBinding::save(ctx, &settings)`
    /// after the framework borrow ends so the binary can persist. The
    /// renderer and the migrated manager (Phase 22) read this directly
    /// — never the binding.
    pub(super) toast_settings: ToastSettings,
}

impl<Ctx: AppContext> Framework<Ctx> {
    /// Construct a fresh framework with an explicit initial focus and
    /// both lifecycle flags cleared. `KeymapBuilder::build_into(&mut
    /// framework)` later populates `mode_queries` and `pane_order`.
    pub fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self {
        Self {
            focused:           initial_focus,
            quit_requested:    false,
            restart_requested: false,
            mode_queries:      HashMap::new(),
            pane_order:        Vec::new(),
            overlay:           None,
            keymap_pane:       KeymapPane::new(),
            settings_pane:     SettingsPane::new(),
            toasts:            Toasts::new(),
            toast_settings:    ToastSettings::default(),
        }
    }

    /// Live framework toast settings. Read by the bar renderer and
    /// (Phase 22+) the migrated toast manager.
    pub fn toast_settings(&self) -> &ToastSettings { &self.toast_settings }

    /// Mutate the live framework toast settings. The Phase 21
    /// `SettingsPane` editor mutates this directly; on commit,
    /// dispatch calls `ToastSettingsBinding::save(ctx, &settings)`
    /// after the framework borrow ends so the binary can persist.
    /// `Framework<Ctx>` is the sole mutable owner — there is no
    /// app-side mutable copy.
    pub(super) fn toast_settings_mut(&mut self) -> &mut ToastSettings {
        &mut self.toast_settings
    }

    /// Currently focused pane.
    pub const fn focused(&self) -> &FocusedPane<Ctx::AppPaneId> { &self.focused }

    /// Update the focused pane. The binary's `Focus` subsystem calls
    /// this from its existing transitions.
    pub const fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) {
        self.focused = focus;
    }

    /// Polled by the binary's main loop after `GlobalAction::Quit`.
    pub const fn quit_requested(&self) -> bool { self.quit_requested }

    /// Polled by the binary's main loop after `GlobalAction::Restart`.
    pub const fn restart_requested(&self) -> bool { self.restart_requested }

    /// Registered app-pane ids in registration order. Read by the
    /// `NextPane`/`PrevPane` dispatchers and the bar renderer's
    /// pane-cycle row.
    pub fn pane_order(&self) -> &[Ctx::AppPaneId] { &self.pane_order }

    /// The currently open framework overlay, if any. Orthogonal to
    /// `focused` — the underlying app pane stays focused while an
    /// overlay is open.
    pub const fn overlay(&self) -> Option<FrameworkOverlayId> { self.overlay }

    /// File path of the editor currently active on a framework
    /// overlay. Returns the keymap or settings pane's
    /// `editor_target` when the matching overlay is open, `None`
    /// otherwise.
    pub fn editor_target_path(&self) -> Option<&Path> { /* … */ }

    /// Resolve the focused pane's `Mode<Ctx>`. **Overlay layer wins**
    /// when `overlay` is `Some`; otherwise dispatches by `focused`:
    ///
    /// - `FocusedPane::App(id)` → registered mode query.
    /// - `FocusedPane::Framework(Toasts)` → `self.toasts.mode(ctx)`
    ///   (`Mode::Static` in Phase 11; `Mode::Navigable` in Phase 12+).
    ///
    /// Returns `Option<Mode<Ctx>>` (Phase 10 contract).
    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Option<Mode<Ctx>> { /* … */ }

    /// Run the framework dismiss chain (Phase 11). Phase 12 renames
    /// this to `dismiss_framework` and adds a free `dismiss_chain`
    /// dispatcher; see "Dismiss chain" note above.
    pub fn dismiss(&mut self) -> bool { /* … */ }
}
```

**Tradeoff.** Public fields for the three framework panes — the binary uses them directly in `bar::render` dispatch and input routing. `focused`, `quit_requested`, `restart_requested`, `mode_queries`, `pane_order`, and `overlay` are private; their writers are all `pub(super)` (only `framework/dispatch.rs` and `KeymapBuilder` write into them) and their readers are the public methods above. `Framework::new` takes an explicit `initial_focus` rather than implementing `Default`; the binary picks the starting pane.

**Frozen-signature surface.** The Phase 6 fields (`focused`, `quit_requested`, `restart_requested`) and the four `const fn` getters/setters (`focused`, `set_focused`, `quit_requested`, `restart_requested`) keep their signatures verbatim across phases. `Framework::new`'s signature is also frozen, but its body grows new field initializers (Phase 10 added `mode_queries` / `pane_order` / `overlay`; Phase 11 added the three pane defaults). The `const fn` qualifier on `new` was lost at Phase 10 once `HashMap::new()` entered the body.

**Dismiss chain — Phase 11 method, Phase 12+ free fn.** Phase 11 ships `Framework::dismiss(&mut self) -> bool` as a method that pops the focused toast (placeholder `Vec<String>` stack), then closes any open overlay. Phase 12 replaces that with `Framework::dismiss_framework(&mut self) -> bool` plus a free `framework::dispatch::dismiss_chain<Ctx>(ctx: &mut Ctx, fallback: Option<fn(&mut Ctx) -> bool>) -> bool` so the binary's optional `dismiss_fallback` hook can take `&mut Ctx` after the framework borrow drops (no aliasing between `&mut Framework` and `&mut Ctx`). `Framework::close_overlay()` (`pub(super)`) is the overlay-clear primitive both the method and the free fn reuse.

### `Toasts<Ctx>` — framework-owned typed pane

Phase 11 ships `Toasts<Ctx>` as a placeholder pane carrying a `Vec<String>` message stack with `push` / `try_pop_top` / `has_active` and a single `ToastsAction::Dismiss` action. Phase 12 replaces the placeholder with a typed `Toast<Ctx>` manager that owns toast data, the focus viewport, and dismiss/navigation semantics. Phase 20 attaches an `Option<Ctx::ToastAction>` payload to each toast for activation. Phase 21 adds `ToastSettings` registered through the same settings overlay as app settings, with `Framework<Ctx>` owning a `toast_settings: ToastSettings` field. Phase 22 ports cargo-port's lifecycle (`ToastLifetime`, `ToastPhase`, `TaskStatus`, tracked items, hitboxes, render module) into the framework, consuming `framework.toast_settings()` for width/timing/placement.

Phase 12+ surface (subset shown — full surface lands incrementally across Phases 12, 20, 22):

```rust
pub struct ToastId(u64);

pub enum ToastStyle { Normal, Warning, Error }

pub struct Toast<Ctx: AppContext> {
    id:     ToastId,
    title:  String,
    body:   String,
    style:  ToastStyle,
    // Phase 12 ships with `_ctx: PhantomData<fn(&Ctx)>`.
    // Phase 20 replaces the PhantomData with a real Ctx-tied field:
    action: Option<Ctx::ToastAction>,
    // Phase 22 adds: lifetime: ToastLifetime, phase: ToastPhase,
    //                tracked_items: Vec<TrackedItem>
}

pub struct Toasts<Ctx: AppContext> {
    toasts:   Vec<Toast<Ctx>>,
    viewport: Viewport,
    next_id:  u64,
    _ctx:     PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> Toasts<Ctx> {
    pub const fn new() -> Self;
    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId;
    pub fn push_with_action(&mut self, title: …, body: …, action: Ctx::ToastAction) -> ToastId; // Phase 20
    pub fn dismiss(&mut self, id: ToastId) -> bool;
    pub fn dismiss_focused(&mut self) -> bool;
    pub fn focused_id(&self) -> Option<ToastId>;
    pub fn has_active(&self) -> bool;
    pub fn active(&self) -> &[Toast<Ctx>];
    pub fn reset_to_first(&mut self);
    pub fn reset_to_last(&mut self);
    /// Resolved-nav entry point. Dispatch translates the app's
    /// resolved navigation action into framework-owned `ListNavigation`
    /// via `Navigation::list_navigation` (default impl matches the
    /// resolved action against the trait's `UP`/`DOWN`/`HOME`/`END`
    /// constants) before calling this method when focus is
    /// `FocusedPane::Framework(FrameworkFocusId::Toasts)`. `Toasts<Ctx>`
    /// never references the binary's `NavigationAction` directly.
    pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome;
    /// Pre-globals hook. Dispatch calls this when the inbound key maps
    /// to `GlobalAction::NextPane`/`PrevPane` and Toasts is focused.
    /// Returns `true` when there is internal scroll room — consumes
    /// the key, blocks the cycle advance.
    pub fn try_consume_cycle_step(&mut self, direction: Direction) -> bool;
    /// No-op wrapper retained for tests that drive raw key dispatch.
    /// Production path uses `on_navigation` + `try_consume_cycle_step`.
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome;
    pub fn handle_key_command(&mut self, bind: &KeyBind)
        -> (KeyOutcome, ToastCommand<Ctx::ToastAction>); // Phase 20

    // Phase 22 additions:
    pub fn push_timed     (&mut self, title: …, body: …, timeout: Duration) -> ToastId;
    pub fn push_task      (&mut self, title: …, body: …, linger: Duration) -> (ToastId, ToastTaskId);
    pub fn push_persistent(&mut self, title: …, body: …) -> ToastId;
    pub fn finish_task    (&mut self, task_id: ToastTaskId);
    pub fn set_tracked_items(&mut self, id: ToastId, items: Vec<TrackedItem>);
    pub fn prune          (&mut self, now: Instant);
    pub fn render         (&self, area: Rect, buf: &mut Buffer, settings: &ToastSettings) -> Vec<ToastHitbox>;
}
```

`ToastsAction` from Phase 11 carries `Dismiss`; Phase 12 removes `Dismiss` (dismiss flows through `GlobalAction::Dismiss`) and the enum is empty in Phase 12 (Phase 20 adds `Activate`). Toast viewport movement is keymap-driven, not literal-key-driven: dispatch resolves the inbound key against the app's `Navigation` scope, translates the resolved action into framework-owned `ListNavigation` via `Navigation::list_navigation` (default impl matches against the trait's `UP`/`DOWN`/`HOME`/`END` constants), and calls `framework.toasts.on_navigation(list_nav)`. `try_consume_cycle_step` is the pre-globals hook that consumes `NextPane`/`PrevPane` keystrokes when there is internal scroll room. `Toasts::mode(&Ctx) -> Mode<Ctx>` is `Mode::Static` in Phase 11 (placeholder), `Mode::Navigable` in Phase 12+ (focused toasts behave like a list). All Phase 12 entry points take `&mut self` only — pane-local viewport mutation never aliases the framework borrow.

```rust
/// Framework-owned navigation vocabulary for list-style framework
/// panes (Toasts today; reusable by future framework list panes).
/// Decouples Phase 12 framework code from the binary's
/// `NavigationAction` enum (defined in Phase 14).
pub enum ListNavigation { Up, Down, Home, End }
```

```rust
pub enum ToastCommand<A> {
    None,
    Activate(A),
}

pub enum NoToastAction {} // uninhabited filler for apps without toast activation
```

---

## 9. `SettingsRegistry<Ctx>`

```rust
/// Builder filled inside `KeymapBuilder::with_settings(|registry| …)`.
/// Each setting carries the TOML key, the display label, a getter
/// closure, and a setter closure.
///
/// **Closures, not free fns.** The framework calls these closures while
/// already inside the dispatch path — the closure *is* the borrow holder
/// that touches `Ctx`. There's no re-entrancy hazard, and closures are
/// the more ergonomic choice (they capture `&App` field paths cleanly).
pub struct SettingsRegistry<Ctx> {
    entries: Vec<SettingEntry<Ctx>>,
}

impl<Ctx> SettingsRegistry<Ctx> {
    pub(crate) fn new() -> Self { Self { entries: Vec::new() } }

    /// Bool setting. The setter receives the new value directly.
    pub fn add_bool(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        get:      impl Fn(&Ctx) -> bool + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, bool) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Closed-set enum setting. `variants` is the full list of TOML
    /// keys; the editor-pane cycles through them. Getter returns the
    /// current variant's TOML key; setter receives one of `variants`.
    pub fn add_enum(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        variants: &'static [&'static str],
        get:      impl Fn(&Ctx) -> &'static str + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, &str) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Integer setting with optional min/max. Values outside the range
    /// are rejected by the editor pane before `set` runs.
    pub fn add_int(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        get:      impl Fn(&Ctx) -> i64 + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, i64) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Set min/max bounds for the most recently added int setting.
    /// Calling on a non-int registers a runtime warning at `build()`.
    pub fn with_bounds(&mut self, min: i64, max: i64) -> &mut Self { /* … */ self }
}

/// Internal record kept per setting.
pub(crate) struct SettingEntry<Ctx> {
    pub toml_key: &'static str,
    pub label:    &'static str,
    pub kind:     SettingKind<Ctx>,
}

pub(crate) enum SettingKind<Ctx> {
    Bool {
        get: Box<dyn Fn(&Ctx) -> bool + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, bool) + Send + Sync>,
    },
    Enum {
        variants: &'static [&'static str],
        get: Box<dyn Fn(&Ctx) -> &'static str + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, &str) + Send + Sync>,
    },
    Int {
        bounds: Option<(i64, i64)>,
        get: Box<dyn Fn(&Ctx) -> i64 + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, i64) + Send + Sync>,
    },
}
```

**Persistence.** The binary owns persistence. The registry's setter closure is the binary's hook — when the binary writes to its config struct it can also write to disk inside the same closure (or trigger a debounced write later). The framework does **not** open or write any config file on its own; conflating "framework UI" with "binary's config layout" would force every consumer to use one TOML schema. The registry's only persistence-adjacent feature is the `toml_key` field, used by the keymap-overlay help screen and by any export-defaults tooling the binary chooses to add.

---

## 10. `GlobalAction`, `VimMode`, `KeymapError`

```rust
/// Framework-owned global actions. Defaults: q/R/Tab/Shift+Tab/Ctrl+K/s/x.
/// The dispatch for `Quit`, `Restart`, `Dismiss` is supplied by the
/// binary as the three positional `Keymap::builder(quit, restart, dismiss)`
/// arguments. `NextPane`/`PrevPane`/`OpenKeymap`/`OpenSettings` dispatch
/// is owned entirely by the framework (it knows the pane registry).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlobalAction {
    Quit,
    Restart,
    NextPane,
    PrevPane,
    OpenKeymap,
    OpenSettings,
    Dismiss,
}

impl Action for GlobalAction {
    const ALL: &'static [Self] = &[
        Self::Quit, Self::Restart, Self::NextPane, Self::PrevPane,
        Self::OpenKeymap, Self::OpenSettings, Self::Dismiss,
    ];

    fn toml_key(self) -> &'static str {
        match self {
            Self::Quit         => "quit",
            Self::Restart      => "restart",
            Self::NextPane     => "next_pane",
            Self::PrevPane     => "prev_pane",
            Self::OpenKeymap   => "open_keymap",
            Self::OpenSettings => "open_settings",
            Self::Dismiss      => "dismiss",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Quit         => "Quit",
            Self::Restart      => "Restart",
            Self::NextPane     => "Next pane",
            Self::PrevPane     => "Previous pane",
            Self::OpenKeymap   => "Open keymap viewer",
            Self::OpenSettings => "Open settings",
            Self::Dismiss      => "Dismiss overlay / output",
        }
    }
}

// `Keymap::<Ctx>::builder()` takes no positional dispatch callbacks.
// Under the post-Phase-3 design the framework owns dispatch for every
// `GlobalAction` variant:
//
//   - Quit:        framework sets `Framework<Ctx>::quit_requested = true`.
//                  Binary's main loop polls `framework.quit_requested()`
//                  and exits cleanly.
//   - Restart:     framework sets `Framework<Ctx>::restart_requested = true`.
//                  Binary's main loop polls and re-launches.
//   - Dismiss:     framework runs its own dismiss chain (toasts → focused
//                  framework overlay), then bubbles to the binary's
//                  registered `dismiss_fallback` (returns `bool`: did I
//                  handle it?) for app-level dismissables (finder,
//                  output, deleted ProjectList row, etc.).
//   - NextPane / PrevPane / OpenKeymap / OpenSettings:
//                  framework dispatches via its own pane registry. Binary
//                  never sees these.
//
// Optional cleanup hooks the binary may register on the builder:
//
//     Keymap::<App>::builder()
//         .on_quit(|app| { /* save state */ })          // optional
//         .on_restart(|app| { /* save state */ })       // optional
//         .dismiss_fallback(|app| -> bool {              // optional
//             app.try_dismiss_focused_app_thing()
//         })

/// Vim-mode flag passed to `KeymapBuilder::vim_mode`. When `Enabled`,
/// the framework appends `'k'/'j'/'h'/'l'` to `Navigation::UP/DOWN/LEFT/RIGHT`
/// and walks every registered `Shortcuts::vim_extras()` after TOML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimMode {
    Disabled,
    Enabled,
}

impl Default for VimMode {
    fn default() -> Self { Self::Disabled }
}

/// Errors surfaced by the TOML loader. Wrapped by
/// `BuilderError::Toml` when surfaced from `build()`.
///
/// Per Phase 2 retrospective, `KeymapError` is `#[derive(thiserror::Error)]`
/// with `#[error("…")]` per variant. The `BadKey` variant wraps
/// `KeyParseError` via `#[from]` for `?` propagation from `KeyBind::parse`
/// and `Display` source-chaining.
#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    /// `std::io::Error` opening the file. Missing file is **not** an
    /// error — the loader treats it as "use defaults" and returns `Ok`.
    #[error("I/O error reading keymap config")]
    Io(#[from] std::io::Error),
    /// Top-level TOML parse failure.
    #[error("TOML parse error in keymap config")]
    Parse(#[from] toml::de::Error),
    /// Two TOML keys in the same array refer to the same physical key.
    #[error("duplicate key '{key}' in {scope}.{action}")]
    InArrayDuplicate { scope: String, action: String, key: String },
    /// Two actions in the same scope bind to the same key.
    #[error("key '{key}' bound to both {} and {} in [{scope}]", actions.0, actions.1)]
    CrossActionCollision {
        scope: String,
        key: String,
        actions: (String, String),
    },
    /// A TOML key string failed `KeyBind::parse`.
    #[error("invalid binding for {scope}.{action}")]
    InvalidBinding { scope: String, action: String, #[source] source: KeyParseError },
    /// TOML referenced an unknown action in a known scope. Constructed
    /// at the loader: `A::from_toml_key(key)` returned `None` and the
    /// loader attaches the scope name.
    #[error("unknown action '{action}' in [{scope}]")]
    UnknownAction { scope: String, action: String },
    /// TOML referenced an unknown scope name.
    #[error("unknown scope [{scope}]")]
    UnknownScope { scope: String },
}
```

**Tradeoff.** `GlobalAction` impls `Action` so it can flow through the same `ScopeMap` / TOML / display machinery as every other scope. The binary never names it; the impl exists purely so framework-internal code reuses one path.

---

## 11. Module hierarchy and `lib.rs` re-exports

**Canonical source.** This section is the single authoritative definition of the `tui_pane` foundations module hierarchy (split across Phases 2–10 of `tui-pane-lib.md`). Each sub-phase references this section for its file list; do not duplicate the file list elsewhere. The directories declared here (`keymap/`, `bar/`, `panes/`, plus `settings.rs`) are the same skeletons that Phases 10 and 11 fill in.

```rust
//! `tui_pane` — keymap-driven pane framework on top of crossterm + ratatui.
//!
//! The binary supplies an app context type `Ctx` (typically `App`) and
//! every public type / trait below is generic over it.

#![deny(missing_docs)]

mod bar;
mod framework;
mod keymap;
mod panes;
mod settings;

// All submodules are private (`mod`, never `pub mod` — `cargo mend` denies
// `pub mod` workspace-wide). Public types re-export from the crate root,
// so callers write `tui_pane::Foo` flat — never `tui_pane::keymap::Foo`.
// The sub-path forms below (`crate::keymap::Foo`) are the *facade* paths
// that `keymap/mod.rs` exposes via its own `pub use` chain; they resolve
// from inside `lib.rs` because `keymap` is a child of the crate root.

// --- Keymap core -----------------------------------------------------
pub use crate::keymap::{
    Action, Bindings, BuilderError, GlobalAction, Globals, KeyBind,
    KeyInput, KeyParseError, Keymap, KeymapBuilder, KeymapError, Navigation,
    ScopeMap, Shortcuts, VimMode,
};

// --- Bar -------------------------------------------------------------
pub use crate::bar::{BarRegion, BarSlot, Mode, ShortcutState, Visibility};
pub use crate::pane::Pane;

// --- Framework panes + aggregator -----------------------------------
pub use crate::framework::Framework;
pub use crate::panes::{KeymapPane, SettingsPane, Toasts, ToastsAction};
pub use crate::toasts::{NoToastAction, Toast, ToastCommand, ToastId, ToastStyle};
pub use crate::settings::SettingsRegistry;

// --- Pane identity ---------------------------------------------------
//
// The framework defines its own three-variant pane id and a wrapper over
// the binary's pane id. `AppContext` is the trait every `Ctx` must impl.
// See `paneid-ctx.md` for the full split rationale (option (d)).
pub use crate::app_context::AppContext;
pub use crate::pane_id::{FrameworkOverlayId, FrameworkFocusId, FocusedPane};

// --- Macros -----------------------------------------------------------
//
// `#[macro_export]` already places `bindings!` and `action_enum!` at the
// crate root — no `pub use` needed. The Phase 4 retrospective confirmed
// the speculative `pub use crate::keymap::bindings::bindings;` line was
// unnecessary; do not add it.
```

**Final list of exported names** (alphabetical, grouped):

- **Action machinery:** `Action`, `GlobalAction`, `action_enum!`
- **Keys:** `KeyBind`, `KeyParseError`
- **Bindings & maps:** `Bindings`, `ScopeMap`, `bindings!`
- **Keymap:** `Keymap`, `KeymapBuilder`, `BuilderError`, `KeymapError`, `VimMode`
- **Traits:** `Shortcuts`, `Navigation`, `Globals`
- **Bar:** `BarRegion`, `BarSlot`, `ShortcutState`, `Visibility`, `Mode<Ctx>`
- **Framework panes & aggregator:** `Framework`, `KeymapPane`, `SettingsPane`, `Toasts`, `ToastsAction`, `Toast`, `ToastId`, `ToastStyle`, `ToastCommand`, `NoToastAction`, `SettingsRegistry`
- **Pane identity & context:** `AppContext`, `FrameworkOverlayId`, `FrameworkFocusId`, `FocusedPane`

**Tradeoff.** Every `pub use` is from one of five top-level modules (`bar`, `framework`, `keymap`, `panes`, `settings`). The crate root is the only public path; internal module paths are not part of the API. This keeps the binary's `use tui_pane::{KeyBind, Keymap, Shortcuts, …};` flat and stable across internal reorganisations.
