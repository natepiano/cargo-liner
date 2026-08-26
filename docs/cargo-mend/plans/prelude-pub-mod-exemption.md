# Spec: auto-allow crate-root `pub mod prelude`

## Goal
Stop the `ReviewPubMod` finding (a `Severity::Error`) from firing on a crate-root
`pub mod prelude;`, by default, with no per-project `mend.toml` override. Add a
default-on switch in the auto-created global config so the behavior is discoverable
and can be turned off. Keep existing generated global configs working via a
comment-preserving reconcile pass (no dated migration to remove later).

## Behavior
- Exempt iff: item is a module, `visibility_text` starts with `pub`,
  `item.name == "prelude"`, and the declaring location is the crate root
  (`ModuleLocation::CrateRoot`).
- Nested `pub mod prelude;` and any other crate-root `pub mod foo;` still fire.
- The existing `allow_pub_mod` file allowlist is unchanged and still works
  (it is broader: it exempts every `pub mod` in a named file).
- Switch default = exempt. Set `allow_prelude_pub_mod = false` to review
  crate-root preludes like any other `pub mod`.
- The switch lives only in the global config; project `mend.toml` does not override it.

## Config model
New enum `PreludePubMod { Allowed, Reviewed }` (`src/config/prelude_pub_mod.rs`),
default `Allowed`, serialized as a bool on the wire — same pattern as
`DiagnosticStatus`. New resolved field on `VisibilityConfig`:

```rust
#[serde(default, rename = "allow_prelude_pub_mod")]
pub(crate) prelude_pub_mod: PreludePubMod,
```

`VisibilityConfig` is already (a) serialized into the cache fingerprint and
(b) serialized to JSON for the rustc driver, so the new field reaches the scanner
and invalidates the cache when toggled — no extra plumbing. `load_config` stamps the
resolved global value into `VisibilityConfig.prelude_pub_mod` **before** computing the
fingerprint, in both the found-config and no-config branches.

## Global config: reconcile (replaces one-off migration)
`load_global_diagnostics` becomes `load_global_config -> GlobalConfig
{ diagnostics, prelude_pub_mod }`. On load it reconciles the global config file:

- Missing file → write the canonical default (rendered from the schema: every
  `DiagnosticCode::ALL` key `= true` under `[diagnostics]`, plus
  `allow_prelude_pub_mod = true` under `[visibility]` with its comment).
- Existing file → parse with `toml_edit` (format-preserving). For each schema key
  absent under its table, insert it with its default; create `[visibility]` only if
  missing. **Write back only if a key was inserted.** Comments, ordering, and explicit
  values are preserved; a complete file is a no-op.
- Read/parse/write failure degrades to in-memory defaults; never errors the run.

This is permanent infrastructure: every future global toggle is one schema entry and
is reconciled automatically. Source of truth for diagnostics keys is
`DiagnosticCode::ALL` + `as_str()`.

### Dependency
Add `toml_edit = "0.25"` (`toml` 1.x dropped the `toml_edit` backing, so it is a
separate crate). Used only for comment-preserving reconcile.

## Detection (`src/compiler/visibility/scan/record.rs`)
In `record_review_pub_mod`, after the module/`pub` guard:

```rust
if matches!(ctx.settings.visibility_config.prelude_pub_mod, PreludePubMod::Allowed)
    && item.name == Some("prelude")
    && finding_context.module_location == ModuleLocation::CrateRoot
{
    return Ok(());
}
```

`module_location` and `item.name` are already available; `ModuleLocation` is already
imported.

## Call sites
- `src/main.rs` — `load_global_config()`, pass `&global.diagnostics` to the help
  builder and `&global` to `load_config`.
- `src/config/loaded.rs` — `load_config` takes `&GlobalConfig`; stamp
  `prelude_pub_mod` before fingerprinting in both branches.
- `src/config/global.rs` — schema-driven default + reconcile; return `GlobalConfig`.
- `src/config/mod.rs` — export `PreludePubMod`, `GlobalConfig`, `load_global_config`.
- No change in `execute.rs` / `settings.rs` — they carry `VisibilityConfig` whole.

## Tests
- `prelude_pub_mod`: bool round-trip, default `Allowed`.
- `global`: reconcile inserts only missing keys; preserves a complete file with
  comments byte-for-byte (no write); inserts a missing key while keeping existing
  comments; idempotent second run; missing file → canonical default that parses;
  every `DiagnosticCode::ALL` variant present after reconcile.
- integration (`tests/diagnostics/`): crate-root `pub mod prelude;` with no
  `mend.toml` is not flagged; nested `pub mod prelude;` still flagged; crate-root
  `pub mod other;` still flagged.

## Docs
- README config section + `CHANGELOG.md`.

## Knock-on
`bevy_kana` can delete its `mend.toml` override (`allow_pub_mod = ["src/lib.rs"]`):
its `src/lib.rs` has only `pub mod prelude;` public. cargo-mend's own `mend.toml` is
project-specific — verify before dropping.
