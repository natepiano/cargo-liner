# cargo-tile favorites — follow-up work — Next

## Items to consider

- [ ] **Name the absent case at `backdrop/`'s platform boundary**
  - Target: `crates/tui_pane/src/backdrop/` — `query.rs` and the CoreGraphics read path in `desktop.rs`
  - Why needed: six optional outcomes sit at the foreign-API edge and still require callers to infer whether the system supplied no usable answer, the requested property is genuinely absent, or a completed lookup found no match — `query::window_origin` returns `Option<(f64, f64)>` for the terminal's screen position, `TerminalWindowCandidate::owner` returns `Option<i32>` for the owning process, `platform::window_titled` and `platform::window_at` each return `Option<u32>` before the shared wrappers construct `TerminalWindowSearchOutcome`, `platform::number` reads a window id out of a CoreGraphics dictionary as `Option<u32>`, and `platform::window_titles` returns `Vec<(u32, Option<String>)>`
  - Completion condition: each of the six optional outcomes is represented by a named domain type at the `backdrop/` boundary, with the existing behavior unchanged and the feature-enabled backdrop suite green
  - Revealed by: Phases 16 and 17
