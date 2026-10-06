# Adding a keybinding

Every action key goes through the keymap system. Never hardcode a `KeyCode` match for action dispatch.

The framework keymap (`tui_pane::Keymap<App>`) owns dispatch, the status bar, the keymap overlay and TOML loading. Each pane scope is a `Shortcuts<App>` host in `src/tui/integration/framework_keymap/`. Paths are relative to `crates/cargo-port/`.

## Checklist

1. Add a variant to the scope's action enum in `src/tui/keymap/actions.rs` (e.g., `ProjectListAction::NewThing`). Its `tui_pane::action_enum!` entry holds the TOML key, the bar label and the description: `NewThing => ("new_thing", "new", "Do the new thing");`. Leave the bar label out and it defaults to the TOML key
2. Add the default binding in the host's `Shortcuts::defaults()` (`tui_pane::bindings!`) in `src/tui/integration/framework_keymap/<pane>_pane.rs`
3. Mirror that default in `ResolvedKeymap::defaults()` in `src/tui/keymap/resolved.rs` when the scope is one of the six it still carries (`project_list`, `package`, `git`, `targets`, `ci_runs`, `lints`) — `defaults_scope_map_consistency` fails on an action with no binding there
4. Add the dispatch arm in the function the host's `Shortcuts::dispatcher()` returns: `dispatch_project_list_action` or `dispatch_output_action` in `src/tui/input/dispatch.rs`, the `dispatch_*_action` functions in `src/tui/panes/actions.rs`, or `dispatch_finder_action` in `src/tui/finder/dispatch.rs`
5. Check the status bar entry. The default `Shortcuts::bar_slots` gives every action a `BarRegion::PaneAction` slot; a host that overrides `bar_slots` (e.g., `ProjectListPane`) must add the new action itself. The bar reads the key from the keymap and the label from `bar_label`
6. Update the expected keymap TOML in `tests/assets/default-keymap.toml` — `framework_keymap_template_matches_golden_file` compares it with what `tui_pane::keymap_toml` writes. The keymap overlay builds its rows from the registered scopes, so it needs no edit; set an order within the section in `keymap_pane_sort_priority` (`src/tui/integration/framework_keymap/app_context.rs`) only when sorting by description is wrong
7. Add tests in `src/tui/app/mod.rs` for the bar and the dispatch (e.g., `focused_project_list_bar_renders_pane_action_and_nav_slots`), and in `src/tui/keymap/load.rs` for TOML parsing and conflicts

## Common mistakes

- Matching `KeyCode::Char('x')` directly instead of going through keymap lookup
- Writing a key into a bar label or help string instead of letting the bar read it from the keymap — `bar_label` names the action, never its key
- Adding a pane scope without registering its host in `build_framework_keymap` (`src/tui/integration/framework_keymap/builder.rs`) and listing its `AppPaneId` in `KEYMAP_OVERLAY_PANE_ORDER` (`src/tui/integration/constants.rs`). An unregistered scope gets no dispatch, bar slots or conflict checks; a pane missing from that list gets no overlay section and no table in the written keymap TOML
