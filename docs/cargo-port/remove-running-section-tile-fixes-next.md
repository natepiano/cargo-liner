# tile-fixes — Next

## Items to consider

- **Wide characters wrap by the cells they take** — Target: `crates/tui_pane/src/wrap.rs` `cell_count` and `split_at_cells`, which count characters, not terminal cells. Need: a word or a nonbreaking run holding double-width characters is measured at half its drawn width, so a line can be clipped with no mark; the counting is older than phase 15, whose nonbreaking runs now promise a mark. Completion: the wrapper measures and splits by terminal cells without splitting a grapheme; tests with double-width text through `wrapped`, `wrapped_runs` and the toast renderer. Source: Phase 15 (architect review). Placed here by the user's answer, relayed by the showrunner 2026-10-13: no capture has shown wide text yet.
