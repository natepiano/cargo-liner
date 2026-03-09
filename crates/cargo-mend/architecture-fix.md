# Architecture Fix Plan

This file tracks the remaining architectural cleanup in `cargo-mend` before
returning to real-repo behavior.

## Working rules

- After each architectural step:
  - run `cargo +nightly fmt --all`
  - run `RUSTC_WRAPPER= cargo test`
  - run `RUSTC_WRAPPER= cargo clippy --all-targets`
- Keep the existing suite green after every step.
- Do not operate on the real `obsidian_knife` repo again until:
  - the architectural refactor is complete
  - and the copied `obsidian_knife` regression fixture is green

## Remaining architectural work

1. Remove note-text parsing from `pub_use_fixes`.
   - `pub_use_fixes` must consume typed report facts, not `Finding.related`.
   - `Report` should remain the presentation/output object.
   - `PubUseFixFact` should be the canonical bridge from compiler analysis to
     `--fix-pub-use`.

2. Finish the typed compiler sink pipeline.
   - Use a typed sink for compiler findings plus fix facts.
   - Avoid separate loose vectors and follow-on recomputation.
   - Keep `Finding` creation at the presentation boundary.

3. Keep fixability canonical.
   - `FixSupport` remains the only source of truth for:
     - summary counts
     - CLI fix notes
     - fix-mode eligibility
   - No duplicated “is this fixable?” logic in render/apply paths.

4. Keep operation intent canonical.
   - `ReadOnly`
   - `DryRun { fixes }`
   - `Apply { fixes }`
   - Avoid old boolean-driven fix control flow.

5. Add the missing regression fixture for the remaining `obsidian_knife` case.
   - Child type is still exposed by another crate-visible signature.
   - This must make `--fix-pub-use` ineligible.
   - The test should fail first, then be fixed through the typed pipeline.

6. Improve fix failure outcomes.
   - Remove any leftover generic “during mend analysis” wording for apply-mode
     validation failures.
   - Outcome/result reporting should remain typed end-to-end.

## Success criteria

- All existing tests pass.
- The new `obsidian_knife`-derived regression fixture passes.
- `cargo +nightly fmt --all` passes.
- `RUSTC_WRAPPER= cargo clippy --all-targets` passes.
- `--fix-pub-use` no longer over-classifies cases that are still exposed by
  other crate-visible signatures.
