# cargo-tile favorites — follow-up work — Next

## Items to consider

- [ ] **Name the absent case at `backdrop/`'s platform boundary**
  - Target: `crates/tui_pane/src/backdrop/` — `query.rs` and the CoreGraphics read path in `desktop.rs`
  - Why needed: four bare options sit at the foreign-API edge and each `None` conflates "the system did not answer" with "there is genuinely nothing here", leaving callers to decode which by context — `query::window_origin` returns `Option<(f64, f64)>` for the terminal's screen position, `TerminalWindowCandidate::owner` returns `Option<i32>` for the owning process, `platform::number` reads a window id out of a CoreGraphics dictionary as `Option<u32>`, and `window_titles` returns `Vec<(u32, Option<String>)>`
  - Completion condition: each of the four returns a named outcome that states which case it is, with the existing behavior unchanged and the feature-enabled backdrop suite green
  - Revealed by: Phase 16
