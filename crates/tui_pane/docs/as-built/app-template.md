# TUI App Template As Built

Status: implemented, tagged `app-template-v1`.

`cargo-tile` at that tag is a complete `tui_pane` application with no
application in it. Everything present is framework wiring that every TUI needs;
nothing present is about cargo, tiles, or any particular domain. It is the
recommended starting point for a new TUI app in this workspace.

Everything below describes the tag, which does not move. `crates/cargo-tile` on
`main` has since grown a running-cargo process table and the tests that come
with it, so the working tree and this document are expected to diverge — take
the template from `app-template-v1`, not from `main`.

This is an as-built record of what the tag contains, not a plan. The
user-facing key list and the "how do I add X" table live in
`crates/cargo-tile/README.md`; this document covers what the template is, what
it deliberately leaves out, and how to start from it.

## Starting From The Tag

```sh
git show app-template-v1:crates/cargo-tile   # inspect without checking out
git checkout app-template-v1 -- crates/cargo-tile
```

Then copy `crates/cargo-tile` to the new crate name, rename the package in
`Cargo.toml`, change `BINARY_NAME` and `CONFIG_DIRNAME` in `constants.rs`, and
add the crate to the workspace members glob (it is `crates/*`, so a new
directory is already a member).

`main.rs` is the whole entry point:

```rust
fn main() -> ExitCode { terminal::run() }
```

## What Is In It

Ten modules, ~1,250 lines, all of it framework wiring:

| file | what it holds |
| --- | --- |
| `app.rs` | `App` and its four framework trait impls — `AppContext`, `KeymapUiContext`, `KeymapEditContext`; the `AppPaneId` enum |
| `terminal.rs` | terminal lifecycle, the input thread, the demand-driven event loop, the key dispatch ladder |
| `render.rs` | frame layout, the status line, and the three framework overlays |
| `keymap.rs` | keymap assembly — `register_globals`, `register_overlay`, `register_pane` |
| `globals.rs` | the app-globals scope, empty until the app adds an action |
| `settings.rs` | the settings overlay's rows and their steppers |
| `config.rs` | `config.toml` load and the config-directory paths |
| `theme.rs` | theme resolution and installation |
| `constants.rs` | filenames, popup geometry, the keymap TOML header |
| `main.rs` | `fn main` |

## What The Template Already Does

All of this works at the tag, with no app code:

- **Eight global shortcuts** from `tui_pane::GlobalAction`'s defaults — `?`
  shortcuts, ctrl-k keymap, `s` settings, `q` quit, `R` restart, `x` dismiss,
  Tab / shift-Tab pane cycling.
- **Three overlays** — settings, keymap, and the compact `?` global-shortcuts
  list — each rendered by the framework and dispatched to from
  `terminal::dispatch_overlay_key`.
- **Keymap editing.** Enter on a row in the keymap overlay (or the `?` overlay)
  captures the next keypress, checks it against every binding in force, writes
  `keymap.toml`, and reloads. The whole flow is `tui_pane::KeymapEditContext`;
  `App` supplies only the file path, the TOML header, inline-error get/set, and
  how to rebuild the keymap afterwards.
- **Theming** — built-in themes plus `themes/*.toml`, switched live from the
  settings overlay, which writes `config.toml`.
- **A status line** with an uptime segment and a `? shortcuts` slot in the
  bottom-right corner.
- **Restart in place** — `R` re-execs the binary with its original arguments,
  so the shell that ran `cargo run` keeps waiting on the same job.
- **Demand-driven rendering.** A frame is painted only when an event arrives,
  so an idle app costs essentially nothing. An app with live data sets the
  loop's `dirty` flag when that data changes.
- **Input on its own thread.** `event::read` blocks whenever crossterm's
  buffered bytes do not yet form a whole event. On the render thread that
  stalls drawing and the per-frame size query, which is how a resize ends up
  invisible; on its own thread it stalls nothing but itself.

## What It Deliberately Leaves Out

Each of these is an extension point, not an oversight:

- **No app-pane shortcuts.** `MainPane` registers through `register_pane`
  rather than `register`, because it has no actions of its own yet.
- **No app globals.** `AppGlobalAction` is an enum with no variants, so the
  app-globals scope is empty and `[global]` in `keymap.toml` accepts only
  framework action names. Because `action_enum!` requires at least one variant,
  its `Action` impl is written by hand — `ALL` is `&[]` and every method is
  `match self {}`.
- **No navigation scope.** `register_navigation` is never called. Pane cycling
  comes from the framework globals instead. An app with list panes registers a
  navigation impl and gets arrow / page / home / end handling with it.
- **No toasts.** `App::ToastAction` is `NoToastAction`.
- **No tests.** `cargo nextest run -p cargo-tile` reports "0 tests run" and
  exits 4. The template has no logic worth locking down; the framework code it
  drives is covered by `tui_pane`'s and cargo-port's suites.

## Framework Surface It Exercises

Worth knowing, because it is the shortest list of `tui_pane` types a working
app actually needs:

- **State and traits** — `AppContext`, `Framework`, `FocusedPane`,
  `NoToastAction`, `Pane`.
- **Keymap** — `Keymap`, `KeymapError`, `Action`, `Bindings`, `Globals`,
  `GlobalAction`, `OverlayAction`, `KeyBind`.
- **Overlays** — `FrameworkOverlayId`, `KeymapPane`, `KeymapUiContext`,
  `KeymapEditContext`, `SettingsRow`, `SettingsRenderOptions`, `PopupFrame`,
  `matches_open_overlay_toggle`, `overlay_is_in_text_mode`.
- **Rendering** — `render_status_line`, `StatusLine`, `StatusLineGlobal`,
  `ScanIndicator`, `BarPalette`, `RenderFocus`, `PaneFocusState`,
  `selection_style`, `SECTION_HEADER_INDENT`, `SECTION_ITEM_INDENT`,
  `FRAME_POLL_MILLIS`.
- **Theming** — `ThemeRegistry`, `ThemeState`, `Appearance`, and the color
  accessors (`accent_color`, `title_color`, `text_default`, and the rest).

The two framework overlay panes it never names are `GlobalShortcutsPane` and
`SettingsPane`: both are reached as fields on `Framework`, so the template uses
them without importing their types.

## One Structural Choice Worth Repeating

The keymap lives on `App` as `Rc<Keymap<App>>`, not as a local in the event
loop. Rebinding a key rebuilds the whole keymap, so the loop cannot both own it
and hand out `&Keymap<App>` to dispatch. Every dispatch site starts with
`Rc::clone(&app.keymap)`. cargo-port does the same thing for the same reason.
