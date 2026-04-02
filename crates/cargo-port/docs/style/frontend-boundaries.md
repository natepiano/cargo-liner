# Frontend Boundaries

- `src/*` peers are app capabilities and data domains
- `src/tui/*` is one frontend

Use this test:

- if a second frontend existed tomorrow, would it reuse this code unchanged?
- if yes, it probably belongs in `src/*`
- if no, and it is specific to this terminal UI, it belongs in `src/tui/*`

Examples that belong in `src/*`:

- scanning projects
- reading port reports
- watching the filesystem
- talking to GitHub or crates.io
- loading and saving config

Examples that belong in `src/tui/*`:

- rendering
- focus and pane state
- keyboard and mouse handling
- view shaping
- deciding how this UI reacts when config or runtime state changes

Dependency direction matters:

- root modules should not know about `App`, `PaneId`, `ratatui`, `crossterm`, or status flashes
- `src/tui/*` may depend on root modules

Example:

- `config.rs` defines config types, normalization, and load/save behavior
- `tui/config_reload.rs` owns how the TUI reacts to config changes
