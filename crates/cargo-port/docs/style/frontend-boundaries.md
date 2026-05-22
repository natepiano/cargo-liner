# Frontend Boundaries

The repo is split into two crates:

- **`tui_pane/`** — the framework. Reusable across any app that embeds the pane system. Owns rendering primitives, the keymap system, the status bar, the toast system, the theme system (registry, loader, resolver, OS appearance poller, runtime state), and `Appearance` (light/dark).
- **`cargo-port` (`src/`)** — the app. Owns cargo/project/git/CI domain logic, app-specific config schema and paths, the CLI, and the wiring that composes framework primitives into this terminal UI.

## What goes where

**Framework (`tui_pane/`):** anything not specific to cargo-port. If a future client would want it verbatim, it belongs in the framework.

- Rendering, focus, panes, layout, keymap, status bar, toasts
- Theme primitives: types, registry, file loader, watch, resolver, OS appearance poller, runtime state struct
- `Appearance` (light/dark) and any other domain-neutral theme concepts

**App (`src/`):**

- Domain logic: scanning projects, port reports, watching the filesystem, talking to GitHub or crates.io, lint orchestration
- App-specific config: `AppearanceConfig` (the cargo-port TOML schema) and other config types
- App-specific paths: where on disk the user themes directory lives (uses `APP_NAME = "cargo-port"`)
- Wiring: composing the framework's primitives into this app's main loop, background message channel, and event flow

## Dependency direction

- **`src/tui/*` may depend on `tui_pane` and on root modules.** It is the composition layer.
- **Root modules under `src/*` (outside `src/tui/*`)** are app capabilities and data domains. They may depend on `tui_pane` types that represent generic concepts (e.g. `tui_pane::Appearance`) — these are framework *primitives*, not TUI-internal types. They should not depend on TUI-internal types like `App`, `PaneId`, `Framework`, ratatui widgets, crossterm events, or status flashes.
- **`tui_pane` depends on neither.** It is reusable on its own.

## The test that matters

When deciding where to put new code, ask: *is this specific to cargo-port, or is it generic to any pane-based TUI app?*

- Specific to cargo-port → app.
- Generic to any pane-based TUI → framework.

Don't gate on a hypothetical second client. The framework is the framework; generic code lives in it on principle.
