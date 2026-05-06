# `tui_pane` TOML keymap format and `KeyBind` display mapping

This document specifies two formal pieces of the `tui_pane` framework:

1. The on-disk TOML grammar for `keymap.toml`, including the loader's error taxonomy and the vim-after-TOML interaction rules.
2. The `KeyBind` parse / display / `display_short` tables.

Companion to `docs/tui-pane-lib.md`. Where this doc and the design doc disagree, the design doc wins; raise a question rather than diverging silently.

---

## 1. TOML grammar

### 1.1 Top-level structure

A `keymap.toml` file is a sequence of TOML tables. Each top-level table name is a registered **scope name** matching some trait impl's `SCOPE_NAME` constant. Scopes the loader recognizes for cargo-port:

| Table header | Source | Action enum |
|---|---|---|
| `[base_global]` | `BaseGlobals` (framework) | `BaseGlobalAction` |
| `[global]` | `AppGlobals` impl | `AppGlobalAction` |
| `[navigation]` | `Navigation` impl | `NavigationAction` |
| `[project_list]` | `ProjectListPane` | `ProjectListAction` |
| `[package]` | `PackagePane` | `PackageAction` |
| `[git]` | `GitPane` | `GitAction` |
| `[targets]` | `TargetsPane` | `TargetsAction` |
| `[ci_runs]` | `CiRunsPane` | `CiRunsAction` |
| `[lints]` | `LintsPane` | `LintsAction` |
| `[finder]` | `FinderPane` | `FinderAction` |
| `[output]` | `OutputPane` | `OutputAction` |
| `[settings]` | `SettingsPane` (framework) | `SettingsAction` |
| `[keymap]` | `KeymapPane` (framework) | `KeymapPaneAction` |
| `[toasts]` | `Toasts` (framework) | `ToastsAction` |

A header that does not match any registered `SCOPE_NAME` produces `KeymapError::UnknownScope`. Scope names are matched literally and case-sensitively (TOML's own table-name semantics).

### 1.2 Per-scope grammar

Inside a scope table, each key is an **action TOML key** matching one of that scope's `action_enum!` `"toml_key"` literals. Values are one of:

- A single string: `key = "Enter"`.
- An array of strings: `key = ["Enter", "Return"]`.

Empty arrays are rejected (`InvalidKeyBind { reason: EmptyBindingList }`). Arrays may contain any positive number of strings; the **first** entry is the primary key (what the bar's paired-row renderer uses).

```ebnf
keymap         = { scope_table } ;
scope_table    = "[" scope_name "]" newline { binding } ;
binding        = action_key "=" binding_value newline ;
binding_value  = string | "[" string { "," string } "]" ;
string         = (* TOML basic or literal string *) ;
```

Unknown action keys within a recognized scope produce `KeymapError::UnknownAction`. They are skipped (other entries in the same scope still apply); the loader does not abort the file.

### 1.3 Replace-not-merge semantics

A scope table in TOML **replaces** the entire default binding set for that scope. Actions absent from the TOML scope table are **not** carried over from defaults — they fall back to unbound after TOML application, then framework re-applies vim extras (when enabled).

A scope **table not present at all** in TOML keeps the full default+vim binding set. Distinguishing "table omitted" from "table present but missing this action" is what gives the user a single uniform override mechanism: write the table to override, leave it out to keep defaults.

The loader records every action that was missing from a present-but-incomplete scope table in `KeymapLoadResult::missing_actions` (one entry per `scope.action`) and backfills the file with the resolved keymap on the next save.

### 1.4 Modifier syntax

A binding string is a `+`-separated list of modifier names followed by exactly one key token:

```
[Ctrl] [Alt] [Shift] [+] <key>
```

Rules:

- Modifier names: `Ctrl`, `Alt`, `Shift`. Aliases: `Control` for `Ctrl`, `Option` for `Alt`. Case-insensitive (`ctrl`, `CTRL`, `Ctrl` all parse).
- **Order is canonical: `Ctrl+Alt+Shift+<key>`.** The parser accepts any order; the formatter (`KeyBind::to_toml_string`) always emits canonical order.
- **At most one of each modifier.** Repeats (e.g. `"Ctrl+Ctrl+k"`) are rejected.
- The key token is the **last** segment after the final `+`. It is parsed by the rules in §1.5.
- The literal `"+"` and `"="` are special: a binding string of exactly `"+"` or `"="` is the `Char('+')` / `Char('=')` key with no modifiers (see §1.5 and §5).

### 1.5 Special key names

Recognized special-key tokens (case-insensitive). Each binds to a `KeyCode` directly with no modifier transformation:

| Token | `KeyCode` |
|---|---|
| `Enter` (alias `Return`) | `KeyCode::Enter` |
| `Esc` (alias `Escape`) | `KeyCode::Esc` |
| `Tab` | `KeyCode::Tab` |
| `Backspace` | `KeyCode::Backspace` |
| `Delete` (alias `Del`) | `KeyCode::Delete` |
| `Insert` (alias `Ins`) | `KeyCode::Insert` |
| `Home` | `KeyCode::Home` |
| `End` | `KeyCode::End` |
| `PageUp` | `KeyCode::PageUp` |
| `PageDown` | `KeyCode::PageDown` |
| `Up` | `KeyCode::Up` |
| `Down` | `KeyCode::Down` |
| `Left` | `KeyCode::Left` |
| `Right` | `KeyCode::Right` |
| `Space` | `KeyCode::Char(' ')` |
| `F1` … `F12` | `KeyCode::F(1)` … `KeyCode::F(12)` |

`F0` and `F13`+ are rejected (`InvalidKeyBind { reason: UnknownKeyToken }`).

### 1.6 Char syntax

Any single Unicode scalar value not consumed by the special-name table parses as `KeyCode::Char(c)`:

```
"a"   → Char('a')
"A"   → Char('A')           # uppercase implies Shift; Shift modifier is stripped
"="   → Char('=')
"+"   → Char('+')           # bare "+" parses as plus-key, not "modifier with no key"
"-"   → Char('-')
"/"   → Char('/')
" "   → Char(' ')           # but prefer "Space" for readability
"é"   → Char('é')
```

**No escape sequences.** TOML's own string escapes (`"\""`, `"\\"`) are the only mechanism for embedding `"` or `\`. The `,` `=` `+` characters need no escaping inside TOML strings; `=` and `+` parse as their `Char` codes when standing alone, and as modifier separators when surrounded by other tokens. To bind `Ctrl+=`, write `"Ctrl+="`; to bind `Ctrl++`, write `"Ctrl++"`.

### 1.7 Comments

TOML `#` line comments are accepted at any position permitted by TOML itself. The loader does not impose extra comment rules.

```toml
# end-of-line comment
[global]
quit = "q" # comment after value
```

---

## 2. Complete example `keymap.toml`

```toml
# cargo-port keymap configuration
# Edit bindings below. Format: action = "Key" | "Modifier+Key" | ["Key1", "Key2"]
# Modifiers: Ctrl, Alt, Shift. Examples: "Ctrl+r", "Shift+Tab", "q"

# ── Framework base globals (Quit / Restart / pane cycling / overlays) ─
[base_global]
quit          = "q"
restart       = "R"
next_pane     = "Tab"
prev_pane     = "Shift+Tab"
open_keymap   = "Ctrl+k"
open_settings = "s"
dismiss       = "x"

# ── App globals (cargo-port specific verbs) ──────────────────────────
[global]
rescan        = "Ctrl+r"
open_editor   = "e"
open_terminal = "t"
find          = "/"

# ── Navigation: arrow defaults; vim adds h/j/k/l on top when enabled ─
[navigation]
up        = "Up"
down      = "Down"
left      = "Left"
right     = "Right"
page_up   = "PageUp"
page_down = "PageDown"
home      = "Home"
end       = "End"

# ── Project list: paired ←/→ row, paired +/- row, plus Clean ─────────
[project_list]
expand_row   = "Right"           # vim mode appends 'l'
collapse_row = "Left"            # vim mode appends 'h'
expand_all   = ["+", "="]        # both physical keys, primary "+"
collapse_all = "-"
clean        = "c"

[package]
activate = "Enter"
clean    = "c"

[git]
activate = "Enter"
clean    = "c"

[targets]
activate      = "Enter"
release_build = "r"
clean         = "c"

[ci_runs]
activate    = "Enter"
fetch_more  = "f"
toggle_view = "v"
clear_cache = "d"

[lints]
activate      = "Enter"
clear_history = "d"

# ── Finder (text-input overlay): own navigation actions ──────────────
[finder]
activate   = "Enter"
cancel     = "Esc"
prev_match = "Up"
next_match = "Down"
home       = "Home"
end        = "End"

[output]
cancel = "Esc"

# ── Framework overlays ───────────────────────────────────────────────
[settings]
activate     = "Enter"
toggle_back  = "Left"
toggle_next  = "Right"
cancel       = ["Esc", "s"]      # close-on-toggle parity with today
confirm      = "Enter"

[keymap]
activate = "Enter"
clear    = "Delete"
cancel   = "Esc"

[toasts]
dismiss = ["Esc", "x"]
```

---

## 3. TOML error taxonomy

One unified enum; the loader returns a `Vec<KeymapError>` in `KeymapLoadResult` so a single bad entry does not abort the whole file.

```rust
pub struct KeymapError {
    pub scope:  String,          // empty if the error is file-level
    pub action: String,          // empty if the error is scope-level
    pub key:    String,          // raw TOML value that triggered the error
    pub reason: KeymapErrorReason,
}

pub enum KeymapErrorReason {
    /// Top-level table name is not a registered scope.
    UnknownScope,

    /// Action TOML key is not declared in the scope's `action_enum!`.
    UnknownAction,

    /// Binding string failed `KeyBind::from_str`. `reason` carries the
    /// parser's error variant (see `KeyParseError`).
    InvalidKeyBind(KeyParseError),

    /// Same string appeared twice in `key = ["Enter", "Enter"]`.
    InArrayDuplicate,

    /// Same `KeyBind` was assigned to two distinct actions in the
    /// same scope: `[finder] activate = "Enter"` and `cancel = "Enter"`.
    /// `other_action` names the action this key was already assigned to.
    CrossActionCollision { other_action: String },

    /// Action TOML key was removed in a previous version; the binary
    /// chose to filter it before passing to `tui_pane`. Reported only
    /// when the binary opts into legacy reporting.
    LegacyAction,

    /// Key parses but is reserved by the active vim mode (`'h'`, `'j'`,
    /// `'k'`, `'l'` with no modifiers when `VimMode::Enabled`).
    ReservedForVimMode,

    /// Key parses but is reserved by `NavigationAction` (`Up`, `Down`,
    /// `Left`, `Right`, `Home`, `End` with no modifiers). Only fires
    /// for non-navigation scopes.
    ReservedForNavigation,

    /// Binding parses but conflicts with a key already bound in the
    /// `base_global` or `global` scope.
    ConflictWithGlobal { global_action: String },

    /// File-level: I/O error reading the file.
    Io(String),

    /// File-level: TOML syntax error.
    TomlSyntax(String),

    /// Empty binding list: `key = []`.
    EmptyBindingList,
}

pub enum KeyParseError {
    /// String was empty or whitespace-only.
    Empty,
    /// Modifier appeared with no following key token: `"Ctrl+"`.
    ModifierWithoutKey,
    /// `"++"` — ambiguous modifier separator vs key token.
    AmbiguousPlusSeparator,
    /// Unknown modifier name: `"Hyper+k"`.
    UnknownModifier(String),
    /// Same modifier listed twice: `"Ctrl+Ctrl+k"`.
    DuplicateModifier(String),
    /// Token is neither a special name, F-key, nor single char.
    UnknownKeyToken(String),
    /// `F0` or `F13`+.
    FKeyOutOfRange(u8),
}
```

The loader's `Display` impl for `KeymapError` formats as `scope.action: "raw" — reason` (matching today's output at `keymap.rs:582-588`).

---

## 4. Vim-after-TOML interaction worked example

Defaults declared by `Navigation::defaults()` for `NavigationAction::Up`: `[Up]`. Vim extras for `Up`: `[Char('k')]`. The user's TOML may replace the entire `[navigation]` table; vim extras then re-apply.

### Case A — defaults + vim on, no TOML

Build steps:

1. `Navigation::defaults()` → `Up: [Up]`.
2. No `[navigation]` table in TOML → keep step 1.
3. Vim on → append `Char('k')` → `Up: [Up, Char('k')]`.

Resulting `ScopeMap<NavigationAction>` for `Up`:

```rust
ScopeMap {
    by_key: {
        KeyBind { code: Up,        mods: NONE } => NavigationAction::Up,
        KeyBind { code: Char('k'), mods: NONE } => NavigationAction::Up,
    },
    by_action: {
        NavigationAction::Up => vec![
            KeyBind { code: Up,        mods: NONE },  // primary
            KeyBind { code: Char('k'), mods: NONE },
        ],
    },
}
```

`key_for(Up)` returns the primary, `Up`. `display_keys_for(Up)` returns both, in `by_action` order.

### Case B — defaults + vim on + TOML `[navigation] up = "PageUp"`

Build steps:

1. Default `Up: [Up]`.
2. `[navigation]` table present → **replace** scope. After step 2: `Up: [PageUp]`. (No carryover of the default `Up`.)
3. Vim on → append `Char('k')` (skipping any binding already present) → `Up: [PageUp, Char('k')]`.

Resulting state:

```rust
ScopeMap {
    by_key: {
        KeyBind { code: PageUp,    mods: NONE } => NavigationAction::Up,
        KeyBind { code: Char('k'), mods: NONE } => NavigationAction::Up,
    },
    by_action: {
        NavigationAction::Up => vec![
            KeyBind { code: PageUp,    mods: NONE },  // primary (TOML's first)
            KeyBind { code: Char('k'), mods: NONE },
        ],
    },
}
```

`key_for(Up)` returns `PageUp`. The arrow-`Up` key is unbound; the user's TOML overrode it.

### Case C — defaults + vim off + TOML `[navigation] up = ["Up", "i"]`

Build steps:

1. Default `Up: [Up]`.
2. `[navigation]` present → replace → `Up: [Up, Char('i')]`.
3. Vim off → no append.

Resulting state:

```rust
ScopeMap {
    by_key: {
        KeyBind { code: Up,        mods: NONE } => NavigationAction::Up,
        KeyBind { code: Char('i'), mods: NONE } => NavigationAction::Up,
    },
    by_action: {
        NavigationAction::Up => vec![
            KeyBind { code: Up,        mods: NONE },  // primary
            KeyBind { code: Char('i'), mods: NONE },
        ],
    },
}
```

The vim append step is unconditional-on-mode but conditionally-on-already-bound. Since vim is off, step 3 is a no-op for every navigation action.

---

## 5. `KeyBind` parse table

Every TOML string token resolves to a `(KeyCode, KeyModifiers)` pair. Parser is implemented as `KeyBind::from_str`.

### 5.1 Char form

| Input | `KeyCode` | `KeyModifiers` |
|---|---|---|
| `"a"` | `Char('a')` | `NONE` |
| `"A"` | `Char('A')` | `NONE` (uppercase encodes Shift; SHIFT stripped) |
| `"z"` | `Char('z')` | `NONE` |
| `"0"` | `Char('0')` | `NONE` |
| `"="` | `Char('=')` | `NONE` |
| `"+"` | `Char('+')` | `NONE` |
| `"-"` | `Char('-')` | `NONE` |
| `"/"` | `Char('/')` | `NONE` |
| `" "` | `Char(' ')` | `NONE` |
| `"é"` | `Char('é')` | `NONE` |

Any single-codepoint string that is not a special-name match parses as `Char`.

### 5.2 Special-name form

| Input (case-insensitive) | `KeyCode` | `KeyModifiers` |
|---|---|---|
| `"Enter"` / `"Return"` | `Enter` | `NONE` |
| `"Esc"` / `"Escape"` | `Esc` | `NONE` |
| `"Tab"` | `Tab` | `NONE` |
| `"Backspace"` | `Backspace` | `NONE` |
| `"Delete"` / `"Del"` | `Delete` | `NONE` |
| `"Insert"` / `"Ins"` | `Insert` | `NONE` |
| `"Home"` | `Home` | `NONE` |
| `"End"` | `End` | `NONE` |
| `"PageUp"` | `PageUp` | `NONE` |
| `"PageDown"` | `PageDown` | `NONE` |
| `"Up"` | `Up` | `NONE` |
| `"Down"` | `Down` | `NONE` |
| `"Left"` | `Left` | `NONE` |
| `"Right"` | `Right` | `NONE` |
| `"Space"` | `Char(' ')` | `NONE` |
| `"F1"` … `"F12"` | `F(1)` … `F(12)` | `NONE` |

### 5.3 Modifier form

The string is split on `+`. The last segment is the key token; preceding segments are modifier names.

| Input | `KeyCode` | `KeyModifiers` |
|---|---|---|
| `"Ctrl+K"` | `Char('K')` | `CONTROL` (uppercase encodes Shift; SHIFT stripped) |
| `"Ctrl+k"` | `Char('k')` | `CONTROL` |
| `"Shift+G"` | `Char('G')` | `NONE` (Shift folded into uppercase) |
| `"Shift+g"` | `Char('G')` | `NONE` (Shift+lowercase normalized to uppercase) |
| `"Shift+Tab"` | `Tab` | `SHIFT` |
| `"Ctrl+Shift+P"` | `Char('P')` | `CONTROL` |
| `"Ctrl+Shift+p"` | `Char('P')` | `CONTROL` |
| `"Alt+d"` | `Char('d')` | `ALT` |
| `"Ctrl+Alt+Shift+F1"` | `F(1)` | `CONTROL \| ALT \| SHIFT` |
| `"Ctrl+Up"` | `Up` | `CONTROL` |
| `"Ctrl++"` | `Char('+')` | `CONTROL` |
| `"Ctrl+="` | `Char('=')` | `CONTROL` |

**Modifier order rule.** The parser accepts any order. The formatter emits canonical order: `Ctrl`, `Alt`, `Shift`, then key. So `"Shift+Ctrl+k"` parses but round-trips as `"Ctrl+Shift+k"`.

**Modifier case rule.** Modifier names are matched case-insensitively. The formatter always emits TitleCase (`Ctrl`, `Alt`, `Shift`). Aliases (`Control`, `Option`) parse but are normalized away on format.

**Shift+letter normalization.** `KeyBind::new` strips `SHIFT` when the code is an uppercase ASCII letter, and uppercases lowercase ASCII letters when `SHIFT` is set. This guarantees `"Shift+r"`, `"R"`, and the crossterm event `Char('R') + SHIFT` are all equal `KeyBind` values. (Existing behavior; see `keymap.rs:30-48`.)

### 5.4 Reject set

| Input | Error |
|---|---|
| `""` | `KeyParseError::Empty` |
| `"   "` | `KeyParseError::Empty` (trimmed) |
| `"Ctrl+"` | `KeyParseError::ModifierWithoutKey` |
| `"++"` | `KeyParseError::AmbiguousPlusSeparator` |
| `"+++"` | `KeyParseError::AmbiguousPlusSeparator` |
| `"Hyper+k"` | `KeyParseError::UnknownModifier("Hyper")` |
| `"Ctrl+Ctrl+k"` | `KeyParseError::DuplicateModifier("Ctrl")` |
| `"F0"` | `KeyParseError::FKeyOutOfRange(0)` |
| `"F13"` | `KeyParseError::FKeyOutOfRange(13)` |
| `"abc"` | `KeyParseError::UnknownKeyToken("abc")` |
| `"Ctrl+abc"` | `KeyParseError::UnknownKeyToken("abc")` |

---

## 6. `KeyBind::display` mapping table (full names)

Used by the keymap-overlay UI where horizontal space is generous. Every modifier renders as a glyph prefix; the key token renders as a full name.

Modifier glyph prefixes (canonical order, concatenated):

| Modifier | Glyph |
|---|---|
| `CONTROL` | `⌃` |
| `ALT` | `⌥` |
| `SHIFT` | `⇧` |

`KeyCode` → display string:

| `KeyCode` | Display |
|---|---|
| `Char('=')` | `+` (the `=`/`+` physical key normalizes to `Char('=')`; surfaces as `+`) |
| `Char('+')` | `+` (after normalize, never reaches display as `Char('+')`) |
| `Char(c)` (any other) | `c.to_string()` — single character |
| `Enter` | `Enter` |
| `Esc` | `Esc` |
| `Tab` / `BackTab` | `Tab` |
| `Backspace` | `Backspace` |
| `Delete` | `Delete` |
| `Insert` | `Insert` |
| `Home` | `Home` |
| `End` | `End` |
| `PageUp` | `PageUp` |
| `PageDown` | `PageDown` |
| `Up` | `Up` |
| `Down` | `Down` |
| `Left` | `Left` |
| `Right` | `Right` |
| `F(n)` | `F{n}` (e.g. `F1`, `F12`) |
| any other | `format!("{:?}")` fallback |

Examples:

- `KeyBind { Char('q'), NONE }` → `"q"`
- `KeyBind { Char('R'), NONE }` → `"R"`
- `KeyBind { Char('k'), CONTROL }` → `"⌃k"`
- `KeyBind { Tab, SHIFT }` → `"⇧Tab"`
- `KeyBind { F(1), CONTROL | ALT }` → `"⌃⌥F1"`
- `KeyBind { Char('='), NONE }` → `"+"`

---

## 7. `KeyBind::display_short` mapping table (glyphs)

Used by the status bar where space is tight. Identical to `display` except the four arrow keys render as Unicode arrows.

| `KeyCode` | `display_short` | `display` |
|---|---|---|
| `Up` | `↑` | `Up` |
| `Down` | `↓` | `Down` |
| `Left` | `←` | `Left` |
| `Right` | `→` | `Right` |
| every other variant | (delegates to `display`) | (same) |

Modifier glyph prefixes are identical to §6.

Examples:

- `KeyBind { Up, NONE }`.display_short → `"↑"`
- `KeyBind { Down, SHIFT }`.display_short → `"⇧↓"`
- `KeyBind { Tab, NONE }`.display_short → `"Tab"` (delegates)
- `KeyBind { Char('='), NONE }`.display_short → `"+"` (delegates)
- `KeyBind { Char('/'), NONE }`.display_short → `"/"` (delegates — see invariant below)

### Paired-row invariant

No `KeyCode` variant produces a `display_short` string that contains `,` or `/`. The framework's paired-row renderer uses `"/"` as the separator (`↑/↓`) and the single-row renderer uses `","` to join multiple keys (`Up,k`); both must be unambiguous.

The invariant holds for every variant in the table above:

- Arrow glyphs `↑↓←→` are not `,` or `/`.
- Special names (`Enter`, `Tab`, `F1`, …) are alphanumeric.
- `Char(c)` cases:
  - `Char('=')` displays as `+` — clean.
  - `Char(',')` would display as `,` — **forbidden**. The parser still accepts `","` as a binding, but the framework rejects it during build via a `debug_assert!` (and a Phase 2 unit test walks every `KeyCode` variant the parser can produce).
  - `Char('/')` displays as `/` — same problem. Rejected with the same mechanism.

A user who writes `key = ","` or `key = "/"` for an action that lands in a paired row gets a build-time assertion in debug builds; in release, the bar will render an ambiguous string. The parser-level fix would be to reject `,` and `/` outright, but that loses the ability to bind them to single-row actions like `Find` (`'/'` is its default). The chosen rule: parser permits, paired-row renderer asserts.

---

## 8. Round-trip property test spec

### Property

For every `KeyBind` produced by the parser, formatting it with `to_toml_string` and re-parsing yields the original `KeyBind`:

```rust
fn round_trip(input: &str) {
    let kb: KeyBind = input.parse().unwrap();
    let serialized = kb.to_toml_string();
    let reparsed: KeyBind = serialized.parse().unwrap();
    assert_eq!(kb, reparsed, "round-trip failed for {input:?}");
}
```

The test runs against:

- All special-name tokens from §1.5 (with and without each modifier subset).
- All ASCII-printable single chars including `"="`, `"+"`, `"-"`, `"/"`, `" "`.
- All `F1`–`F12` (with and without each modifier subset).
- Random sampling of multi-modifier combinations.

### Canonical form

The parser is **lossy** by design — multiple input strings collapse to one `KeyBind`. The serializer always emits the canonical form. Inputs and their canonical re-serializations:

| Input | Canonical re-serialization |
|---|---|
| `"Ctrl+K"` | `"Ctrl+K"` (uppercase preserved as part of `Char('K')`) |
| `"Ctrl+k"` | `"Ctrl+k"` (lowercase preserved) |
| `"Shift+r"` | `"R"` (Shift+lowercase folded to uppercase, SHIFT stripped) |
| `"R"` | `"R"` (already canonical) |
| `"shift+CTRL+k"` | `"Ctrl+Shift+k"` (modifier order canonicalized; case-folded) |
| `"Control+k"` | `"Ctrl+k"` (alias normalized) |
| `"Option+d"` | `"Alt+d"` (alias normalized) |
| `"Return"` | `"Enter"` (alias normalized) |
| `"Escape"` | `"Esc"` (alias normalized) |
| `"="` | `"+"` (after `Char('+')` → `Char('=')` normalize, displays as `+`) |

The round-trip property is: `parse(format(parse(s))) == parse(s)` — i.e. fixpoint after one round of normalization. The naive `parse(s) == parse(format(parse(s)))` already holds because `format` emits canonical and `parse` accepts canonical.

### Exceptions

- **`Shift+lowercase` collapses.** `"Shift+r".parse() == "R".parse()`. Both serialize as `"R"`. This is intentional (matches crossterm's event model where `Shift+r` arrives as `Char('R') + SHIFT`).
- **`+`/`=` collapse, post-refactor.** Per the design doc, the old `parse_keybind` collapse is dropped. `"="` parses to `Char('=')`; `"+"` parses to `Char('+')`. The normalizer in `KeyBind::new` still maps `Char('+')` → `Char('=')` so the two physical keys produce the same `KeyBind`. Round-trip: `parse("+")` → `Char('=')` → format → `"+"` (because `code_label` renders `Char('=')` as `+`) → reparse → `Char('=')`. Stable after one cycle. **If the framework chooses to drop the `Char('+') → Char('=')` normalization too**, the two keys become distinct and round-trip is exact. Pick at implementation time; both are coherent.
- **Modifier order.** `"Shift+Ctrl+k".parse() == "Ctrl+Shift+k".parse()`; both serialize as `"Ctrl+Shift+k"`. Same fixpoint argument.
- **Alias names.** `"Return"` / `"Control"` / `"Option"` / `"Escape"` / `"Del"` / `"Ins"` parse but never round-trip to themselves; they collapse to `"Enter"` / `"Ctrl"` / `"Alt"` / `"Esc"` / `"Delete"` / `"Insert"`.

The property test ignores aliases on the *input* side (since the formatter never produces them) and asserts fixpoint after one normalization round on the *output* side.
