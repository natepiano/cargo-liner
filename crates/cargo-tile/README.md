# cargo-tile

A terminal UI cargo tool.

Built on [`tui_pane`](../tui_pane), the `ratatui` pane framework this workspace
shares with [`cargo-port`](../cargo-port). The binary is a skeleton today: it
implements `tui_pane::AppContext`, holds a `Framework<App>`, and draws one
placeholder pane.

```bash
cargo run -p cargo-tile   # press q to quit
```
