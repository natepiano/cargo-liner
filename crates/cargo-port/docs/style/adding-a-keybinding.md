# Adding a keybinding

Every action key goes through the keymap system. Never hardcode a `KeyCode` match for action dispatch.

## Checklist

1. Add a variant to the action enum in `src/keymap.rs` (e.g., `ProjectListAction::NewThing`)
2. Add the default binding in `ResolvedKeymap::defaults()`
3. Add the TOML key in `default_toml` and `default_toml_from`
4. Add the dispatch arm in the handler — look up from `app.current_keymap.<scope>.by_key`
5. Add a status bar entry in `src/tui/shortcuts.rs` using `by_action` lookup
6. Update the row builder in `src/tui/keymap_ui.rs`
7. Add tests in `src/keymap.rs`

## Common mistakes

- Matching `KeyCode::Char('x')` directly instead of going through keymap lookup
- Using `&'static str` for key labels instead of reading from `by_action`
- Adding a scope without wiring it into conflict detection in `resolve_scope`
