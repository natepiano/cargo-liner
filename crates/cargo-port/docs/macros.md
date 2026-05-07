# `tui_pane` macros — formal specification

This document formalizes the two declarative macros that ship with the
`tui_pane` workspace crate extracted from cargo-port:

- `bindings!` — constructs a `Bindings<A>` from `key => Action` rules.
- `action_enum!` — declares an action enum plus its `Action` impl.

Companion to `docs/tui-pane-lib.md` (the design doc); this file is the
source of truth for macro syntax and expansion.

---

## 1. `bindings!` macro

### 1.1 Grammar

`macro_rules!` matcher form:

```text
bindings_input := <empty> | rule_list
rule_list      := rule ( "," rule )* ","?
rule           := key_spec "=>" action_expr
key_spec       := single_key | key_list
single_key     := $key:expr
key_list       := "[" $key:expr ( "," $key:expr )+ "]"
action_expr    := $action:expr
```

Notes:

- `single_key` accepts any expression of type `T: Into<KeyBind>` —
  `KeyCode::Enter`, `'c'`, `KeyBind::shift('g')`, `KeyBind::ctrl('k')`,
  pre-built `KeyBind` values, etc.
- `key_list` requires at least two keys (`+` repetition with a leading
  required element). A one-element bracketed list `[KeyCode::Enter]` is
  rejected; users write `KeyCode::Enter` instead. Two or more is the
  multi-bind form.
- `action_expr` is a single expression of type `A` (the action enum).
- The body may be empty (`bindings! {}` produces `Bindings::new()`),
  trailing comma is optional, and single-key and list-key rules may be
  freely mixed.

### 1.2 `macro_rules!` definition

This is the body that lives in `tui_pane/src/keymap/bindings.rs` and is
re-exported at the crate root via `#[macro_export]`.

```rust
#[macro_export]
macro_rules! bindings {
    // Empty body.
    () => {
        $crate::Bindings::new()
    };

    // One or more rules, optional trailing comma.
    ( $( $rule:tt )+ ) => {{
        let mut __b: $crate::Bindings<_> = $crate::Bindings::new();
        $crate::__bindings_rules!(__b ; $( $rule )+);
        __b
    }};
}

// Internal recursive helper. Not part of the public API.
#[doc(hidden)]
#[macro_export]
macro_rules! __bindings_rules {
    // Terminal: no rules left.
    ( $b:ident ; ) => {};

    // List form, with trailing comma.
    ( $b:ident ; [ $first:expr $(, $rest:expr )+ ] => $action:expr , $( $tail:tt )* ) => {
        $b.bind_many(
            [
                ::core::convert::Into::<$crate::KeyBind>::into($first),
                $( ::core::convert::Into::<$crate::KeyBind>::into($rest), )+
            ],
            $action,
        );
        $crate::__bindings_rules!($b ; $( $tail )*);
    };

    // List form, final rule (no trailing comma).
    ( $b:ident ; [ $first:expr $(, $rest:expr )+ ] => $action:expr ) => {
        $b.bind_many(
            [
                ::core::convert::Into::<$crate::KeyBind>::into($first),
                $( ::core::convert::Into::<$crate::KeyBind>::into($rest), )+
            ],
            $action,
        );
    };

    // Single-key form, with trailing comma.
    ( $b:ident ; $key:expr => $action:expr , $( $tail:tt )* ) => {
        $b.bind($key, $action);
        $crate::__bindings_rules!($b ; $( $tail )*);
    };

    // Single-key form, final rule (no trailing comma).
    ( $b:ident ; $key:expr => $action:expr ) => {
        $b.bind($key, $action);
    };
}
```

The two-macro split (public `bindings!` + private
`__bindings_rules!`) is needed because `macro_rules!` cannot match a
list arm and a single-`expr` arm in the same alternation without
running into the `expr` follow-set restriction (the `[ … ]` arm has to
be tried before `$key:expr`, which a single-pattern macro can't
guarantee). Recursive descent over a `tt` stream resolves it.

### 1.3 Expansion examples

#### Example A — single-key only

Input:

```rust
bindings! {
    KeyCode::Enter => PackageAction::Activate,
    'c'            => PackageAction::Clean,
}
```

Expansion:

```rust
{
    let mut __b: ::tui_pane::Bindings<_> = ::tui_pane::Bindings::new();
    __b.bind(KeyCode::Enter, PackageAction::Activate);
    __b.bind('c', PackageAction::Clean);
    __b
}
```

#### Example B — mixed single/list, no trailing comma

Input:

```rust
bindings! {
    [KeyCode::Up, 'k']  => NavigationAction::Up,
    ['=', '+']          => ProjectListAction::ExpandAll,
    '-'                 => ProjectListAction::CollapseAll
}
```

Expansion:

```rust
{
    let mut __b: ::tui_pane::Bindings<_> = ::tui_pane::Bindings::new();
    __b.bind_many(
        [
            ::core::convert::Into::<::tui_pane::KeyBind>::into(KeyCode::Up),
            ::core::convert::Into::<::tui_pane::KeyBind>::into('k'),
        ],
        NavigationAction::Up,
    );
    __b.bind_many(
        [
            ::core::convert::Into::<::tui_pane::KeyBind>::into('='),
            ::core::convert::Into::<::tui_pane::KeyBind>::into('+'),
        ],
        ProjectListAction::ExpandAll,
    );
    __b.bind('-', ProjectListAction::CollapseAll);
    __b
}
```

#### Example C — `KeyBind` constructors on the left

Input:

```rust
bindings! {
    KeyBind::shift('g') => SettingsAction::ToggleNext,
    KeyBind::ctrl('k')  => GlobalAction::OpenKeymap,
}
```

Expansion:

```rust
{
    let mut __b: ::tui_pane::Bindings<_> = ::tui_pane::Bindings::new();
    __b.bind(KeyBind::shift('g'), SettingsAction::ToggleNext);
    __b.bind(KeyBind::ctrl('k'),  GlobalAction::OpenKeymap);
    __b
}
```

`Bindings::bind` takes `key: impl Into<KeyBind>`, so single-key rules
do not need an explicit `Into` call — the trait bound on `bind`
handles `KeyCode`, `char`, and `KeyBind` uniformly. List rules use
`Bindings::bind_many` which takes `IntoIterator<Item = KeyBind>`, so
the macro inserts an explicit `Into::<KeyBind>::into(...)` per element
to coerce mixed `KeyCode`/`char`/`KeyBind` values into a single
homogeneous array.

### 1.4 Edge cases

| Case | Behavior |
|---|---|
| `bindings! {}` (empty body) | First arm fires, expands to `Bindings::new()`. |
| Trailing comma after final rule | Handled by the two-arm pattern inside `__bindings_rules!` — one arm matches `, $( $tail:tt )*` (recurse into the rest) and one matches a final rule with no trailing tokens. Trailing comma is optional, never required. |
| Mixed single + list rules in one block | Each rule independently matches a `__bindings_rules!` arm; order is preserved. |
| Same action on multiple keys (multi-bind) | Allowed. Either via list form `[K1, K2] => A` (one `bind_many` call) or via repeated single-key rules `K1 => A, K2 => A` (two `bind` calls). Both populate `ScopeMap::by_action[A] = vec![K1, K2]` in insertion order; the first key inserted is the **primary**. |
| Same key bound to two different actions in the same scope | Compiles, but at runtime `ScopeMap::insert` fires its `debug_assert!` ("`ScopeMap::insert: key {key:?} already maps to a different action`") in debug builds. In release builds the second insertion silently overwrites `by_key` — an accepted compromise; the assert catches regressions in dev/test. The macro itself does not (and cannot, without a proc-macro) detect this at compile time because keys are arbitrary expressions. |
| Same key bound to the same action twice | Idempotent in `by_key`; appends a duplicate entry to `by_action[A]` (insertion order is preserved). The primary-key invariant test (`by_key.len() == sum of by_action vec lengths`) is the canary if this ever happens; production callers should not duplicate. |
| One-element bracketed list `[K] => A` | Rejected: macro arm requires `$( , $rest:expr )+` (one or more after the first). User writes `K => A` instead. |

### 1.5 Doctest

```rust
/// ```
/// use tui_pane::{bindings, action_enum, Bindings, KeyBind, KeyCode, Ctx};
///
/// action_enum! {
///     #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
///     pub enum NavAction {
///         Up   => ("up",   "up",   "Move up");
///         Down => ("down", "down", "Move down");
///     }
/// }
///
/// let b: Bindings<NavAction> = bindings! {
///     [KeyCode::Up,   'k'] => NavAction::Up,
///     [KeyCode::Down, 'j'] => NavAction::Down,
/// };
///
/// // Realize into a ScopeMap and probe a lookup.
/// let scope = b.into_scope_map();
/// assert_eq!(scope.action_for(&KeyBind::from(KeyCode::Up)), Some(NavAction::Up));
/// assert_eq!(scope.action_for(&KeyBind::from('j')),         Some(NavAction::Down));
/// assert_eq!(scope.key_for(NavAction::Up),                   Some(&KeyBind::from(KeyCode::Up)));
///
/// // `Ctx` from tui_pane is the public fixture context type for examples.
/// let _ctx_marker = std::marker::PhantomData::<Ctx>;
/// ```
```

The doctest uses `Bindings::into_scope_map` (a method already implied by
the design doc — `Bindings<A>` is the staging buffer that the builder
folds into a `ScopeMap<A>`). If that method ends up named differently
in the implementation, this doctest needs to track the chosen name.

---

## 2. `action_enum!` macro

### 2.1 `Action` trait — formal definition

Lives in `tui_pane/src/keymap/action_enum.rs`, re-exported at the crate root.

```rust
/// Marker plus minimal vocabulary every action enum implements.
///
/// Implemented automatically by the `action_enum!` macro. Hand-rolled
/// impls are allowed but unusual; the macro is the supported path.
pub trait Action:
    Copy + Eq + ::core::hash::Hash + ::core::fmt::Debug + ::core::fmt::Display + 'static
{
    /// Every variant of `Self`, in declaration order. Stable across runs.
    const ALL: &'static [Self];

    /// Identifier used in TOML config keys (e.g. `"activate"`,
    /// `"expand_all"`). Must be stable — TOML files are user-edited.
    fn toml_key(self) -> &'static str;

    /// Default short label shown in the bar (e.g. `"activate"`,
    /// `"clean"`). The pane's `Shortcuts::label` returns this by
    /// default; overrides only fire when the label is state-dependent.
    fn bar_label(self) -> &'static str;

    /// Human-readable description (used by the keymap-overlay help).
    /// `Display::fmt` delegates to this.
    fn description(self) -> &'static str;

    /// Inverse of `toml_key`. Returns `None` for unknown identifiers.
    fn from_toml_key(key: &str) -> Option<Self>;
}
```

Super-traits chosen so generic code (`ScopeMap<A: Action>`,
keymap-overlay rendering, TOML round-trip) needs only one bound, not
five. `Copy + Eq + Hash` cover dispatch and registry use; `Debug +
Display + 'static` cover error reporting and trait-object-free generic
code.

### 2.2 Macro body

```rust
#[macro_export]
macro_rules! action_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Name:ident {
            $( $Variant:ident => ( $toml_key:literal , $bar:literal , $desc:literal ) ; )+
        }
    ) => {
        $(#[$meta])*
        $vis enum $Name {
            $( $Variant, )+
        }

        impl $crate::Action for $Name {
            const ALL: &'static [Self] = &[ $( Self::$Variant, )+ ];

            fn toml_key(self) -> &'static str {
                match self {
                    $( Self::$Variant => $toml_key, )+
                }
            }

            fn bar_label(self) -> &'static str {
                match self {
                    $( Self::$Variant => $bar, )+
                }
            }

            fn description(self) -> &'static str {
                match self {
                    $( Self::$Variant => $desc, )+
                }
            }

            fn from_toml_key(key: &str) -> ::core::option::Option<Self> {
                match key {
                    $( $toml_key => ::core::option::Option::Some(Self::$Variant), )+
                    _ => ::core::option::Option::None,
                }
            }
        }

        impl ::core::fmt::Display for $Name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(<Self as $crate::Action>::description(*self))
            }
        }
    };
}
```

Design notes:

1. Repetition is `+` (at least one variant), not `*`. An empty action
   enum has no useful semantics — `ALL` would be empty, dispatch would
   never fire — so the macro rejects accidental empty bodies at
   expansion time.
2. Every method lands in `impl $crate::Action for $Name`. `ALL` is
   reachable as `MyAction::ALL` because Rust resolves trait-associated
   consts on the implementing type.
3. `toml_key` / `description` are not `const`. Trait methods cannot be
   `const` on stable Rust without `#![feature(const_trait_impl)]`.
4. `Display` is generated alongside `Action`, delegating to
   `description()`.
5. `$crate::Action` paths so the macro works when invoked from any
   downstream crate.

### 2.3 Items the macro generates — checklist

For `action_enum! { pub enum Foo { A => ("a", "a-bar", "alpha"); B => ("b", "b-bar", "beta"); } }`:

- `pub enum Foo { A, B }` — the enum.
- `impl tui_pane::Action for Foo` with:
  - `const ALL: &'static [Self] = &[Self::A, Self::B];`
  - `fn toml_key(self) -> &'static str` mapping `A => "a"`, `B => "b"`.
  - `fn bar_label(self) -> &'static str` mapping `A => "a-bar"`, `B => "b-bar"`.
  - `fn description(self) -> &'static str` mapping `A => "alpha"`, `B => "beta"`.
  - `fn from_toml_key("a") -> Some(A)`, `("b") -> Some(B)`, else `None`.
- `impl core::fmt::Display for Foo` delegating to `description`.

Not generated (must come from the user's `#[derive(...)]` meta or from
hand-rolled impls): `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`, `Hash`.
Existing call sites already write `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]`
before every `action_enum!` invocation; that pattern continues. The
`Action` super-trait bounds on `Copy + Eq + Hash + Debug` are
satisfied by the user's derives, not by the macro.

### 2.4 Doctest

```rust
/// ```
/// use tui_pane::{action_enum, Action};
///
/// action_enum! {
///     #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
///     pub enum DemoAction {
///         Activate => ("activate", "activate", "Open / activate selected field");
///         Clean    => ("clean",    "clean",    "Clean target dir");
///     }
/// }
///
/// // Trait-level constant.
/// assert_eq!(DemoAction::ALL.len(), 2);
/// assert_eq!(DemoAction::ALL[0], DemoAction::Activate);
///
/// // TOML round-trip.
/// assert_eq!(DemoAction::Activate.toml_key(), "activate");
/// assert_eq!(DemoAction::from_toml_key("clean"), Some(DemoAction::Clean));
/// assert_eq!(DemoAction::from_toml_key("nope"),  None);
///
/// // Bar-label default consumed by Shortcuts::label.
/// assert_eq!(DemoAction::Activate.bar_label(), "activate");
///
/// // Display delegates to description.
/// assert_eq!(format!("{}", DemoAction::Activate), "Open / activate selected field");
/// ```
```

---

## 3. Re-export plan

The macros exist to spare callers from importing crossterm types and
internal `tui_pane` paths separately. The crate root therefore re-exports
the small set of names every macro invocation refers to.

In `tui_pane/src/lib.rs`:

```rust
// Macros — defined with `#[macro_export]`, so they live at the crate
// root automatically. No `pub use` needed; downstream code writes
// `tui_pane::bindings!` / `tui_pane::action_enum!`.

// crossterm re-exports — let downstream code write `KeyCode::Enter`
// / `KeyModifiers::CONTROL` inside `bindings!` without a separate
// `use crossterm::event::...` line.
pub use crossterm::event::{KeyCode, KeyModifiers};

// Framework types referenced by the expanded macro bodies. These are
// the keymap module's facade re-exports, exposed flat at the crate root
// per Phase 4: `tui_pane::Bindings`, `tui_pane::KeyBind`, etc.
pub use crate::keymap::{Action, Bindings, KeyBind, ScopeMap};

// Public fixture context type — referenced by the doctests above and
// by every doctest on a generic public item that names a `Ctx`.
pub use crate::test_fixtures::Ctx;
```

Rules:

- `bindings!` and `action_enum!` are `#[macro_export]`, so they live
  at the crate root and are invoked as `tui_pane::bindings!` /
  `tui_pane::action_enum!` (or imported via
  `use tui_pane::{bindings, action_enum};`).
- `KeyCode` and `KeyModifiers` are re-exported because the design doc
  explicitly states "`crossterm::event::KeyCode` used directly. No
  alias." (line 316-317). Re-exporting from the crate root preserves
  that — users still see the real `KeyCode` type — while letting them
  write `tui_pane::KeyCode::Enter` if they prefer a single import.
- `KeyBind`, `Bindings`, `ScopeMap`, `Action` are re-exported
  because every macro expansion mentions one or more of them, and
  because they appear unprefixed in user code (`KeyBind::shift('g')`,
  `Bindings<NavAction>`).
- `Ctx` is re-exported per the doctest policy (design doc line 1150,
  1156).

---

## 4. Hygiene gotchas

### 4.1 `$crate` paths

Every name the expanded macro mentions resolves through `$crate`, the
absolute path to the crate that defined the macro. Without `$crate`,
the expansion would resolve names against the **call-site** module,
which breaks the moment someone invokes the macro outside `tui_pane`
itself (e.g. cargo-port).

The expansions therefore use:

- `$crate::Bindings` — the builder type.
- `$crate::KeyBind` — used in the explicit `Into::<$crate::KeyBind>::into(...)`
  coercion inside list-form rules.
- `$crate::Action` — the trait that `action_enum!` implements.

Standard-library paths use the `::core::` absolute prefix
(`::core::convert::Into`, `::core::fmt::Display`, `::core::option::Option`)
so the expansion is immune to a downstream crate that has shadowed
`Into`, `Display`, or `Option` in the call-site scope. `core` (not
`std`) is used because `tui_pane` itself does not require `std` for
any of these paths and the `::core::` prefix is shorter and equally
valid in `std`-using crates.

User-supplied tokens (`$key`, `$action`, `$Variant`, `$toml_key`,
`$desc`, `$Name`, `$vis`, `$meta`) flow through unchanged — they are
captured at the call site and resolve in the call-site scope, as
intended.

### 4.2 What must be in scope at the call site

For `bindings!`:

- The action enum type (e.g. `NavAction`) — referenced on the right
  side of `=>`.
- `KeyCode` / `KeyModifiers` / `KeyBind` — only if the call site
  literally writes `KeyCode::Enter` etc. as a key expression. The macro
  itself does not introduce or rely on the call-site scope having these
  names; they appear only because the user wrote them.
- The call site does **not** need `Bindings` in scope — the expansion
  uses `$crate::Bindings` exclusively.

For `action_enum!`:

- Nothing beyond what the user already wrote in the macro body. The
  user's `#[derive(...)]` attributes resolve in call-site scope, as do
  any types named in `$(#[$meta])*`. The trait impl uses
  `$crate::Action`, the `Display` impl uses `::core::fmt::*`, and
  the `from_toml_key` body uses `::core::option::Option` — all
  fully-qualified, all immune to call-site shadowing.

### 4.3 Name collisions inside the expansion

The `bindings!` expansion introduces one local binding (`__b`). The
double-underscore prefix is the conventional macro-hygiene marker —
`macro_rules!` hygiene already prevents user code from referring to it
by accident, but the prefix flags it as macro-internal for any reader
who expands `cargo expand`. The recursive helper threads `__b` through
explicitly so each rule's mutation is unambiguous in the expansion.

The `action_enum!` expansion introduces no local bindings. Trait impls
are at item position; no temporaries.

