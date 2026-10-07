# TUI App Template As Built

Status: implemented; the annotated tag `app-template-v1` points at commit
`39a19715`.

`cargo-handler` at that tag is the recommended starting point for a new
`tui_pane` app in this workspace: a complete application whose tiles are empty.
It opens a tile grid on a summary cell with nothing to show; `+` and `-` add and
remove empty tiles; the arrows, Tab and a click move the focus ring; the attract
screen comes with favorites; and the settings, keymap and `?` overlays and the
status line work. None of it is domain logic: the only cargo-specific code is
the `cargo handler` subcommand spelling in `cli.rs`.

The tag does not move. `crates/cargo-handler` on `main` keeps changing after it,
so take the template from `app-template-v1`, not from the working tree. The
user-facing key list and the configuration files are in the crate's README at
the tag (`git show app-template-v1:crates/cargo-handler/README.md`); this
document covers what the template contains, what it leaves out, and how to
start from it.

## Starting From The Tag

```sh
git show app-template-v1:crates/cargo-handler              # list the files
git show app-template-v1:crates/cargo-handler/src/app.rs   # read one
mkdir crates/cargo-foo
git archive app-template-v1 crates/cargo-handler | tar -x --strip-components=2 -C crates/cargo-foo
```

`git checkout app-template-v1 -- crates/cargo-handler` writes the tag's files
over `crates/cargo-handler` itself, in the working tree and the index, which
builds and runs the template as tagged (`cargo run -p cargo-handler`). Files
`main` added to the crate after the tag stay in place through it, so copy a new
crate out with `git archive`, not from a checked-out directory.

In the new crate, with `cargo-foo` as its name, these name the crate
(`grep -rni handler crates/cargo-foo` lists every site, doc comments included):

| file | what to rename |
| --- | --- |
| `Cargo.toml` | `name`, `description`, `homepage` |
| `src/constants.rs` | `BINARY_NAME`, `CONFIG_DIRNAME`, `SUBCOMMAND_NAME` (the name without `cargo-`), `CLI_ABOUT`, `APP_NAME` (padded with a space each side for the status line), the first line of `KEYMAP_TOML_HEADER` |
| `src/config.rs` | the identity type `CargoHandler`, also named in `app.rs`, `settings.rs` and `terminal.rs`; `iterm2_profile = "cargo-handler"` in two tests, because the profile default is `AppIdentity::DEFAULT_ITERM2_PROFILE`, which is `BINARY_NAME` |
| `src/render.rs` | the settings golden's three `/<config>/cargo-handler/...` rows and the `cargo-handler` literal in `status_row`. The settings overlay is as wide as its widest row, one of those paths, so a name of another length can change the overlay's width: redraw that golden rather than editing its text |
| `src/cli.rs` | `CARGO_HANDLER_TEST_ARGUMENTS` in `cli_rejects_unknown_process_arguments` |
| `assets/cargo-handler.desktop.in` | the file name (`assets/cargo-foo.desktop.in`), `Name=`, `Comment=`, and the install line in its header comment |
| `themes/*.toml` | the comments naming the config directory; `starter.toml`'s family `name` and its variant `Cargo Handler Dark`, which must not match a built-in id |
| `README.md`, `CHANGELOG.md` | rewritten for the new tool |

The workspace members glob is `crates/*`, so the new directory is a member, and
`Cargo.lock` gains the package on the first build. CI's workspace-wide jobs
match every crate except cargo-mend, so they cover it with no edit. The macOS
job names its crates (`-p cargo-tile -p cargo-handler`); add `-p cargo-foo`
there for macOS coverage of the attract backdrop's capture path.

`main.rs` is the module list and one line:

```rust
fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
```

## What Is In It

Thirteen modules: about 1,350 lines outside the test modules, 268 of them the
built-in palettes in `theme.rs`, and about 1,000 lines of tests.

| file | what it holds |
| --- | --- |
| `main.rs` | the module list and `fn main` |
| `cli.rs` | `Cli`, a clap parser with no options, and `without_subcommand_name`, which drops the `handler` word cargo passes ahead of the arguments for `cargo handler` |
| `terminal.rs` | `run`: load `config.toml`, install the theme, build `App`, and hand it to `tui_pane::run_terminal`; `Ticker`, the `PollWork` that moves the favorites fade, the grid's motion and the attract screen's frames on each pass |
| `app.rs` | `App`, `AppPaneId`, `APP_PANE_DISPLAY_ORDER`, and the framework trait impls below |
| `keymap.rs` | `build_keymap`, and which framework globals the `?` overlay lists |
| `globals.rs` | `AppGlobalAction`: the twelve app globals, their default keys, and `dispatch` |
| `interaction.rs` | `HitTestRegistry` and `InputContext` for `App`: a click focuses the tile under it, and the open favorites overlay absorbs clicks |
| `render.rs` | `draw`: the grid under the attract layers, the status line, the toasts, the favorites overlay and the framework overlay; `Cells`, the `TileCells` impl |
| `tiles.rs` | `NoGroup`, an uninhabited group type, and the `TileGrid`, `TileContent` and `TileDemands` aliases over it |
| `settings.rs` | the settings overlay's rows and `cycle`, which steps them; `AppSetting`, uninhabited |
| `config.rs` | `CargoHandler`, the `AppIdentity`; `Config` with `[appearance]` and `[tiles]`; `LoadedConfig` |
| `theme.rs` | the four built-in theme variants |
| `constants.rs` | names, labels, section headings, the keymap TOML header, the status line height |

Beside `src/`: `themes/` (`default_dark.toml`, `default_light.toml`,
`high_contrast.toml` with both high-contrast variants, and `starter.toml`),
`assets/cargo-handler.desktop.in`, the README, the changelog and the two
licenses. `Cargo.toml` enables `tui_pane`'s `backdrop` feature, which captures
the desktop behind the window for the attract screen.

`App`'s trait impls are the list of what a `tui_pane` app supplies:

| trait | what `App` supplies |
| --- | --- |
| `AppContext` | the `Framework`; `ToastAction = NoToastAction` |
| `SettingsHost` | `step_setting`, which calls `settings::cycle` |
| `KeymapUiContext` | the inline error, and `APP_PANE_DISPLAY_ORDER` |
| `KeymapEditContext` | `AppGlobals`, `KEYMAP_TOML_HEADER`, the keymap path, the inline error's set and clear, and `reload_keymap` |
| `TerminalApp` | `Identity`, `Probe = NoProbe`, `keymap`, `draw`, `click`; the hooks `before_draw`, `visual_deadline`, `modal_key`, `attract_key`, `resized`, `resize_settled`, `holds_full_repaint` and `before_exit`, each one call into the attract screen or the favorites overlay |
| `TileGridHost` | the grid, under `AppPaneId::Main` |
| `AttractHost` | the `Attract`, and the three `AppPaneId::Attract` ids |
| `FavoritesHost` | the favorites overlay, under `AppPaneId::Favorites` |
| `HitTestRegistry`, `InputContext` | where a click lands (`interaction.rs`) |

The other impls: `Globals<App>` for `AppGlobalAction`, `PollWork<App>` for
`Ticker`, `TileCells<NoGroup>` for `Cells`, `AppIdentity` for `CargoHandler`,
and `AppConfig` for `Config`.

`TileCells` requires `summary_title`, `demands` and `draw`. Its defaulted
`summary_foot`, `summary_labels` and `group_title` methods let an app add
summary-foot text and border labels or name group cells without changing a
minimal implementation. `summary_foot` returns `SummaryFoot::Empty` by default.

Three choices come from `tui_pane` itself and hold for every app built on it.
The summary cell is always there: `tui_pane::TABLE_CELL` is the constant 1, and
no grid exists without it. The grid actions (`+`, `-`, the arrows) are variants
of the app's own `AppGlobalAction`, whose `dispatch` calls `TileGrid::apply`
with a `tui_pane` `TileAction`, so they stay in the app's `[global]` keymap
table. `tui_pane` ships no colours: each app hands its own built-in palettes to
`ThemeRegistry::from_dir_with_builtins` and keeps its own `themes/*.toml`.

## What The Template Already Does

- **The grid.** Cell one is titled `summary` and reads `nothing to show yet`.
  `+` adds a numbered empty cell, `-` removes an empty one, the arrows and
  Tab / shift-Tab move the focus ring, and a click focuses the cell under it.
  The first column grows to `[tiles] initial_rows` (default 4, stepped in the
  settings overlay) before the grid squares up, and cells animate between
  arrangements. Every cell reserves its last row for the framework's readout;
  the summary can also carry an app-supplied foot at the left end of that row.
- **The attract screen.** `a` shows or hides it, and it comes on by itself
  over the idle grid after a quiet spell. `r` draws one at random and `u` undoes
  that replacement. While it was asked for, the keys its three animation scopes
  bind (`[attract_moving_band]`, `[attract_moving_text]`, `[attract_pixelate]`)
  go to it ahead of the grid's; `q` still quits and `a` gives the grid back. The
  status line shows `attract` while it was asked for.
- **Favorites.** ctrl-s saves the attract parameters on screen to
  `favorites.toml`, ctrl-o opens the saved list, `m` shows one at random. The
  open favorites overlay owns every key (`modal_key`) and absorbs every click.
- **Three framework overlays.** `s` settings: the Appearance steppers (mode,
  light theme, dark theme), Tiles (initial rows), the Files paths and the
  Notices. ctrl-k keymap editor: Enter on a row captures the next key, writes
  `keymap.toml`, and calls `reload_keymap`. `?` global shortcuts, which leaves
  out the `x` Dismiss row while `x` keeps its binding. Esc closes each.
- **The keymap.** The eight framework globals come from
  `tui_pane::GlobalAction`'s defaults: `?`, ctrl-k, `s`, `q`, `R`, `x`, Tab,
  shift-Tab. `keymap.toml` overrides any binding, and an entry the build does
  not know is skipped (`ignore_unknown_entries`) rather than refusing to start.
  Its tables are `[global]` (framework and app globals together),
  `[navigation]` (`SettingsNavigation`, which moves through the settings
  overlay), the three attract tables, `[favorites]` and `[overlay]`.
- **Theming.** Four built-in variants; `themes/*.toml` under the config
  directory layer over them, replacing a built-in when the names match. The
  settings overlay switches them live and writes `config.toml`; `mode = "auto"`
  follows the terminal.
- **iTerm2.** While it runs, the iTerm2 session wears the profile
  `iterm2_profile` names (default: the binary name), put back on exit and on a
  panic. `""` leaves the session alone.
- **The status line.** Uptime at the left; the app name and version, the
  `attract` flag, and `? shortcuts` at the right.
- **Restart in place.** `R` re-execs the binary with its original arguments,
  so the shell that ran `cargo run` keeps waiting on the same job.
- **Demand-driven drawing.** `run_terminal` reads input on its own thread and
  draws a frame only when input arrives or `Ticker` returns `Repaint::Needed`:
  while the grid moves, while the attract screen is up and once at the end of
  the quiet it waits out, and while the favorites overlay fades a row out. A
  burst of resize events is drained before the next draw.

## What It Deliberately Leaves Out

Each of these is an extension point. Most are uninhabited types, so filling one
in makes the compiler name every site that has to handle it.

- **Tile contents.** `NoGroup` has no values, `Cells::draw` draws only the
  summary's note, and `Cells::demands` asks for no room. An app with something
  to show replaces `NoGroup` with its own id type and draws it in `Cells`.
- **Running work.** `render::draw` always passes `AttractWork::Idle`, so the
  attract screen takes the terminal after its quiet spell; an app passes
  `AttractWork::Running` while its work runs, and the attract screen gives the
  terminal back. `Ticker` holds no channel. Outside data is drained in
  `Ticker::poll`, which returns `Repaint::Needed` when it changes the screen;
  `run_terminal`'s `start` closure builds the ticker once the terminal is set
  up, so nothing it spawns runs before then.
- **App settings.** `AppSetting` is uninhabited, and `cycle` answers
  `SettingTarget::App(setting)` with `match setting {}`.
- **Command-line options.** `Cli` is `struct Cli {}`, and `Cli::run`
  destructures it with `let Self {} = self;`, so an added option has to be
  handled there.
- **Toast actions.** `ToastAction = NoToastAction`. Toasts themselves are
  drawn: `u` with nothing to undo shows `Nothing to undo`.
- **A frame probe.** `Probe = NoProbe`, and `Attract` is
  `tui_pane::Attract<NoProbe>`, so no frame log is kept.
- **Hover.** `HitTestRegistry::viewport_mut` returns `None`: the grid tracks
  focus by cell, not through a viewport.

## Adding To It

| to add | do this |
| --- | --- |
| a global shortcut | a variant in the `action_enum!` in `globals.rs`, a default key in `Globals::defaults`, and an arm in `dispatch`. `render_order` returns `Action::ALL`, so the keymap and `?` overlays list it with no further step. `each_default_key_maps_to_its_action` and `EXPECTED_OVERLAY_ROWS` change with it, and so do the keymap and `?` overlay frames when its row lands on their first screen (Global Shortcuts rows sort by description) |
| a setting | a field on `Config`, an `AppSetting` variant, a `SettingsRows::stepper` row in `settings::rows`, and an arm in `cycle`'s `SettingTarget::App` match; `the_default_config_serializes_to_the_known_file` pins the default `config.toml` |
| a pane | an `AppPaneId` variant; a registration in `keymap::build_keymap`, `register` for a pane whose keys go in `keymap.toml` as the attract panes and favorites do, `register_pane` for one with none as the grid does; an entry in `APP_PANE_DISPLAY_ORDER`, or its shortcuts go unlisted; an arm in `HitTestRegistry::pane` and a place in `HIT_TEST_Z_ORDER` if a click can land on it; a rectangle in `render::draw_panes`, which gives the whole body to the grid |
| a status-line entry | a `StatusLineNote` in `render::draw_status_line`'s notes, or a `StatusLineGlobal` in its globals |
| tile contents | an id type in place of `NoGroup` in `tiles.rs`, and `Cells`' `summary_title`, `demands` and `draw` in `render.rs`; implement `summary_foot` when the summary needs text on its bottom row |

## Tests And Goldens

`cargo nextest run -p cargo-handler` runs 32 tests:

| file | tests | what they pin |
| --- | --- | --- |
| `render.rs` | 8 | 80×24 frames: the first frame, after `+ +`, after `+ + -`, the settings overlay, the keymap overlay's first screen, the `?` overlay's first screen, and the favorites overlay on one saved favorite; the attract test compares only the status line, because the body depends on the desktop behind the terminal |
| `keymap.rs` | 4 | the keymap assembles with a navigation scope; the attract and favorites table names; `?` leaves out dismiss; the keymap overlay's 89 rows in order (`EXPECTED_OVERLAY_ROWS`) |
| `theme.rs` | 5 | each mirror under `themes/` equals its constructor; the starter parses under a name no built-in uses; the four built-in ids in order |
| `cli.rs` | 4 | the bare binary and the `cargo handler` spelling parse; a later `handler` is left alone; an unknown argument exits 2, checked in a child process |
| `config.rs` | 3 | the default `config.toml` text; a complete config serializes to itself; missing keys are restated at their defaults |
| `globals.rs` | 3 | each default key reaches its action, and there is one per variant; `r` asks for the attract screen; `u` with nothing to undo shows a toast |
| `terminal.rs` | 3 | `Ticker` repaints only while something moves; the favorites overlay takes keys until Esc; each framework overlay opens on its key and closes on Esc |
| `interaction.rs` | 2 | a click on the second cell focuses it; the open favorites overlay absorbs a click |

In a test build, `CargoHandler`'s `config_path`, `keymap_path` and `themes_dir`
resolve under `/<config>/cargo-handler/` (`TEST_CONFIG_ROOT`), a directory a
test has no permission to create: a test that saves a setting fails instead of
writing the user's files, and the settings overlay draws the same width on
every machine. `App::new_for_test` builds on the default config and the default
keymap and reads no file. The goldens reset `app.started` so the uptime reads
`0s`, and the favorites golden's favorite is dated 2020, so its row reads the
same whatever year the test runs.

## KDE Desktop Entry

`assets/cargo-handler.desktop.in` is not a menu entry (`NoDisplay=true`). It
names `org.kde.KWin.ScreenShot2` in `X-KDE-DBUS-Restricted-Interfaces`, and KWin
grants that interface by matching the entry's `Exec` path to the running
binary. Without it the attract backdrop draws Plasma's wallpaper instead of the
windows behind the terminal, and nothing on screen says why.
`scripts/install-desktop-entry.sh` writes it to
`~/.local/share/applications` (under `XDG_DATA_HOME` when set) with `@EXEC@`
replaced by the binary's absolute path, then rebuilds KDE's cache:

```sh
scripts/install-desktop-entry.sh --crate cargo-handler   # the cargo-handler on PATH
scripts/install-desktop-entry.sh target/release/cargo-handler cargo-handler-dev
```

The script reads `crates/<crate>/assets/<crate>.desktop.in`, so a new crate
whose asset is renamed to match installs with `--crate cargo-foo` and no script
change. Without `--crate`, the crate is the binary argument's file name when a
template exists under that name, and cargo-tile otherwise. Rerun it after the
binary moves or is reinstalled, and give each build its own entry name. On any
desktop but KDE Plasma under Wayland the entry is inert.

## One Structural Choice Worth Repeating

The keymap lives on `App` as `Rc<Keymap<App>>`. Dispatch needs `&Keymap<App>`
and `&mut App` at once, and rebinding a key in the keymap overlay replaces the
whole map mid-dispatch, so `TerminalApp::keymap` hands out an `Rc::clone` that
the loop re-reads at every frame and every key. `reload_keymap` rebuilds from
the file the editor just wrote through `build_keymap`, the same path a
hand-edited `keymap.toml` takes at startup.
