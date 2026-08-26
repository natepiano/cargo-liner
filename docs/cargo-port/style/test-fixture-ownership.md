## Test Fixture Ownership

Tests must make background work, process-global overrides, and fixture lifetimes explicit. Prefer quiet fixtures by default; only opt into real startup effects when the test owns every handle, guard, temp directory, and teardown path.

Use contextual panics in test helpers, not `std::process::abort()`. If a helper starts work that can outlive the test, return an owning fixture wrapper that drops the app first, joins/drains workers, then releases override guards and temp directories.

```rust
let app = make_app(&projects); // quiet: no real startup workers

let app = make_app_with_lint_runtime(&projects, &config); // owning fixture
drop(app); // joins owned lint runtime before removing fixture cache
```

Do not return a plain `App` from helpers that depend on live config, keymap, theme, cache, or worker ownership unless the helper deliberately persists those paths and suppresses real effects.

Tests that exercise process-global-ish startup fixtures should be stable under nextest. If a focused regression gate combines such tests, either make the fixtures process-isolated or run that focused gate with `--test-threads 1` and document why.

When adding or changing tests in these areas, update the executable gates in the same change set: `./scripts/check-no-test-abort.sh` and `PHASE6_REPEAT_COUNT=10 ./scripts/repeat-phase6-nextest.sh`.
