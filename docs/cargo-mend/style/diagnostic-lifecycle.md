## Diagnostic lifecycle

Every new diagnostic must touch all of these files, kept in `DiagnosticCode::ALL` order:

1. `src/config.rs` — `DiagnosticCode` variant, `as_str()` arm, `ALL` array, `DEFAULT_GLOBAL_CONFIG_TOML` line
2. `src/diagnostics.rs` — `DiagnosticSpec` static and match arm in `diagnostic_spec()` (headline, anchor, detail mode, fix support)
3. `src/fix_support.rs` — if auto-fixable: `FixSupport` variant, wire `note()` and `summary_bucket()`
4. `src/run_mode.rs` — if auto-fixable: `FixKind` variant, include in `from_cli()` and `all_fix_kinds()`
5. `src/runner.rs` — wire scan into `plan()`, `build_report()`, and if fixable: `combined_fixes()`, `build_fix_notice()`
6. `README.md` — section under "Diagnostic Reference" with anchor matching `help_anchor`
7. `tests/diagnostics/` — integration test asserting the finding fires; if fixable, assert `--fix` output
