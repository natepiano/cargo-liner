# Style coverage

Mapping between `nate_style/rust/` entries and `cargo-mend` as of 2026-03-29.

51 style entries total. 9 currently enforced by cargo-mend.

## Currently enforced (9)

| Style entry | Rule | Severity | Fix |
|---|---|---|---|
| `no-pubcrate-in-nested-modules` | `forbidden_pub_crate` | error | -- |
| `no-pubin-cratepath` | `forbidden_pub_in_crate` | error | -- |
| `never-use-pub-mod` | `review_pub_mod` | error | -- |
| `leaf-module-visibility` | `suspicious_pub` | warning | `--fix-pub-use` (conditional) |
| `facade-first-visibility` | `internal_parent_pub_use_facade` | warning | `--fix-pub-use` |
| `no-wildcard-reexports` | `wildcard_parent_pub_use` | warning | -- |
| `import-the-module-for-functions-not-the-function-itself` | `prefer_module_import` | warning | `--fix` |
| `import-types-directly` | `inline_path_qualified_type` | warning | `--fix` |
| `prefer-local-relative-imports` | `shorten_local_crate_import` | warning | `--fix` |

## Good candidates (6)

Mechanical, clear signal, low false-positive rate. Natural fit for cargo-mend's AST visitors.

| Style entry | What to detect | Fix? |
|---|---|---|
| `imports-go-at-the-top-of-the-file` | `use` inside fn/impl bodies | detect+fix (hoist to top) |
| `import-constants-at-the-top` | Inline `SCREAMING_SNAKE` paths (`super::constants::FOO`) | detect+fix (extend `inline_path_qualified_type`) |
| `never-bare-allowdeadcode` | `#[allow(dead_code)]` missing `reason` field | detect |
| `never-allowclippytoomanylines` | `#[allow(clippy::too_many_lines)]` | detect |
| `usedunderscorebinding-module-level-allow-only` | `#[allow(clippy::used_underscore_binding)]` not on a `mod` item | detect |
| `never-prefix-unused-fields-or-variables-with` | Struct fields / let bindings starting with `_` | detect (caveat: `_guard` RAII is legitimate) |

## TOML config checks (5)

Different domain (config files, not Rust source) but natural `cargo mend` extension.

| Style entry | What to check |
|---|---|
| `standard-lint-profile` | `Cargo.toml` `[lints.clippy]` matches expected deny list |
| `standard-allowed-lints` | `Cargo.toml` has no unapproved clippy allows |
| `standard-rustfmt-config` | `rustfmt.toml` matches expected config |
| `edition-2024` | `Cargo.toml` `edition = "2024"` |
| `workspace-dependencies` | Member `Cargo.toml` deps use `.workspace = true` |

## Already covered by clippy / rustfmt (10)

Standard lint profile + rustfmt config already enforce these. No cargo-mend work needed.

| Style entry | Covered by |
|---|---|
| `avoid-redundant-closures` | `clippy::redundant_closure` / `redundant_closure_for_method_calls` |
| `borrow-the-slice-not-the-container` | `clippy::ptr_arg` (pedantic) |
| `collapse-if-let-with-inner-conditions` | `clippy::collapsible_if` / `collapsible_match` |
| `inline-variables-in-format-strings` | `clippy::uninlined_format_args` (pedantic) |
| `make-functions-const-fn-when-possible` | `clippy::missing_const_for_fn` (nursery) |
| `methods-that-dont-use-self-should-be-associated-functions` | `clippy::unused_self` (pedantic) |
| `omit-return-in-expression-position` | `clippy::needless_return` |
| `prefer-functional-patterns` | `clippy::option_if_let_else` (nursery) covers the main case |
| `use-usizefrombool-for-bool-to-integer-conversion` | `clippy::bool_to_int_with_if` (pedantic) |
| `one-use-per-line` | `imports_granularity = "Item"` in rustfmt config |

## Possible but noisy (5)

Detectable in theory but high false-positive rate or fix requires design decisions.

| Style entry | Problem |
|---|---|
| `enums-over-bool-for-owned-booleans` | Tons of legitimate bools; fix requires designing new enum types |
| `dont-create-traits-for-single-implementations` | New traits start with one impl before a second arrives |
| `use-a-context-struct-when-arguments-exceed-7` | Param count is detectable but the fix is a design decision |
| `no-magic-values` | Literals everywhere (`0`, `1`, `""`, indices) — false-positive storm |
| `spell-out-names` | Heuristics for "too abbreviated" are fragile (`idx` ok, `a4_h_m` not) |

## Bevy-specific (9)

Domain-specific rules. Could be a `--bevy` flag or separate tool.

| Style entry | Feasibility |
|---|---|
| `api-renames` | Possible — grep for deprecated method names, but tightly coupled to Bevy version |
| `bundles-are-deprecated-use-required-components` | Possible — detect `#[derive(Bundle)]` / `impl Bundle` |
| `messages-are-strongly-discouraged` | Possible — find `MessageWriter` / `MessageReader` usage |
| `prefer-observers-over-events` | Hard — not all `EventReader` usage is wrong |
| `prefer-observers-over-polling` | No — architectural judgment, no syntactic signal |
| `reflectcomponent-suffices-for-brp-mutation` | Possible — detect manual `register_type` where `#[reflect(Component)]` exists |
| `type-registration-is-automatic` | Possible — same as above |
| `current-version-bevy-0180` | No — informational, not a lint |
| `use-bevykana-in-all-bevy-crates` | Possible — check `Cargo.toml` for `bevy_kana` dep when `bevy` is present |

## Not automatable (6)

Process guidance, philosophy, or informational entries.

| Style entry | Reason |
|---|---|
| `always-use-nextest` | Workflow guidance |
| `build-workflow` | Workflow guidance |
| `ci-pattern` | CI config, not source analysis |
| `fix-root-causes-never-workarounds` | Philosophy |
| `if-else-chains-signal-missing-types` | Design smell, requires judgment |
| `exception-std-paths-are-allowed-inline` | Informational — already respected by `inline_path_qualified_type` |

## Meta (1)

| Style entry | Notes |
|---|---|
| `always-use-cargo-mend` | Self-referential |

## Summary

| Category | Count |
|---|---|
| Currently enforced | 9 |
| Good candidates | 6 |
| TOML config checks | 5 |
| Already covered (clippy/rustfmt) | 10 |
| Possible but noisy | 5 |
| Bevy-specific | 9 |
| Not automatable | 6 |
| Meta | 1 |
| **Total** | **51** |
