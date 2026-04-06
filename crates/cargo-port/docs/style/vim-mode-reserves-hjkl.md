# Vim mode reserves bare hjkl

When vim navigation is enabled, `h`, `j`, `k`, `l` without modifiers are reserved for navigation. They cannot be used as action keys.

`h`, `j`, `k`, `l` WITH modifiers (`Ctrl+h`, `Alt+j`) are fine — vim normalization only affects bare letters.

Both the keymap loader (`resolve_scope`) and the keymap UI (`handle_awaiting_key`) enforce this. Any new validation site must check `app.navigation_keys().uses_vim()`.
