---
tier: C
family: types
mechanism: llm
---
## cargo-port test fixtures

The general rule is the global `rust/test-fixture-ownership.md`: a test owns the work it starts through a fixture, and the fixture's `Drop` releases the app, then the workers, then the guards and temp paths. This file gives only cargo-port's own fixtures.

- `make_app` is the quiet fixture. It starts no real startup workers, so it returns a plain `App`.
- `make_app_with_lint_runtime` starts the lint runtime, so it returns an owning fixture, never a plain `App`.
- `KeymapFixture` (keymap tests in `tui/app/mod.rs`) owns the app, the keymap path override guard and its temp dir.
- Lint runtime drop order: dropping the fixture drops the app first, then joins the owned lint runtime, then removes the fixture cache.

```rust
let app = make_app(&projects); // quiet: no real startup workers

let app = make_app_with_lint_runtime(&projects, &config); // owning fixture
drop(app); // joins owned lint runtime before removing fixture cache
```

When you add or change tests in these areas, update the executable gate in the same change: `./scripts/check-no-test-abort.sh`.
