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
    /// case-insensitive lookup is a keymap-layer concern (Phase 8 loader),
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

**Tradeoff.** `shift` / `ctrl` take `impl Into<Self>` rather than `impl Into<KeyCode>` because crossterm's `KeyCode` does not implement `From<char>` (and orphan rules block adding it). Routing through `Into<KeyBind>` reuses the two `From` impls and makes the constructors composable. `KeyParseError` derives `thiserror::Error` so downstream wrappers (e.g. `KeymapError`) get automatic `#[source]` chaining via `#[from]`. The `InvalidChar` variant from earlier drafts was dropped — no parser path emits it.

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
    /// Empty map. `pub(crate)` — only `Bindings::into_scope_map` and the
    /// TOML loader construct one; consumers always receive a built map
    /// from the `Keymap`.
    pub(crate) fn new() -> Self {
        Self { by_key: HashMap::new(), by_action: HashMap::new() }
    }

    /// Insert one (key, action) pair.
    ///
    /// `pub(crate)` — same reason as `new`.
    ///
    /// `debug_assert!`s that `key` is either unbound or already bound
    /// to the same `action`. Cross-action collisions inside one scope
    /// are bugs in `defaults()`; the TOML loader catches the same
    /// condition for user input and returns `Err` instead of panicking.
    pub(crate) fn insert(&mut self, key: KeyBind, action: A) {
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

**Tradeoff.** `new` and `insert` are `pub(crate)` — the framework owns construction. App code only ever receives a built `&ScopeMap<A>` via `Keymap::scope_for`, so a public mutable surface would be a hazard with no caller. `action_for` / `key_for` / `display_key_for` / `display_keys_for` are all `pub` because the bar code (in `tui_pane`) and the input handlers (in the binary) both call them.

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
pub trait ActionEnum:
    Copy + Eq + std::hash::Hash + std::fmt::Debug + std::fmt::Display + 'static
{
    /// Every variant in declaration order.
    const ALL: &'static [Self];

    /// TOML key for this variant (e.g. `"activate"`).
    fn toml_key(self) -> &'static str;

    /// Default short label rendered in the bar (e.g. `"activate"`,
    /// `"clean"`). The pane's `Shortcuts::label` returns this by
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
pub trait Shortcuts<Ctx: AppContext>: 'static {
    /// The pane's action enum.
    type Action: ActionEnum;

    /// TOML table name. Survives type renames; one-line cost. Required.
    const SCOPE_NAME: &'static str;

    /// Stable per-pane identity used by the framework's per-pane
    /// query registry (e.g. `input_mode_queries`). The trait covers
    /// app panes only — framework panes (Keymap, Settings, Toasts) are
    /// special-cased — so the variant is always an `AppPaneId`.
    const APP_PANE_ID: Ctx::AppPaneId;

    /// Default keybindings. No framework default — every pane declares
    /// its own keys.
    fn defaults() -> Bindings<Self::Action>;

    /// Per-action bar label. `None` hides the slot. Default returns
    /// `Some(action.bar_label())` (the static label declared in
    /// `action_enum!`). Override only when the label depends on pane
    /// state (e.g. `PackageAction::Activate` reads `"open"` on
    /// `CratesIo` fields, `"activate"` elsewhere).
    fn label(&self, action: Self::Action, _ctx: &Ctx) -> Option<&'static str> {
        Some(action.bar_label())
    }

    /// Per-action enabled / disabled status. Default `Enabled`.
    /// Override when the action is visible but inert (e.g.
    /// `PackageAction::Clean` grayed out when no target dir exists).
    fn state(&self, _action: Self::Action, _ctx: &Ctx) -> ShortcutState {
        ShortcutState::Enabled
    }

    /// Bar slot layout. Default: one `(PaneAction, Single(action))` per
    /// `Action::ALL` in declaration order. Override to introduce
    /// `Paired` slots, route into `BarRegion::Nav`, or omit
    /// data-dependent slots.
    fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Action>)> {
        Self::Action::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Pane's current input mode (Navigable / Static / TextInput).
    /// Drives bar-region suppression and the structural Esc gate.
    fn input_mode(&self, _ctx: &Ctx) -> InputMode { InputMode::Navigable }

    /// Free-fn variant of `input_mode`, registered with the
    /// `Framework<Ctx>::input_mode_queries` map at `register::<P>()`
    /// time. Default forwards to a dummy that returns `Navigable`;
    /// panes whose mode varies with `Ctx` state override.
    fn input_mode_query() -> fn(&Ctx) -> InputMode {
        |_ctx| InputMode::Navigable
    }

    /// Optional vim-extras: pane actions that gain a vim binding when
    /// `VimMode::Enabled`. Default empty. Used by
    /// `ProjectListAction::ExpandRow` (`'l'`) and `CollapseRow` (`'h'`).
    fn vim_extras() -> &'static [(Self::Action, KeyBind)] { &[] }

    /// Free-function dispatcher. Framework calls
    /// `Self::dispatcher()(action, ctx)` while holding `&mut Ctx`.
    /// Implementations navigate from the `Ctx` root rather than
    /// holding a `&mut self` borrow (split-borrow rule).
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}

/// Navigation scope. One impl per app (the binary defines a single
/// `AppNavigation` zero-sized type and impls this trait for it).
///
/// `'static` is implied by the `ActionEnum` bound on `Action`.
pub trait Navigation<Ctx: AppContext>: 'static {
    type Action: ActionEnum;

    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Action;
    const DOWN:  Self::Action;
    const LEFT:  Self::Action;
    const RIGHT: Self::Action;

    fn defaults() -> Bindings<Self::Action>;

    /// Free fn the framework calls when any navigation action fires.
    /// `focused` lets the dispatcher pick the right scrollable surface;
    /// callers read `ctx.framework().focused()` and pass it through.
    fn dispatcher() -> fn(Self::Action, focused: FocusedPane<Ctx::AppPaneId>, ctx: &mut Ctx);
}

/// App-extension globals scope. One impl per app. The framework's own
/// pane-management/lifecycle globals live in `GlobalAction` and are
/// not part of this scope.
pub trait Globals<Ctx: AppContext>: 'static {
    type Action: ActionEnum;

    const SCOPE_NAME: &'static str = "global";

    fn render_order() -> &'static [Self::Action];
    fn defaults() -> Bindings<Self::Action>;
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}
```

**Tradeoff.** All three traits are `'static`. The framework stores `fn` pointers and TypeId-keyed lookups; both require `'static`. App pane structs are owned by `App` and are themselves `'static`, so the bound costs nothing in practice.

`Shortcuts::input_mode` exists in two forms — an instance method (`&self`) callers can use directly when they hold the pane, and a free-fn `input_mode_query()` registered with the framework. The free-fn form is what `Framework<Ctx>::input_mode_queries` stores, because the framework only knows the `PaneId` and `&Ctx` at query time. The default `input_mode_query` returns `Navigable`; panes whose mode varies (Finder, Output, Settings) override it explicitly.

---

## 5. `ShortcutState` / `BarSlot<A>` / `BarRegion` / `InputMode`

```rust
/// Per-action enabled/disabled flag, returned by `Shortcuts::state`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutState { Enabled, Disabled }

/// One slot in the bar. `Single` shows every key bound to one action,
/// joined by `,`. `Paired` glues two actions with `/` under one label,
/// using **primary keys only** — alternative bindings for paired
/// actions never appear in paired slots.
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

/// Pane's current typing/scrolling mode. Drives bar-region suppression
/// and the structural Esc pre-handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    /// List/cursor pane. `NavigationAction` drives it; `Nav` region
    /// emitted; `Global` emitted; structural Esc enabled.
    Navigable,
    /// No scrolling, no typed input (Output, Toasts, KeymapPane
    /// Conflict). `Nav` suppressed; `Global` emitted.
    Static,
    /// Pane consumes typed characters (Finder, Settings Editing,
    /// KeymapPane Awaiting). `Nav` and `Global` suppressed; structural
    /// Esc suppressed.
    TextInput,
}
```

**Tradeoff.** `Shortcuts::label` returns `Option<&'static str>` because every label in cargo-port today is a literal; supporting owned strings would cost an allocation per bar render with no caller asking for it. If a future pane needs a runtime-computed label, lift the return type to `Option<Cow<'static, str>>` then.

---

## 6. `Keymap<Ctx>`

```rust
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;

/// Built-once, read-every-frame container. TypeId-keyed per scope.
///
/// `PhantomData<fn(&mut Ctx)>` (rather than `PhantomData<Ctx>`) keeps
/// `Keymap<Ctx>` invariant in `Ctx` and *not* auto-`Send`/`Sync` from
/// `Ctx` alone — the actual `Send`/`Sync` impls flow from the
/// `HashMap<TypeId, Box<dyn Any + Send + Sync>>` interior.
pub struct Keymap<Ctx> {
    scopes: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
    framework_globals: ScopeMap<GlobalAction>,
    _ctx: PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> Keymap<Ctx> {
    /// Entry point. No required dispatch hooks — under the post-Phase-3
    /// design the framework owns dispatch for every `GlobalAction`
    /// variant. The binary registers optional hooks on the builder
    /// (`KeymapBuilder::on_quit` / `on_restart` / `dismiss_fallback`).
    pub fn builder() -> KeymapBuilder<Ctx> { KeymapBuilder::new() }

    /// Pane scope lookup. `pub` — input handlers and bar code call this.
    pub fn scope_for<P: Shortcuts<Ctx>>(&self) -> &ScopeMap<P::Action> {
        self.scopes
            .get(&TypeId::of::<P>())
            .and_then(|b| b.downcast_ref::<ScopeMap<P::Action>>())
            .expect("scope_for: pane not registered")
    }

    /// Navigation scope lookup. `pub` — base-pane input handlers call.
    pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Action> {
        self.scopes
            .get(&TypeId::of::<N>())
            .and_then(|b| b.downcast_ref::<ScopeMap<N::Action>>())
            .expect("navigation: scope not registered")
    }

    /// App-globals scope lookup. `pub`.
    pub fn globals<G: Globals<Ctx>>(&self) -> &ScopeMap<G::Action> {
        self.scopes
            .get(&TypeId::of::<G>())
            .and_then(|b| b.downcast_ref::<ScopeMap<G::Action>>())
            .expect("globals: scope not registered")
    }

    /// Framework-internal scope. `pub(crate)` — only the framework
    /// dispatcher and the bar's `Global` region read it; the binary
    /// never names `GlobalAction` directly.
    pub(crate) fn framework_globals(&self) -> &ScopeMap<GlobalAction> {
        &self.framework_globals
    }

    /// Returns `{dirs::config_dir()}/<name>/keymap.toml` for use with
    /// `KeymapBuilder::load_toml`. App supplies its own name (e.g.
    /// `"cargo-port"`). `pub` and free of `Ctx` because it does not
    /// touch the keymap state.
    pub fn config_path(name: &str) -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_default();
        p.push(name);
        p.push("keymap.toml");
        p
    }
}
```

**Tradeoff.** `framework_globals` is `pub(crate)` — exposing it would let app code rebind framework-owned actions outside the builder, which the design explicitly rejects (the framework owns `GlobalAction` dispatch, full stop). `config_path` is associated rather than free so callers find it on the type they already mention.

---

## 7. `KeymapBuilder<Ctx>`

```rust
use std::collections::HashMap;
use std::path::Path;

/// Builder for `Keymap<Ctx>`. Construction is a flat builder (no
/// type-state) — the binary calls `register` and friends in any order,
/// and `build()` validates required pieces at runtime by returning
/// `Result<Keymap<Ctx>, BuilderError>`.
pub struct KeymapBuilder<Ctx: AppContext> {
    // Optional framework-lifecycle cleanup hooks (post-Phase-3 design:
    // framework owns Quit/Restart/Dismiss dispatch; binary opts in to
    // notification via these hooks). All three default to `None`.
    on_quit:          Option<fn(&mut Ctx)>,
    on_restart:       Option<fn(&mut Ctx)>,
    dismiss_fallback: Option<fn(&mut Ctx) -> bool>,
    vim:              VimMode,
    /// Per-scope `Bindings<A>` boxed via `Any`, keyed by `TypeId<P>` /
    /// `TypeId<N>` / `TypeId<G>`.
    pending: HashMap<TypeId, PendingScope<Ctx>>,
    /// Per-AppPaneId input-mode queries, captured at `register::<P>()`.
    input_mode_queries: HashMap<Ctx::AppPaneId, fn(&Ctx) -> InputMode>,
    settings:    SettingsRegistry<Ctx>,
    nav_registered: bool,
    globals_registered: bool,
    toml_path: Option<PathBuf>,
}

impl<Ctx: AppContext> KeymapBuilder<Ctx> {
    /// Optional cleanup hook fired after the framework processes a
    /// `Quit` action (just before the main loop exits).
    pub fn on_quit(mut self, f: fn(&mut Ctx)) -> Self { self.on_quit = Some(f); self }

    /// Optional cleanup hook fired after the framework processes a
    /// `Restart` action (just before the main loop tears down).
    pub fn on_restart(mut self, f: fn(&mut Ctx)) -> Self { self.on_restart = Some(f); self }

    /// Optional fallback dismiss handler. The framework's own dismiss
    /// chain (toasts, focused framework overlay) runs first; if nothing
    /// is dismissed at the framework level, this fn is called. Returns
    /// `true` if the binary handled the dismiss, `false` to no-op.
    pub fn dismiss_fallback(mut self, f: fn(&mut Ctx) -> bool) -> Self {
        self.dismiss_fallback = Some(f);
        self
    }

    /// Toggle vim mode. Optional — defaults to `VimMode::Disabled`.
    pub fn vim_mode(mut self, mode: VimMode) -> Self { self.vim = mode; self }

    /// Register a pane's `Shortcuts<Ctx>` impl. Repeatable. Required
    /// at least once in practice (a keymap with zero pane scopes has
    /// nothing to dispatch), but not enforced — `build()` succeeds.
    pub fn register<P: Shortcuts<Ctx>>(mut self) -> Self { /* … */ self }

    /// Register the navigation scope. **Required.** `build()` returns
    /// `Err(BuilderError::NavigationMissing)` if omitted.
    pub fn with_navigation<N: Navigation<Ctx>>(mut self) -> Self { /* … */ self }

    /// Register the app-globals scope. **Required.** `build()` returns
    /// `Err(BuilderError::GlobalsMissing)` if omitted.
    pub fn with_globals<G: Globals<Ctx>>(mut self) -> Self { /* … */ self }

    /// Populate the settings registry. Optional — apps with no
    /// configurable settings skip it.
    pub fn with_settings(
        mut self,
        f: impl FnOnce(&mut SettingsRegistry<Ctx>),
    ) -> Self {
        f(&mut self.settings);
        self
    }

    /// Optionally apply user TOML. Returns `Self` (not `Result<Self>`)
    /// because TOML errors are deferred to `build()` — the chain stays
    /// fluent. Errors surface as `BuilderError::Toml(KeymapError)`.
    pub fn load_toml(mut self, path: impl AsRef<Path>) -> Self {
        self.toml_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Validate, fold defaults + TOML + vim, and produce the final
    /// `Keymap<Ctx>`.
    pub fn build(self) -> Result<Keymap<Ctx>, BuilderError> { /* … */ unimplemented!() }
}

/// Errors the builder can surface at `build()` time. All required
/// pieces missing produce a distinct variant — the binary fixes the
/// call site rather than running into a panic.
#[derive(Debug)]
pub enum BuilderError {
    /// `with_navigation::<N>()` was never called.
    NavigationMissing,
    /// `with_globals::<G>()` was never called.
    GlobalsMissing,
    /// TOML file present but unreadable / unparseable / contains
    /// cross-action collisions.
    Toml(KeymapError),
    /// The same pane type was registered twice.
    DuplicateScope { type_name: &'static str },
}

impl std::fmt::Display for BuilderError { /* … */ }
impl std::error::Error for BuilderError {}
```

**Tradeoff.** Flat builder + `Result<Keymap<Ctx>, BuilderError>` from `build()` over a type-state builder. Type-state would catch the two required-method omissions at compile time but at the cost of two extra type parameters on `KeymapBuilder` and a less-readable signature for every intermediate state. The runtime `Result` matches Rust idiom for fallible startup config (TOML errors are fallible regardless), and the binary calls `build()` exactly once at startup so the cost of catching the error there is one `?`.

`load_toml` defers errors to `build()` so the chain reads top-to-bottom without a `?` mid-chain. Framework panes (Toasts, SettingsPane, KeymapPane) reach `&mut Framework<Ctx>` from their free-fn dispatchers via `ctx.framework_mut()` on the `AppContext` trait — no separate accessor hook is needed.

---

## 8. `Framework<Ctx>`

```rust
/// Aggregator owned by the binary's `App`. Holds the three framework
/// panes, the per-AppPaneId input-mode query registry, and the
/// currently focused pane. Focus is owned here (not on the
/// `AppContext` trait) so framework code reads `self.focused` instead
/// of calling back through `Ctx`.
///
/// All three pane fields are `pub` so the binary can pass references
/// directly to its bar-render dispatch (`bar::render(&app.framework.toasts, …)`)
/// without going through accessor methods. Mutation goes through each
/// pane's typed methods, not field assignment.
pub struct Framework<Ctx: AppContext> {
    pub keymap_pane:   KeymapPane<Ctx>,
    pub settings_pane: SettingsPane<Ctx>,
    pub toasts:        Toasts<Ctx>,
    /// Currently focused pane. The binary's `Focus` subsystem (richer
    /// policy: overlay-return memory, visited set, `pane_state`) is a
    /// layer above this field — it writes here as part of every
    /// transition.
    focused: FocusedPane<Ctx::AppPaneId>,
    /// Per-AppPaneId input-mode queries, populated by
    /// `KeymapBuilder::register`. Framework panes (Keymap, Settings,
    /// Toasts) are special-cased in `focused_pane_input_mode` and do
    /// not appear here.
    pub(crate) input_mode_queries: HashMap<Ctx::AppPaneId, fn(&Ctx) -> InputMode>,
}

impl<Ctx: AppContext> Framework<Ctx> {
    /// Construct an empty framework with an explicit initial focus.
    /// The binary creates one at startup and stores it on `App`.
    /// `KeymapBuilder::build` later writes the `input_mode_queries`
    /// map into it via `ctx.framework_mut()`.
    pub fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self {
        Self {
            keymap_pane:   KeymapPane::new(),
            settings_pane: SettingsPane::new(),
            toasts:        Toasts::new(),
            focused:       initial_focus,
            input_mode_queries: HashMap::new(),
        }
    }

    /// Returns the currently focused pane.
    pub fn focused(&self) -> FocusedPane<Ctx::AppPaneId> { self.focused }

    /// Sets the focused pane. The binary's `Focus` subsystem calls
    /// this from its existing transitions; framework code dispatching
    /// `GlobalAction::{NextPane, PrevPane, OpenKeymap, OpenSettings,
    /// Dismiss}` routes through the binary's `Focus` adapter, which
    /// in turn calls here.
    pub fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) {
        self.focused = focus;
    }

    /// Editor-target path for the focused overlay pane (Settings or
    /// Keymap). Replaces today's `overlay_editor_target_path` helper.
    pub fn editor_target_path(&self) -> Option<&Path> {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.editor_target(),
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.editor_target(),
            _ => None,
        }
    }

    /// Resolve the focused pane's `InputMode`. Used by the structural
    /// Esc pre-handler and the bar-region suppression logic.
    pub fn focused_pane_input_mode(&self, ctx: &Ctx) -> InputMode {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.input_mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.input_mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Toasts)   => InputMode::Static,
            FocusedPane::App(app)                              => self.input_mode_queries
                .get(&app)
                .map_or(InputMode::Navigable, |q| q(ctx)),
        }
    }
}
```

**Tradeoff.** Public fields for the three framework panes — the binary uses them directly in `bar::render` dispatch and input routing. The `focused` field is private (read via `focused()`, written via `set_focused`) so the binary's `Focus` subsystem stays the only writer. The `input_mode_queries` map is `pub(crate)` because only `KeymapBuilder` writes it (through `ctx.framework_mut()` at `build()` time) and only `focused_pane_input_mode` reads it. `Framework::new` takes an explicit `initial_focus` rather than implementing `Default`; the binary picks the starting pane.

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

impl ActionEnum for GlobalAction {
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

**Tradeoff.** `GlobalAction` impls `ActionEnum` so it can flow through the same `ScopeMap` / TOML / display machinery as every other scope. The binary never names it; the impl exists purely so framework-internal code reuses one path.

---

## 11. Module hierarchy and `lib.rs` re-exports

**Canonical source.** This section is the single authoritative definition of the `tui_pane` foundations module hierarchy (split across Phases 2–9 of `tui-pane-lib.md`). Each sub-phase references this section for its file list; do not duplicate the file list elsewhere. The directories declared here (`keymap/`, `bar/`, `panes/`, plus `settings.rs`) are the same skeletons that Phases 10 and 11 fill in.

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
    ActionEnum, Bindings, BuilderError, GlobalAction, Globals, KeyBind,
    KeyInput, KeyParseError, Keymap, KeymapBuilder, KeymapError, Navigation,
    ScopeMap, Shortcuts, VimMode,
};

// --- Bar -------------------------------------------------------------
pub use crate::bar::{BarRegion, BarSlot, InputMode, ShortcutState};

// --- Framework panes + aggregator -----------------------------------
pub use crate::framework::Framework;
pub use crate::panes::{KeymapPane, SettingsPane, Toasts, ToastsAction};
pub use crate::settings::SettingsRegistry;

// --- Pane identity ---------------------------------------------------
//
// The framework defines its own three-variant pane id and a wrapper over
// the binary's pane id. `AppContext` is the trait every `Ctx` must impl.
// See `paneid-ctx.md` for the full split rationale (option (d)).
pub use crate::app_context::AppContext;
pub use crate::pane_id::{FrameworkPaneId, FocusedPane};

// --- Macros -----------------------------------------------------------
//
// `#[macro_export]` already places `bindings!` and `action_enum!` at the
// crate root — no `pub use` needed. The Phase 4 retrospective confirmed
// the speculative `pub use crate::keymap::bindings::bindings;` line was
// unnecessary; do not add it.
```

**Final list of exported names** (alphabetical, grouped):

- **Action machinery:** `ActionEnum`, `GlobalAction`, `action_enum!`
- **Keys:** `KeyBind`, `KeyParseError`
- **Bindings & maps:** `Bindings`, `ScopeMap`, `bindings!`
- **Keymap:** `Keymap`, `KeymapBuilder`, `BuilderError`, `KeymapError`, `VimMode`
- **Traits:** `Shortcuts`, `Navigation`, `Globals`
- **Bar:** `BarRegion`, `BarSlot`, `ShortcutState`, `InputMode`
- **Framework panes & aggregator:** `Framework`, `KeymapPane`, `SettingsPane`, `Toasts`, `ToastsAction`, `SettingsRegistry`
- **Pane identity & context:** `AppContext`, `FrameworkPaneId`, `FocusedPane`

**Tradeoff.** Every `pub use` is from one of five top-level modules (`bar`, `framework`, `keymap`, `panes`, `settings`). The crate root is the only public path; internal module paths are not part of the API. This keeps the binary's `use tui_pane::{KeyBind, Keymap, Shortcuts, …};` flat and stable across internal reorganisations.
