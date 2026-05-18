# Themes plan

cargo-port currently bakes a single dark palette into `tui_pane/src/constants.rs`
and `src/tui/constants.rs` as bare `pub const FOO: Color`. This plan adds a
runtime-swappable theme system modeled on Zed's family-of-variants files, with
cross-platform OS appearance tracking so the app follows system light/dark
switches without restart.

## Phase overview

| Phase | What | Risk | Rough size |
|-------|------|------|------------|
| 1 | `Theme` type (grouped substructs of `StyleSpec` values) + `RwLock<Arc<Theme>>` static + replace every color constant and raw `Color::` literal with theme accessor reads; ship 2 compiled-in defaults as Rust constructors; toasts pinned to fallback palette | Medium | ~30 call sites updated, 2 default constructors, one commit |
| 2 | User-themes registry in tui_pane (Registry type + registration API); cargo-port-side bootstrap scans `~/.config/cargo-port/themes/*.toml` (sorted) and calls into the registry; polled per-tick fingerprint check on `themes/` drives hot reload | Low | New `theme/registry.rs` in tui_pane, scan + `ThemesWatch` + `maybe_reload_themes_from_disk` in cargo-port, one commit |
| 3 | `[appearance]` section in `config.toml` (mode + light_theme + dark_theme); `BackgroundMsg::AppearanceChanged` enum variant; resolve on startup and config reload; unknown names fall back to compiled-in defaults with a toast | Low | Config schema + apply path inside `apply_config`, one commit |
| 4 | Settings overlay UI: mode dropdown + two theme dropdowns sourced from the registry; surfaces "Theme not found" badge and registry load errors; writes back to config and re-applies live | Medium | Settings pane rows + edit handlers, one commit |
| 5 | OS appearance tracking via `dark-light` crate, polled in a background task with backoff; `mode = "auto"` resolves dynamically | Low | New background task, one commit |

Each phase lands as a single commit after `cargo build && cargo nextest run
&& cargo clippy && cargo mend && cargo +nightly fmt` all pass.

## Design points

### Active theme in a `RwLock<Arc<Theme>>` static

Render reads color constants per cell — thousands of reads per frame. A
`static THEME_STATE: OnceLock<ThemeState>` where `ThemeState` holds
`current: RwLock<Arc<Theme>>` matches the pattern already in use for
`ACTIVE_CONFIG: OnceLock<RwLock<CargoPortConfig>>` in `src/config.rs`.
Writes happen rarely (theme switch, config reload, OS appearance flip).
Per-read cost is sub-µs and unmeasurable against ratatui's per-cell work.

Reads inside a single frame must use a snapshot. Calling the global
`theme()` twice in one frame risks a mid-frame swap rendering some
cells with the old palette and others with the new. The pattern: the
main render loop takes one `Arc<Theme>` clone at the top of the frame
and passes it through `PaneRenderCtx`. Per-pane code reads from that
cloned `Arc`, never from the static.

### Single `ThemeState` over two statics

```rust
pub struct ThemeState {
    registry: RwLock<Arc<ThemeRegistry>>,
    current:  RwLock<Arc<Theme>>,
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();
```

One init point, one ownership story. The registry and the active theme
share an invariant ("the active theme name must exist in the registry,
or be a compiled-in default") that one struct enforces better than two
independently-managed `OnceLock`s.

Both slots are `RwLock<Arc<...>>`: readers (per-cell theme lookups,
settings UI iterating variants) take a read lock + Arc clone; writers
([`set_active_theme`] and [`replace_registry`]) take a write lock and
publish a new `Arc`. Phase 2's hot-reload replaces the whole registry
on disk-change, so the registry slot needs the same swappable
`RwLock<Arc<T>>` as `current`.

Both the registry and the active theme live in `tui_pane`. Theming is a
framework capability — `tui_pane` owns the `Theme` type, the
`ThemeRegistry` type, and the registration API. cargo-port owns the
*sources* of themes (which directory to scan, which config file points
at which theme); it calls into tui_pane to register variants and to set
the active one. Same pattern as keymap, where tui_pane owns the engine
and cargo-port owns the file location.

### Grouped `Theme` substructs of `StyleSpec` values

Themes own color *and* style modifiers (`Bold`, `Italic`, `Dim`,
`Underline`). Most call sites today pull modifiers from compiled-in
`Style::default().fg(COLOR).add_modifier(Modifier::BOLD)` chains;
themes absorb those, so call sites become `spec.style()`.

Fields are grouped into substructs that mirror how the rendering code
uses them:

```rust
pub struct Theme {
    pub pane_chrome: PaneChromeTheme,
    pub focus:       FocusTheme,
    pub semantic:    SemanticTheme,
    pub text:        TextTheme,
    pub git:         GitTheme,
    pub status:      StatusTheme,
    pub finder:      FinderTheme,
    pub disk_usage:  DiskUsageTheme,
}

pub struct PaneChromeTheme {
    pub active_border:   StyleSpec,
    pub inactive_border: StyleSpec,
    pub active_title:    StyleSpec,
    pub inactive_title:  StyleSpec,
}

pub struct FocusTheme {
    pub active:     StyleSpec,
    pub hover:      StyleSpec,
    pub remembered: StyleSpec,
}

pub struct SemanticTheme {
    pub accent:       StyleSpec,
    pub error:        StyleSpec,
    pub inline_error: StyleSpec,
    pub success:      StyleSpec,
    pub label:        StyleSpec,
}

pub struct TextTheme {
    pub default:   StyleSpec,   // formerly Color::White at e.g. render.rs:361
    pub secondary: StyleSpec,
    pub dim:       StyleSpec,   // formerly Color::DarkGray inline
    pub bright:    StyleSpec,   // formerly Color::Cyan inline
    pub bg_focus:  StyleSpec,   // formerly Color::Black inline
}

pub struct GitTheme {
    pub ignored:   StyleSpec,
    pub modified:  StyleSpec,
    pub untracked: StyleSpec,
}

pub struct StatusTheme {
    pub bar:           StyleSpec,
    pub target_bench:  StyleSpec,
    pub column_header: StyleSpec,
}

pub struct FinderTheme {
    pub match_bg:          StyleSpec,
    pub discovery_shimmer: StyleSpec,
}

pub struct DiskUsageTheme {
    /// Smallest disk-usage rows. Default dark: green.
    pub low:  StyleSpec,
    /// Mid-percentile rows. Default dark: white. Default light: a
    /// neutral gray so the gradient doesn't bottom out against the
    /// terminal background.
    pub mid:  StyleSpec,
    /// Largest disk-usage rows. Default dark: red.
    pub high: StyleSpec,
}
```

`DiskUsageTheme` exposes the three stops of the per-row disk-usage
gradient computed in `src/tui/panes/project_list.rs::disk_color`. The
interpolation math (green→white→red today, computed via `mul_add`
against the row's percentile) stays in code; only the three endpoint
colors come from the theme. This lets a user with red-green color
vision substitute blue→white→orange or yellow→white→red and keep the
"smaller is cooler, larger is warmer" reading without losing
information to a single hue dimension. Only `.color` is consumed by
the interpolator — modifiers on these specs are ignored for the
gradient itself.

Phase 1 audits both `tui_pane/` and `src/tui/` for raw `Color::White` /
`Color::Black` / `Color::DarkGray` / `Color::Cyan` / etc. literals and
routes each into the matching `Theme` substruct field. The audited
count (see "Phase 1 audit" section below) is 24 non-test literal
occurrences across 12 files: 17 `Color::White` text uses route to
`text.default`, and 7 toast-local constants move into a separate
`fallback_toast_palette()`. The disk-gradient stops hard-coded inside
`disk_color` (`Rgb(100,220,100)`, `Rgb(255,255,255)`,
`Rgb(255,100,100)` plus their interpolation deltas) move into
`DiskUsageTheme.{low,mid,high}.color`; the interpolation arithmetic
stays inline and now reads from `ctx.theme.disk_usage` instead of from
literals.

### `StyleSpec`

```rust
pub struct StyleSpec {
    pub color:     Color,
    pub modifiers: Modifiers,
}

#[derive(Default)]
pub struct Modifiers {
    pub bold:      bool,
    pub italic:    bool,
    pub dim:       bool,
    pub underline: bool,
}

impl StyleSpec {
    pub fn style(&self) -> Style {
        let mut m = ratatui::style::Modifier::empty();
        if self.modifiers.bold      { m |= ratatui::style::Modifier::BOLD; }
        if self.modifiers.italic    { m |= ratatui::style::Modifier::ITALIC; }
        if self.modifiers.dim       { m |= ratatui::style::Modifier::DIM; }
        if self.modifiers.underline { m |= ratatui::style::Modifier::UNDERLINED; }
        Style::default().fg(self.color).add_modifier(m)
    }

    pub const fn from_color(color: Color) -> Self {
        Self { color, modifiers: Modifiers::const_default() }
    }
}
```

Dynamic modifiers added at call sites (e.g., row-selection `REVERSED`)
combine with the theme's base via `theme_style.add_modifier(...)`.

### File format (Zed-style families)

One TOML file holds one *family* with one or more *variants*. Each
variant has a unique `name` and an `appearance` ("light" or "dark"),
followed by grouped color tables.

```toml
schema = 1
name = "Catppuccin"

[[variants]]
name = "Catppuccin Mocha"
appearance = "dark"

[variants.pane_chrome]
active_border   = "Yellow"
inactive_border = "DarkGray"
active_title    = { color = "Yellow", bold = true }
inactive_title  = "White"

[variants.focus]
active     = { r = 125, g = 125, b = 125 }
hover      = { r = 80,  g = 80,  b = 80  }
remembered = { r = 40,  g = 40,  b = 40  }

[variants.semantic]
accent       = "Cyan"
error        = "Red"
inline_error = "Yellow"
success      = "Green"
label        = { r = 150, g = 190, b = 180 }

[variants.git]
ignored   = "DarkGray"
modified  = { indexed = 208 }
untracked = "Yellow"

# ... etc

[[variants]]
name = "Catppuccin Latte"
appearance = "light"
# ...
```

### `StyleSpec` values in TOML

Three forms, all accepted by a custom `Deserialize` impl:

1. **Bare color string** — color only, no modifiers:
   ```toml
   active_border = "Yellow"
   ```

2. **Bare RGB / indexed table** — color only, no modifiers:
   ```toml
   accent = { r = 100, g = 200, b = 255 }
   git_modified = { indexed = 208 }
   ```

3. **Full spec table** — color plus any modifiers, all optional:
   ```toml
   active_title = { color = "Yellow", bold = true }
   secondary    = { color = { r = 180, g = 180, b = 180 }, italic = true, dim = true }
   ```

The custom deserializer recognizes all three and emits errors with
field name + offending value:

> `theme 'catppuccin.toml': field 'pane_chrome.active_border' has invalid color "MaroonChartreuse" — expected a named ratatui Color (e.g. "Yellow"), an { r, g, b } table, an { indexed = N } table, or a full spec like { color = ..., bold = true }`

Color values inside the `color = ...` field accept the same named /
RGB / indexed forms.

### Schema versioning

`schema = 1` at the top of every theme file declares the format version.
A typed `SchemaVersion` enum exists from day one with one variant (`V1`)
so future bumps require touching a match arm:

```rust
pub enum SchemaVersion { V1 }

pub const CURRENT_SCHEMA: SchemaVersion = SchemaVersion::V1;

fn migrate_schema(from: SchemaVersion, to: SchemaVersion, file: &mut toml::Value)
    -> Result<(), MigrationError>
{
    match (from, to) {
        (SchemaVersion::V1, SchemaVersion::V1) => Ok(()),
        // future bumps add arms here
    }
}
```

Behaviors per parsed `schema`:
- match current → load normally
- older known version → run `migrate_schema`, write back, then load
- newer than `CURRENT_SCHEMA` → reject, toast names the file
- absent → assume `V1`

### Registry, lookup, fallback

A `ThemeRegistry` (in tui_pane) holds:
- compiled-in built-in variants (always present, constructed in Rust)
- user variants registered by cargo-port from
  `~/.config/cargo-port/themes/*.toml` in sorted-filename order

Lookup is by typed `ThemeId(Arc<str>)`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThemeId(Arc<str>);

pub struct ThemeVariant {
    pub id:         ThemeId,
    pub appearance: Appearance,
    pub theme:      Theme,
}
```

User variants override built-ins with the same name — lets a user
replace `Default Dark` without losing the slot.

When `config.toml` references a name the registry doesn't have:
- log the miss
- fall back to the compiled-in default for that appearance
- emit a toast naming the missing theme
- record the miss in `RegistryStatus` so the settings UI can show a
  "Theme not found" badge on the affected dropdown

Malformed user files: skipped at scan time, toast names the file with
the parse error. Missing themes directory: silent skip (the registry
returns only built-ins). A later phase that offers "Save Theme" can
create the directory on demand.

### Config schema and theme names (Zed convention)

```toml
[appearance]
mode        = "auto"            # auto | light | dark
light_theme = "Default Light"
dark_theme  = "Default Dark"
```

Both theme name fields are plain strings (`ThemeId(Arc<str>)` typed at
the boundary), matching Zed's `"theme": { "light": "One Light", "dark":
"One Dark" }`. No `Custom(...)` wrapper, no enum split between built-ins
and user themes. A typo like `"Default Dakr"` fails at apply time with
a toast naming the missing id; the settings UI also shows a "Theme not
found" badge so the error stays visible past the toast's lifetime.

Internally:

```rust
pub enum Appearance { Light, Dark }

pub enum AppearanceMode {
    Auto,
    Pinned(Appearance),
}

impl AppearanceMode {
    pub fn resolve(self, os: Option<Appearance>) -> Appearance {
        match self {
            AppearanceMode::Pinned(a) => a,
            AppearanceMode::Auto      => os.unwrap_or(Appearance::Dark),
        }
    }
}
```

One enum (`Appearance`) for "what the OS reports / what a variant
targets," one enum (`AppearanceMode`) for "what the user picked."

### `dark-light` crate, polled with backoff

`dark_light::detect() -> Result<Mode>` covers macOS, Windows, Linux
(XDG Desktop Portal — works inside Flatpak), BSDs, and Wasm. v2.0.0,
~1.5M downloads, MIT/Apache.

No built-in subscription API in v2, so a background task polls every
1500ms. macOS and Windows are single-syscall checks; Linux is a D-Bus
round trip. Negligible.

After 10 consecutive errors (broken portal, sandbox restriction, etc.),
the task logs once at warn level and switches to a 30s interval to stop
hammering a broken interface. The poll never gives up entirely — if the
interface comes back, detection resumes at the slow cadence.

When `detect()` returns a different `Mode` than last poll, the task
sends `BackgroundMsg::AppearanceChanged(Appearance)` through the existing
channel. The main loop handler resolves `light_theme` / `dark_theme`
from config and swaps the active theme. Next frame uses it.

### Reload ordering

When `config.toml` or a file under `themes/` changes, reload in this
order:

1. theme registry (re-scan `themes/` if a theme file changed)
2. active theme (re-resolve config name against registry)
3. keymap (renders use current theme colors for highlight)

Doing theme first prevents keymap rendering from briefly using a stale
palette.

## Phase 1 audit (2026-05-17)

Concrete inventory captured before any code lands so the constructors
match shipped behavior exactly and the Phase 1 commit has no surprises.

### Constants inventory (21 colors across 2 files)

`tui_pane/src/constants.rs` — 18 `pub const FOO: Color` items:

| Const | Value | Role | → Theme field |
|-------|-------|------|---------------|
| `ACCENT_COLOR` | `Cyan` | spinners, shortcut hints, finder cursor | `semantic.accent` |
| `ACTIVE_BORDER_COLOR` | `Yellow` | focused pane border | `pane_chrome.active_border` |
| `ACTIVE_FOCUS_COLOR` | `Rgb(125,125,125)` | focused row background | `focus.active` |
| `HOVER_FOCUS_COLOR` | `Rgb(80,80,80)` | hovered row background | `focus.hover` |
| `COLUMN_HEADER_COLOR` | `Rgb(150,190,180)` | project list column headers | `status.column_header` (+ Bold modifier) |
| `DISCOVERY_SHIMMER_COLOR` | `Rgb(150,210,255)` | new-project shimmer | `finder.discovery_shimmer` |
| `ERROR_COLOR` | `Red` | error text, broken-worktree backgrounds | `semantic.error` |
| `INLINE_ERROR_COLOR` | `Yellow` | inline error on selected settings row | `semantic.inline_error` |
| `INACTIVE_BORDER_COLOR` | `DarkGray` | unfocused pane border | `pane_chrome.inactive_border` |
| `INACTIVE_TITLE_COLOR` | `White` | unfocused pane title | `pane_chrome.inactive_title` |
| `LABEL_COLOR` | aliases `COLUMN_HEADER_COLOR` | labels, countdowns, hints, chevrons | `semantic.label` |
| `REMEMBERED_FOCUS_COLOR` | `Rgb(40,40,40)` | last-focused row background | `focus.remembered` |
| `SECONDARY_TEXT_COLOR` | `Gray` | dim secondary text | `text.secondary` |
| `STATUS_BAR_COLOR` | `DarkGray` | bottom status bar background | `status.bar` |
| `SUCCESS_COLOR` | `Green` | clean/passed/synced | `semantic.success` |
| `TARGET_BENCH_COLOR` | `Magenta` | bench target type | `status.target_bench` |
| `TITLE_COLOR` | `Yellow` | active titles, headers, stat numbers | `pane_chrome.active_title.color` |
| `FINDER_MATCH_BG` | `Rgb(0,90,100)` | fuzzy match background tint | `finder.match_bg` |

`src/tui/constants.rs` — 3 `pub(super) const` items:

| Const | Value | Role | → Theme field |
|-------|-------|------|---------------|
| `GIT_MODIFIED_COLOR` | `Indexed(208)` | git modified marker | `git.modified` |
| `GIT_UNTRACKED_COLOR` | `Green` | git untracked marker | `git.untracked` |
| `GIT_IGNORED_COLOR` | `DarkGray` | git ignored marker | `git.ignored` |

Notes:

- `LABEL_COLOR` aliases `COLUMN_HEADER_COLOR` (same Rgb). The theme
  split keeps them as separate fields (`semantic.label` and
  `status.column_header`); both default to `Rgb(150,190,180)` so the
  migration is behavior-preserving, but a user theme can later
  override them independently.
- `TITLE_COLOR`'s doc comment lists six roles ("active pane titles,
  section headers, group header labels, stat numbers, confirm dialog
  prompts, popup titles, summary row"). Phase 1 routes every call
  site to `pane_chrome.active_title.color`; later phases can split
  these if a user wants stat numbers in a different color from pane
  titles.
- `status.column_header` defaults to Bold because every current call
  site adds `Modifier::BOLD` at the use point (e.g. `lang.rs:75`,
  `lang.rs:91`). Absorbing it into the theme default lets call sites
  drop the explicit modifier.

### Raw `Color::` literal inventory (24 occurrences across 12 files)

Non-test only. Test code retains its inline `Color::Red` / `Color::Blue`
literals — those are pattern-match fixtures, not user-visible palette.

#### Toast-local consts (7 — move into fallback toast palette)

`tui_pane/src/toasts/render/mod.rs` declares its own private copies of
six constants from the framework palette (so toasts stay readable even
if the active theme is corrupt), plus `card.rs:55` uses an inline
`Color::White` as the accent for non-error/non-warning toasts. All seven
move into the new `fallback_toast_palette()` (see next subsection):

| File:line | Value | Fallback palette field |
|-----------|-------|------------------------|
| `tui_pane/src/toasts/render/mod.rs:21` | `Cyan` | `accent` |
| `tui_pane/src/toasts/render/mod.rs:22` | `Yellow` | `border_focused` |
| `tui_pane/src/toasts/render/mod.rs:23` | `Red` | `error` |
| `tui_pane/src/toasts/render/mod.rs:24` | `Rgb(150,190,180)` | `label` |
| `tui_pane/src/toasts/render/mod.rs:25` | `Yellow` | `title` |
| `tui_pane/src/toasts/render/mod.rs:26` | `Yellow` | `warning` |
| `tui_pane/src/toasts/render/card.rs:55` | `White` | `plain_accent` |

#### `Color::White` text uses (17 — route to `text.default`)

`Color::White` is the universal "regular foreground" today. Every
non-test occurrence routes to `text.default`:

| File:line | Surrounding bg | Notes |
|-----------|----------------|-------|
| `src/tui/keymap_ui/view.rs:103` | (default) | keymap entry default text |
| `src/tui/keymap_ui/view.rs:111` | (default) | keymap entry default text |
| `src/tui/keymap_ui/view.rs:128` | (default) | keymap entry default text |
| `src/tui/columns/mod.rs:445` | `ERROR_COLOR` | broken worktree style |
| `src/tui/columns/mod.rs:530` | (default) | column text |
| `src/tui/panes/project_list.rs:621` | `ERROR_COLOR` | broken worktree row |
| `src/tui/render.rs:361` | (default) | finder prompt text |
| `src/tui/render.rs:559` | `STATUS_BAR_COLOR` | status line |
| `src/tui/render.rs:564` | (default) | status value |
| `src/tui/panes/git.rs:385` | `ERROR_COLOR` | broken git pane |
| `src/tui/panes/cpu.rs:116` | (default) | cpu label text |
| `src/tui/panes/cpu.rs:136` | (default) | cpu value text |
| `src/tui/panes/cpu.rs:450` | (default) | cpu chart color |
| `src/tui/finder/dispatch.rs:443` | (default) | finder display_name |
| `src/tui/finder/dispatch.rs:444` | (default) | finder parent |
| `src/tui/finder/dispatch.rs:445` | (default) | finder branch |
| `src/tui/finder/dispatch.rs:446` | (default) | finder dir |

Plus four overlay-test inline literals in `tui_pane/src/overlays/settings.rs:810-818` and three in `tui_pane/src/pane/state.rs:152-174` and `src/tui/app/tests/panes.rs:594,612` — all `#[cfg(test)]` and left in place.

### Fallback toast palette (7 fields, not 4)

The earlier plan estimated four ("border, title, body, accent"); the
real field count from reading `card.rs` and the toast-local `mod.rs`
is seven. `format.rs::fade_to_style` computes `Rgb(v,v,v)` at runtime
for the lifetime-fade animation — it's not a palette color and stays
inline.

```rust
pub struct FallbackToastPalette {
    /// Spinner color in tracked-item rows.
    pub accent:          Color,
    /// Border color when the toast is focused.
    pub border_focused:  Color,
    /// Border + text color for error toasts.
    pub error:           Color,
    /// Border + text color for warning toasts.
    pub warning:         Color,
    /// Countdown text, italic action hint, overflow rows.
    pub label:           Color,
    /// Running tracked-item duration suffix.
    pub title:           Color,
    /// Accent for non-error/non-warning toasts.
    pub plain_accent:    Color,
}

pub fn fallback_toast_palette() -> FallbackToastPalette {
    FallbackToastPalette {
        accent:         Color::Cyan,
        border_focused: Color::Yellow,
        error:          Color::Red,
        warning:        Color::Yellow,
        label:          Color::Rgb(150, 190, 180),
        title:          Color::Yellow,
        plain_accent:   Color::White,
    }
}
```

A toast-fallback test asserts the seven fields exactly equal the values
above, so a future refactor can't silently route toast rendering through
the active theme.

### `text.default` does not exist in today's constants

The current code uses `Color::White` inline at 17 sites instead of a
named constant. Phase 1 adds `text.default` as a new theme field (no
constant to delete) and migrates the 17 sites in the same commit.

### Derives required on `Theme` and substructs

The plan calls for a roundtrip test (TOML starter ↔ Rust constructor).
That requires:

- `#[derive(Clone, Debug, PartialEq, Eq)]` on `Theme`, every substruct,
  `StyleSpec`, and `Modifiers`.
- `#[derive(Serialize)]` (custom or derived) on `StyleSpec`,
  `Modifiers`, and the substructs — only consumed by the test, not by
  runtime save-theme code (that's a deferred follow-up).

Both are mechanical additions called out here so they don't surface as
review surprises during Phase 1.

## Phase 1 — `Theme` type, static, constant migration

### New types

Defined in `tui_pane/src/theme/`:

- `mod.rs` — `Theme`, the substruct types, the `ThemeId` newtype, the
  `Appearance` enum, the `theme()` accessor and `set_active_theme()`
  mutator.
- `spec.rs` — `StyleSpec`, `Modifiers`, the custom `Deserialize` impls,
  the color form parser.
- `builtins.rs` — `pub fn default_dark() -> Theme { ... }` and
  `default_light() -> Theme { ... }`, both written as Rust struct
  literals.

### Compiled-in defaults are Rust constructors

```rust
pub fn default_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   StyleSpec::from_color(Color::Yellow),
            inactive_border: StyleSpec::from_color(Color::DarkGray),
            active_title: StyleSpec {
                color:     Color::Yellow,
                modifiers: Modifiers { bold: true, ..Modifiers::default() },
            },
            inactive_title:  StyleSpec::from_color(Color::White),
        },
        focus: FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(125, 125, 125)),
            hover:      StyleSpec::from_color(Color::Rgb(80, 80, 80)),
            remembered: StyleSpec::from_color(Color::Rgb(40, 40, 40)),
        },
        // ... etc
    }
}
```

Built-ins are compile-time-checked. A typo is a build error. No TOML
parsing happens for defaults — they're available the instant the
binary starts.

### Default light palette

Inverting the dark constants point-for-point fails: `Color::Yellow` text
is unreadable on a white terminal, `Color::Cyan` washes out, raw
`Color::DarkGray` lands too close to the focus-highlight ramp instead
of contrasting against it. The light constructor picks each field for
legibility on a white background, not by mechanical inversion. The
mapping below is the starting point — adjust during Phase 1 if any
field reads poorly in a manual launch.

| Group / field                  | Dark (existing)        | Light (proposed)              | Why                                                  |
|--------------------------------|------------------------|-------------------------------|------------------------------------------------------|
| pane_chrome.active_border      | `Yellow`               | `Rgb(180, 120, 0)` (amber)    | Yellow vanishes on white; amber keeps the hue        |
| pane_chrome.inactive_border    | `DarkGray`             | `Gray`                        | Lighter so it recedes against white                  |
| pane_chrome.active_title       | `Yellow` + Bold        | `Rgb(160, 100, 0)` + Bold     | Same hue family, darker for contrast                 |
| pane_chrome.inactive_title     | `White`                | `Black`                       | Direct inversion                                     |
| focus.active                   | `Rgb(125, 125, 125)`   | `Rgb(200, 200, 200)`          | Subtle highlight that reads as "lighter" on white    |
| focus.hover                    | `Rgb(80, 80, 80)`      | `Rgb(220, 220, 220)`          | Step lighter than active                             |
| focus.remembered               | `Rgb(40, 40, 40)`      | `Rgb(235, 235, 235)`          | Step lighter than hover                              |
| semantic.accent                | `Cyan`                 | `Rgb(0, 95, 135)` (steel)     | Bright cyan vanishes; darker blue holds              |
| semantic.error                 | `Red`                  | `Rgb(170, 0, 0)`              | Standard red is fine; slightly darker for white bg   |
| semantic.inline_error          | `Yellow`               | `Rgb(180, 95, 0)` (orange)    | Yellow unreadable; orange keeps the warning hue      |
| semantic.success               | `Green`                | `Rgb(0, 120, 0)`              | Darker green for white bg                            |
| semantic.label                 | `Rgb(150, 190, 180)`   | `Rgb(60, 100, 90)`            | Darker complement of the dark teal-gray              |
| text.default                   | `White`                | `Black`                       | Direct inversion                                     |
| text.secondary                 | `Gray`                 | `Rgb(70, 70, 70)`             | One step lighter than text.default                   |
| text.dim                       | `DarkGray`             | `Rgb(130, 130, 130)`          | Lighter than secondary, still readable               |
| text.bright                    | `Cyan`                 | `Rgb(0, 95, 135)`             | Match semantic.accent                                |
| text.bg_focus                  | `Black`                | `White`                       | Direct inversion                                     |
| git.ignored                    | `DarkGray`             | `Rgb(150, 150, 150)`          | Same role: faded                                     |
| git.modified                   | `Indexed(208)` orange  | `Indexed(208)` orange         | Indexed 208 reads on both backgrounds                |
| git.untracked                  | `Green`                | `Rgb(0, 120, 0)`              | Match semantic.success on white                      |
| status.bar                     | `DarkGray`             | `Rgb(220, 220, 220)`          | Light bar bg; preserves the "bottom strip" affordance |
| status.target_bench            | `Magenta`              | `Rgb(140, 0, 140)`            | Darker magenta for contrast on white                 |
| status.column_header           | `Rgb(150,190,180)` + Bold | `Rgb(60, 100, 90)` + Bold  | Inverted darkness of the dark teal-gray              |
| finder.match_bg                | `Rgb(0, 90, 100)`      | `Rgb(255, 245, 180)` (cream)  | Pale yellow highlight reads as "matched" on white    |
| finder.discovery_shimmer       | `Rgb(150, 210, 255)`   | `Rgb(120, 140, 200)`          | Darker blue keeps the "new" hue affordance on white  |
| disk_usage.low                 | `Rgb(100, 220, 100)`   | `Rgb(0, 140, 0)`              | Darker green; bright green washes out on white       |
| disk_usage.mid                 | `Rgb(255, 255, 255)`   | `Rgb(90, 90, 90)`             | White mid-stop invisible on white bg; use dark gray  |
| disk_usage.high                | `Rgb(255, 100, 100)`   | `Rgb(200, 0, 0)`              | Darker red for contrast on white                     |

All "Dark (existing)" values in this table come from the Phase 1 audit
section below; "Light (proposed)" picks are starting points and may be
adjusted during the Phase 1 manual launch if any field reads poorly.

### Starter-template TOML files (not loaded at runtime)

`tui_pane/themes/default_dark.toml` and `default_light.toml` exist in
the repo as *templates* the user can copy into their own
`~/.config/cargo-port/themes/` directory as a starting point for
customization. These files are **not** parsed by the app at startup —
they're documentation, mirroring the Rust constructors. A
`#[test] fn templates_match_builtin_constructors` in
`tui_pane/tests/themes.rs` parses each template and asserts it
round-trips with the corresponding Rust constructor, catching drift
between docs and reality.

### State and accessors

```rust
pub struct ThemeState {
    registry: RwLock<Arc<ThemeRegistry>>,
    current:  RwLock<Arc<Theme>>,
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();

pub fn install_theme_state(state: ThemeState) {
    // Idempotent: a second call is a silent no-op so test binaries
    // that re-run startup don't panic. Production startup updates
    // an installed state via `replace_registry` / `set_active_theme`.
    let _ = THEME_STATE.set(state);
}

pub fn set_active_theme(theme: Arc<Theme>) {
    let state = THEME_STATE.get_or_init(/* default-dark + builtins */);
    *state.current.write().expect("theme RwLock poisoned") = theme;
}

pub fn replace_registry(new_registry: ThemeRegistry) {
    let state = THEME_STATE.get_or_init(/* default-dark + builtins */);
    *state.registry.write().expect("registry RwLock poisoned") = Arc::new(new_registry);
}

pub fn theme() -> Arc<Theme> {
    let state = THEME_STATE.get_or_init(/* default-dark + builtins */);
    state.current.read().expect("theme RwLock poisoned").clone()
}

pub fn registry() -> Arc<ThemeRegistry> {
    let state = THEME_STATE.get_or_init(/* default-dark + builtins */);
    state.registry.read().expect("registry RwLock poisoned").clone()
}
```

Main startup calls `install_theme_state` with a registry built from
the user themes directory. The accessors lazy-init a built-ins-only
state on first call if no one has installed yet — keeps tests that
exercise render code (without going through full app startup) from
panicking, with the same default value the explicit install would
have used.

`registry()` returns an `Arc<ThemeRegistry>` snapshot rather than a
`&'static`; the registry is replaced wholesale by hot-reload, so a
`'static` borrow couldn't survive a swap. The Arc-clone cost is the
same as `theme()`.

In Phase 1 (before Phase 2's registry exists), `install_theme_state`
was called via `ThemeState::new(default_dark())` which seeds the
built-ins-only registry automatically. Phase 2 callers use
`ThemeState::with_registry(registry, default_dark())`.

### Per-frame snapshot

The render loop in `src/tui/render.rs::ui` takes one `Arc<Theme>` at
the top of the frame:

```rust
let frame_theme = theme::theme();
// ... build PaneRenderCtx with frame_theme
```

`PaneRenderCtx` gains a `theme: &'a Theme` field. Every per-pane render
reads from `ctx.theme.pane_chrome.active_border.style()` (or whatever
field) instead of calling the global. This eliminates mid-frame swap
tearing.

### Toast pinned to fallback palette

`tui_pane/src/toasts/render/card.rs` reads from a compiled-in
`fallback_toast_palette() -> ToastColors` instead of the active theme.
A broken user theme can never make its own error toast unreadable. The
fallback is a fixed 4-color subset (border, title, body, accent) chosen
to be legible on both light and dark terminals.

### Call-site migration

Every `INACTIVE_BORDER_COLOR` style reference becomes
`ctx.theme.pane_chrome.inactive_border.style()` (or
`.color` if only the color is needed). Raw `Color::White`/`Color::Black`
/ etc. literals route to `ctx.theme.text.default` / `ctx.theme.text.bg_focus`
/ etc. LSP `findReferences` plus `rg 'Color::(White|Black|Gray|Cyan)'`
together enumerate the call sites. The bare `pub const` color items in
`tui_pane/src/constants.rs` and `src/tui/constants.rs` are deleted (the
non-color constants like `BLOCK_BORDER_WIDTH` stay).

### Verification

`cargo nextest run` plus a manual launch: identical visual output. The
template-roundtrip test asserts each TOML starter file matches the
corresponding Rust constructor. A toast-fallback test asserts that
toast styles never read from the active theme.

## Phase 2 — User-themes registry

### Path

`dirs::config_dir().join("cargo-port").join("themes")` — same parent as
`config.toml` and `keymap.toml`. macOS:
`~/Library/Application Support/cargo-port/themes/`. Linux:
`~/.config/cargo-port/themes/`. Missing directory → registry returns
only built-ins (no error, no toast).

### Registry API (tui_pane)

```rust
pub struct ThemeRegistry {
    variants: Vec<ThemeVariant>,
    status:   RegistryStatus,
}

impl ThemeRegistry {
    pub fn empty() -> Self;
    pub fn new_with_builtins() -> Self;
    pub fn register(&mut self, variant: ThemeVariant) -> RegisterOutcome;
    pub fn record_failed_file(&mut self, path: PathBuf, error: ThemeLoadError);
    pub fn find(&self, id: &ThemeId) -> Option<&ThemeVariant>;
    pub fn variants_by_appearance(&self, appearance: Appearance)
        -> impl Iterator<Item = &ThemeVariant>;
    pub fn all(&self) -> impl Iterator<Item = &ThemeVariant>;
    pub const fn status(&self) -> &RegistryStatus;
    pub const fn len(&self) -> usize;
    pub const fn is_empty(&self) -> bool;
}

pub const BUILTIN_DARK_NAME:  &str = "Default Dark";
pub const BUILTIN_LIGHT_NAME: &str = "Default Light";

pub enum RegisterOutcome {
    Inserted,
    Overrode(ThemeId),    // overrode an existing variant with this id
}

pub struct RegistryStatus {
    pub failed_files: Vec<(PathBuf, ThemeLoadError)>,
    pub overridden:   Vec<ThemeId>,
}
```

`new_with_builtins()` seeds the two compiled-in variants under the
stable ids `BUILTIN_DARK_NAME` / `BUILTIN_LIGHT_NAME`. User variants
with the same name replace them in place (preserving registry
ordering) and the override is recorded in `RegistryStatus.overridden`.

### Scan (cargo-port)

cargo-port owns the scan code because the path layout is app-specific.
The implementation lives in `src/themes/mod.rs`. At startup, inside
`AppBuilder::run_startup` (after `config::set_active_config`, before
the rest of the startup pipeline):

1. `themes::build_user_registry(themes::themes_dir().as_deref())`:
   1a. seed with `ThemeRegistry::new_with_builtins()`,
   1b. `read_dir` on the themes directory, sort entries by filename
       ASCII order (sorted iteration is what makes the "later file
       overrides earlier" tie-break deterministic across runs),
   1c. for each `*.toml`: parse as `ThemeFamily`; parse errors record
       into `RegistryStatus.failed_files` and continue,
   1d. for each variant in each parsed family: call `registry.register(...)`.
       Each `Overrode` outcome appends to `RegistryStatus.overridden`.
2. `tui_pane::install_theme_state(ThemeState::with_registry(registry, default_dark()))`.

Startup parse-error / override toasts are not emitted in Phase 2 — the
registry carries the diagnostics in `RegistryStatus`, and the Phase 4
settings UI will surface them. The hot-reload path below does emit
toasts because the user just edited a file.

### Hot-reload

Per-tick polling, not notify subscription. `ThemesWatch` keeps a
fingerprint hashed from `(filename, mtime, len)` of every `*.toml` in
the directory. `App::maybe_reload_themes_from_disk` runs each tick
from `terminal.rs` alongside `maybe_reload_config_from_disk` and
`maybe_reload_keymap_from_disk`. On a fingerprint change:

1. Re-scan the themes directory via `themes::build_user_registry`.
2. Snapshot `failed_files` + `overridden` + `len` off the new registry.
3. Replace the global registry via `tui_pane::replace_registry`
   (one write lock).
4. Dismiss any prior persistent error-toast (the
   `themes.diagnostics_id` slot, mirroring `keymap.diagnostics_id`).
5. If `failed_files` is empty: emit a timed "Themes reloaded" toast
   summarizing variant count + override list. Otherwise push a
   persistent "Themes reload errors" toast and record its id in
   `themes.diagnostics_id` so the next clean reload dismisses it.

Re-resolving the active theme name from config (step 3 in the
original plan) lands in Phase 3 when config gets an `[appearance]`
section. Phase 2 leaves the active theme at `default_dark()`
throughout.

Notify-based watching was considered. Polling is simpler (no new
`BackgroundMsg` variant, no thread, no event coalescing), shares the
per-tick cadence with config and keymap reload, and the per-tick cost
is one `read_dir` + a `stat` per `*.toml` (typically zero files).

### Custom `StyleSpec` deserializer

Hand-rolled `Deserialize` for `StyleSpec` shipped in Phase 1 already
(see `tui_pane/src/theme/spec.rs`) so the same code parses both
in-repo starter templates and Phase 2's user files. Recognizes the
three forms covered in the "File format" design point above; emits
field-name + offending-value errors instead of serde's default
`unknown variant '...'`.

### No UI yet

A user can edit `config.toml` directly to test. The dropdown UI arrives
in Phase 4.

### Phase 2 retrospective

Built and shipped 2026-05-18.

- New file layout: `tui_pane/src/theme/registry.rs` for the registry
  types; `src/themes/mod.rs` for scan + `ThemesWatch`;
  `src/tui/state/themes.rs` for the App-side `Themes` subsystem
  wrapping the watch + diagnostics-toast id.
- `install_theme_state` was changed from "panic on re-install" to
  "silent no-op on re-install" so test binaries that exercise startup
  multiple times don't panic. Production startup hits it once.
- `registry()` returns `Arc<ThemeRegistry>` (snapshot), not the plan's
  literal `&'static ThemeRegistry`. The literal can't survive a
  `replace_registry` swap; the Arc snapshot mirrors `theme()` and is
  what Phase 4's settings UI will want anyway.
- No `BackgroundMsg::ThemesChanged` variant. The watch is polled
  inline per tick — see "Hot-reload" above.
- Per-tick scan stays synchronous in `run_startup` (the disk walk is
  `read_dir` + a stat per `*.toml`, typically zero files). If a real
  user later ships dozens of themes and this shows up in the startup
  perf log, route through the tokio blocking pool with a new
  `BackgroundMsg::ThemesScanned` variant.
- 8 unit tests in `src/themes/mod.rs`, 2 in `src/tui/state/themes.rs`,
  3 registry tests in `tui_pane/tests/themes.rs`. 913 workspace tests
  pass; clippy `-D warnings` clean; mend clean; binary reinstalled.

## Phase 3 — Config schema and apply

### Schema additions to `Config`

A new `[appearance]` section nested in `CargoPortConfig`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, confique::Config, Serialize)]
pub(crate) struct AppearanceConfig {
    #[config(default = "dark")]
    pub mode:        String,
    #[config(default = "Default Light")]
    pub light_theme: String,
    #[config(default = "Default Dark")]
    pub dark_theme:  String,
}
```

All three fields are `String` rather than typed enums / `ThemeId` so
confique's `Layer` Deserialize stays on primitive types. Parsing
happens at apply time inside `themes::resolve_theme`, where a typo in
`mode` surfaces as a timed toast (without rejecting the rest of the
config) and an unknown theme name surfaces as a persistent "Theme not
found" toast plus a built-in fallback.

Defaults when the section is absent: `mode = "dark"`, `light_theme =
"Default Light"`, `dark_theme = "Default Dark"`.

### `AppearanceMode` (cargo-port side)

`AppearanceMode` lives in `src/themes/mod.rs` (not `tui_pane`) because
it pairs a config-level concept (`Auto` vs explicit) with `tui_pane`'s
[`Appearance`]:

```rust
pub(crate) enum AppearanceMode {
    Auto,
    Pinned(Appearance),
}

impl AppearanceMode {
    pub(crate) fn parse(raw: &str) -> Result<Self, String>;
    pub(crate) const fn resolve(self, os: Option<Appearance>) -> Appearance;
}
```

`parse` is case-insensitive and accepts `"auto"`, `"light"`, `"dark"`;
anything else returns an `Err(message)` the caller can toast. `resolve`
returns `Pinned` directly and falls back from `Auto` to `Appearance::Dark`
when no OS signal is available — the same default any other terminal
app would pick.

### Theme resolution helper

`themes::resolve_theme(appearance_cfg, registry, os_appearance) ->
ResolvedTheme` does the work shared by startup and apply-time:

```rust
pub(crate) struct ResolvedTheme {
    pub theme:      Arc<Theme>,
    pub miss:       Option<ThemeId>,    // configured id absent from registry
    pub mode_error: Option<String>,     // mode failed to parse
}
```

The two `Option` slots let the caller decide when to surface
diagnostics — startup ignores both, the apply path toasts both.

### `BackgroundMsg` variant

`BackgroundMsg` (in `src/scan/mod.rs`) gains:

```rust
#[expect(dead_code, reason = "Phase 5 dark-light poller is the sole producer")]
AppearanceChanged(Appearance),
```

The variant ships in Phase 3 so the receiver and theme-apply path can
be wired ahead of Phase 5's `dark-light` poller. Until that poller
lands, the variant is never constructed; the `expect` keeps the strict
clippy gate satisfied and self-removes when Phase 5's producer makes
it live.

### Apply

`App::resolve_and_apply_active_theme` (in
`src/tui/app/async_tasks/config.rs`) is the one place the resolution
runs after startup:

1. Snapshot `tui_pane::registry()` (an `Arc<ThemeRegistry>`, cheap to clone).
2. Call `themes::resolve_theme(&self.config.current().appearance, &registry,
   self.themes.os_appearance())`.
3. Publish via `tui_pane::set_active_theme(resolved.theme)`.
4. Dismiss the prior `themes.miss_toast_id` if any. If `resolved.miss` is
   `Some(id)`, push a persistent "Theme not found" toast and record its id
   in `themes.miss_toast_id` so the next clean resolve dismisses it
   (mirrors the keymap diagnostics pattern).
5. If `resolved.mode_error` is `Some`, surface a timed "Appearance mode" toast.

Two callers invoke it:

- `apply_config` calls it when `self.config.current().appearance != cfg.appearance`. Other config
  changes (CPU poll, lint flags, etc.) skip the call so no toast fires on unrelated reloads.
- `dispatch::handle_appearance_changed` (the `BackgroundMsg::AppearanceChanged` arm) updates
  `themes.os_appearance` and calls it. The arm is extracted into its own method only because the
  match would otherwise push `handle_bg_msg` past clippy's 100-line gate; the logic is two lines.

Startup uses the same `themes::resolve_theme` helper inside
`AppBuilder::run_startup` to pick the initial `ThemeState::with_registry`
theme. Misses are silent at startup (no toast machinery yet — Phase 4's
settings UI surfaces them via a "Not found" badge).

Config file watcher reloads invoke `apply_config`, so the same path
fires on `~/.config/cargo-port/config.toml` edits. Reload order is
documented above (theme → keymap).

`mode = "auto"` in Phase 3 behaves identically to `mode = "dark"`
because `os_appearance` is always `None` until Phase 5 plugs in the
`dark-light` poller. Phase 5 will start sending `AppearanceChanged`,
which the dispatch handler routes through `resolve_and_apply_active_theme`
— no further apply-path changes needed.

### Phase 3 retrospective

Built and shipped 2026-05-18.

- `AppearanceConfig` fields are `String` rather than the plan's typed
  `AppearanceMode` / `ThemeId`. confique's `Layer` Deserialize wants
  concrete primitive types, and "parse + toast at apply time" beats
  "config load fails on a typo" for user-edited files. The typed
  forms still exist — `AppearanceMode::parse` and `ThemeId::new` —
  but they run inside `themes::resolve_theme`, not at deserialization.
- `AppearanceMode` lives in `src/themes/mod.rs` (cargo-port), not
  `tui_pane`. It's a config-level concept that wraps tui_pane's
  `Appearance`; tui_pane has no reason to know about it.
- `ResolvedTheme` collects the three outputs (`theme`, `miss`,
  `mode_error`) so startup and apply-time can share the same helper.
  Earlier drafts returned a bare `Arc<Theme>` and routed diagnostics
  through out-parameters; the struct reads cleaner.
- `Themes` subsystem gained two slots: `miss_toast_id` (mirrors
  `keymap.diagnostics_id`) and `os_appearance` (the
  `BackgroundMsg::AppearanceChanged` receiver writes here).
- `apply_config` gates the resolve call on
  `self.config.current().appearance != cfg.appearance` so reloads that
  only change unrelated fields (e.g. CPU poll) don't redundantly swap
  the theme or fire spurious toasts.
- The `BackgroundMsg::AppearanceChanged` dispatch arm is extracted into
  `handle_appearance_changed` to keep `handle_bg_msg` under clippy's
  100-line `too_many_lines` gate. The extracted function is two lines.
- `tests/assets/default-config.toml` updated to include the new
  `[appearance]` section so the template golden-file test stays
  current.
- 921 workspace tests pass; clippy `-D warnings` clean; mend clean;
  binary reinstalled via `cargo install --path .`.

## Phase 4 — Settings overlay UI

Three rows in the existing settings pane, all under a new `Appearance`
section:

| Row         | Widget kind | Values |
|-------------|-------------|--------|
| Mode        | Stepper     | auto / light / dark (Left/Right/Enter cycle) |
| Light theme | Stepper     | every variant where `appearance == Light`, cycled in registry order |
| Dark theme  | Stepper     | every variant where `appearance == Dark`, cycled in registry order |

The framework's existing `SettingsRowKind::Stepper` is the closest fit
to a dropdown without adding a new widget kind: Left/Right adjust the
value (cycling), Enter activates (cycles forward by one). Each cycle
writes back to `config.toml`, then `apply_config` sees `appearance`
changed and re-runs `resolve_and_apply_active_theme` (Phase 3). No
restart.

When `config.appearance.light_theme` / `dark_theme` doesn't exist in
the registry (typo or older config inherited from before a theme was
removed), the row gets a `Not found` suffix. Cycling moves to a valid
registry entry; the stale value isn't preserved across the cycle (the
config still reflects it until the user advances).

When `RegistryStatus.failed_files` is non-empty, the section header
absorbs the banner — its label becomes
`Appearance — N theme file(s) failed to load (see logs)`. The framework
section row doesn't support suffixes, and the section label is the
natural place for a once-per-render banner.

### Phase 4 retrospective (2026-05-18)

Deviations from the plan, all driven by surface already in the
framework:

- **Stepper instead of dropdown.** `tui_pane`'s settings widgets are
  Section / Value / Toggle / Stepper. Adding a true dropdown kind
  would have meant changing tui_pane's row enum and its renderer for
  three rows; Stepper covers the same UX (cycle through values) without
  the boilerplate.
- **Banner on the section label, not above it.** Section rows render
  through a separate path that doesn't take a `suffix`. The simplest
  surface is to fold the count into the section title.
- **`SettingsUiRow` label widened from `&'static str` to `String`.**
  Required so the dynamic banner can land in the section label slot.
  Mechanical change across the four existing row builders.
- **Bonus refactor: `is_stepper_setting` helper + extracted
  `apply_cpu_settings_edit`.** Pulling appearance rows through Stepper
  exposed a clippy `if_same_then_else` in `framework_settings_rows`
  (the existing CiRunCount stepper and the new appearance steppers
  were identical branches). The CPU extraction was a side-effect of
  re-shrinking `apply_general_settings_edit` below the
  `too_many_lines` 100-line gate after adding three appearance arms.

Stays on plan:

- Cycling writes back to `config.toml` and runs through Phase 3's
  `apply_config` → `resolve_and_apply_active_theme` path.
- "Not found" badge on `Light theme` / `Dark theme` rows when the
  current value isn't in the live registry.
- `[appearance]` is persisted on first run via the existing
  `settings_table_from_config` seed path.
- 921 workspace tests pass; clippy `-D warnings` clean; mend clean.

## Phase 5 — OS appearance tracking

### Dependency

```toml
[workspace.dependencies]
dark-light = "2"
```

Pulled in by the main `cargo-port` binary. No `tui_pane` dep — the
appearance enum crosses the boundary via `BackgroundMsg::AppearanceChanged`.

### Background task

```rust
async fn appearance_poller(tx: mpsc::Sender<BackgroundMsg>) {
    let mut last = dark_light::detect().ok().and_then(to_appearance);
    let mut interval = Duration::from_millis(1500);
    let mut consecutive_errors: u32 = 0;
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;
        match dark_light::detect() {
            Ok(mode) => {
                consecutive_errors = 0;
                if interval != Duration::from_millis(1500) {
                    interval = Duration::from_millis(1500);
                    ticker = tokio::time::interval(interval);
                }
                let next = to_appearance(mode);
                if next != last {
                    last = next;
                    if let Some(app) = next {
                        let _ = tx.send(BackgroundMsg::AppearanceChanged(app)).await;
                    }
                }
            }
            Err(_) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == 10 {
                    tracing::warn!(
                        "dark-light detect failed 10 times; backing off to 30s"
                    );
                    interval = Duration::from_secs(30);
                    ticker = tokio::time::interval(interval);
                }
            }
        }
    }
}

fn to_appearance(mode: dark_light::Mode) -> Option<Appearance> {
    match mode {
        dark_light::Mode::Light       => Some(Appearance::Light),
        dark_light::Mode::Dark        => Some(Appearance::Dark),
        dark_light::Mode::Unspecified => None,
    }
}
```

Spawned during App startup, alongside the other background tasks. The
1500ms baseline gives sub-2s switch latency; the 30s backoff stops a
broken interface from hammering syscalls forever without giving up
entirely.

### Handler

In the main message loop, on `BackgroundMsg::AppearanceChanged(os)`:

```rust
let id = match config.appearance.mode {
    AppearanceMode::Auto => match os {
        Appearance::Light => &config.appearance.light_theme,
        Appearance::Dark  => &config.appearance.dark_theme,
    },
    AppearanceMode::Pinned(_) => return,  // explicit user choice wins
};
let theme = registry().find(id)
    .map(|v| Arc::new(v.theme.clone()))
    .unwrap_or_else(|| Arc::new(builtins::default_dark()));
set_active_theme(theme);
request_redraw();
```

### Platform notes

- **macOS:** `dark_light::detect()` reads `AppleInterfaceStyle`. Reliable.
- **Windows:** registry read. Reliable.
- **Linux:** XDG Desktop Portal `org.freedesktop.portal.Settings` —
  works under Flatpak, GNOME, KDE, sway with `xdg-desktop-portal-*`.
  Falls back gracefully when no portal is running (`Mode::Unspecified`,
  we skip).
- **Headless / no DE:** `Unspecified` → poller emits nothing → user stays
  on whatever they picked. Correct behavior.

### Phase 5 retrospective (2026-05-18)

Deviations from the plan:

- **No `[workspace.dependencies]` table in this repo.** The crate has
  one top-level `[dependencies]` and a single-member workspace; the
  `dark-light = "2"` entry landed in `[dependencies]` directly. Same
  effect, fewer indirections.
- **`tokio::task::spawn_blocking` wraps `dark_light::detect`.** macOS
  is a cheap objc read; Linux hits D-Bus through the XDG portal, which
  can block. Pushing the call off the runtime stops a slow detect from
  stalling the runtime on every platform without per-cfg branches.
- **`MissedTickBehavior::Skip` on the interval.** A long detect call
  must not cause the next tick to fire immediately afterward — that
  would burst-poll right after the slow case the backoff is meant to
  cover.
- **Module location: `src/themes/appearance_poller.rs`.** Belongs with
  the theming subsystem; spawned from `terminal.rs::run` right after
  the streaming scan, sharing the `bg_tx` and the tokio `Handle`.
- **`AppearanceChanged` lost its `#[expect(dead_code)]`.** The variant
  now has a real producer; clippy would (correctly) complain otherwise.

Stays on plan:

- 1500ms baseline cadence; 30s backoff after 10 consecutive errors;
  transitions-only emission (identical values coalesce).
- Receiver path (`handle_appearance_changed` → `set_os_appearance` →
  `resolve_and_apply_active_theme`) was wired in Phase 3 and is
  unchanged here.
- `AppearanceMode::Pinned` ignores the signal — the gate lives in
  `resolve_active_theme`, not in the poller or dispatch arm.

## Open questions

- **Per-pane overrides** (one pane uses a different palette). Adds
  surface; defer until a real user need surfaces.
- **High-contrast / accessibility variants.** The format supports them
  trivially via Bold/Italic modifiers and contrasting colors in a
  variant. Curated defaults can ship in a follow-up.
- **Hover-preview a theme before commit** in the settings UI. Cheap to
  add — `set_active_theme` is one call — but adds cancel-vs-commit UX
  complexity. Defer.
- **Theme file editor integrated in the app.** Could be a follow-up to
  Phase 4 (a "duplicate built-in to custom" button that materializes a
  Rust constructor's output as a TOML file in the user's themes dir).
