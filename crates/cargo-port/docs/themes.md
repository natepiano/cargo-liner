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
| 2 | User-themes registry in tui_pane (Registry type + registration API); cargo-port-side bootstrap scans `~/.config/cargo-port/themes/*.toml` (sorted) and calls into the registry; filesystem watcher extended to `themes/` for hot reload | Low | New `theme/registry.rs` in tui_pane, scan + watcher hook in cargo-port, one commit |
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
    registry: ThemeRegistry,
    current:  RwLock<Arc<Theme>>,
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();
```

One init point, one ownership story. The registry and the active theme
share an invariant ("the active theme name must exist in the registry,
or be a compiled-in default") that one struct enforces better than two
independently-managed `OnceLock`s.

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
    registry: ThemeRegistry,
    current:  RwLock<Arc<Theme>>,
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();

pub fn install_theme_state(state: ThemeState) {
    THEME_STATE
        .set(state)
        .unwrap_or_else(|_| panic!("theme state already installed"));
}

pub fn set_active_theme(theme: Arc<Theme>) {
    let state = THEME_STATE.get().expect("theme state not installed");
    *state.current.write().expect("theme RwLock poisoned") = theme;
}

pub fn theme() -> Arc<Theme> {
    let state = THEME_STATE.get().expect("theme state not installed");
    state.current.read().expect("theme RwLock poisoned").clone()
}

pub fn registry() -> &'static ThemeRegistry {
    &THEME_STATE.get().expect("theme state not installed").registry
}
```

Main startup must install the theme state before any render runs. The
`OnceLock::set` returning an error if called twice catches accidental
re-init. The `theme()` accessor panics if not installed — failing loud
beats serving a silent compiled-in default that masks bugs in init
ordering.

In Phase 1 (before Phase 2's registry exists), `install_theme_state`
is called with an empty registry holding only the two built-in
variants from `builtins::default_dark()` and `builtins::default_light()`.

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
    pub fn new_with_builtins() -> Self;
    pub fn register(&mut self, variant: ThemeVariant) -> RegisterOutcome;
    pub fn find(&self, id: &ThemeId) -> Option<&ThemeVariant>;
    pub fn variants_by_appearance(&self, appearance: Appearance)
        -> impl Iterator<Item = &ThemeVariant>;
    pub fn all(&self) -> impl Iterator<Item = &ThemeVariant>;
    pub fn status(&self) -> &RegistryStatus;
}

pub enum RegisterOutcome {
    Inserted,
    Overrode(ThemeId),    // overrode an existing variant with this id
}

pub struct RegistryStatus {
    pub failed_files: Vec<(PathBuf, ThemeLoadError)>,
    pub overridden:   Vec<ThemeId>,
}
```

### Scan (cargo-port)

cargo-port owns the scan code because the path layout is app-specific.
At startup, after parsing `config.toml`, before installing the theme
state:

1. `ThemeRegistry::new_with_builtins()` to seed the registry.
2. `read_dir` on the themes directory, sort entries by filename ASCII
   order (sorted iteration is what makes the "later file overrides
   earlier" tie-break deterministic across runs).
3. For each `*.toml`: parse as `ThemeFamily`. Parse errors → record in
   `RegistryStatus.failed_files`, toast, continue.
4. For each variant in each parsed family: call `registry.register(...)`.
   Each `Overrode` outcome is recorded in `RegistryStatus.overridden`
   and toasted.
5. `install_theme_state(ThemeState { registry, current: ... })`.

### Hot-reload

The existing config/keymap watcher already watches the cargo-port
config directory; extending it to `themes/*.toml` is a one-line addition.
On change:

1. Re-scan the themes directory.
2. Build a fresh registry; replace via a new helper
   `replace_registry(new: ThemeRegistry)` on `ThemeState` (one write
   lock).
3. Re-resolve the active theme name from config and swap.
4. Emit a toast naming what changed.

### Custom `StyleSpec` deserializer

Hand-rolled `Deserialize` for `StyleSpec` and `ColorSpec` — covered in
the "File format" design point above. Emits field-name + offending-value
errors instead of serde's default `unknown variant '...'`.

### No UI yet

A user can edit `config.toml` directly to test. The dropdown UI arrives
in Phase 4.

## Phase 3 — Config schema and apply

### Schema additions to `Config`

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_mode")]
    pub mode:        AppearanceMode,
    #[serde(default = "default_light_name")]
    pub light_theme: ThemeId,
    #[serde(default = "default_dark_name")]
    pub dark_theme:  ThemeId,
}
```

Defaults when section absent: `mode = "dark"`, `light_theme =
"Default Light"`, `dark_theme = "Default Dark"`.

### `BackgroundMsg` variant

`BackgroundMsg` (currently in `src/scan/mod.rs`) gains:

```rust
AppearanceChanged(Appearance),
```

### Apply

After Phase 2's registry is built, the `apply_config` function (in
`src/tui/app/async_tasks/config.rs`) resolves the active theme:

1. Read `config.appearance.mode`.
2. If `Auto`, defer to last-known OS appearance (Phase 5; until then,
   use `Appearance::Dark`).
3. Select `light_theme` or `dark_theme` accordingly.
4. `registry.find(&id)`:
   - hit → `set_active_theme(Arc::new(variant.theme.clone()))`
   - miss → `set_active_theme(Arc::new(builtins::default_dark()))`,
     toast naming the missing id, record the miss
5. Trigger a redraw.

Config file watcher reloads invoke the same path. Reload order is
documented above (theme → keymap).

`mode = "auto"` in Phase 3 behaves identically to `mode = "dark"`
(no detection yet). Phase 5 plugs OS state into the resolve step.

## Phase 4 — Settings overlay UI

Three rows in the existing settings pane:

| Row         | Type            | Values |
|-------------|-----------------|--------|
| Mode        | enum dropdown   | auto / light / dark |
| Light theme | string dropdown | every variant where `appearance == Light` |
| Dark theme  | string dropdown | every variant where `appearance == Dark` |

On selection: write back to `config.toml`, re-run the Phase 3 apply path.
No restart.

If a dropdown's current value isn't in the registry (typed-in or
inherited from an older config), show a "Not found" badge next to the
row, with the missing name preserved in the dropdown so the user can
fix the typo by editing.

If `RegistryStatus.failed_files` is non-empty, show a header banner
above the rows: "N theme files failed to load — see logs." This keeps
silent degradation visible to the user.

The dropdown widgets follow the existing settings-pane pattern
(`tui_pane/src/overlays/settings.rs` already supports enum and string
fields).

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
