# cargo-vischeck

`cargo-vischeck` provides the `cargo vischeck` subcommand for enforcing a stricter Rust visibility style across a crate or workspace.

## V1 policy

Hard errors:

- `pub(crate)` is forbidden
- `pub(in crate::...)` is forbidden
- `pub mod` requires an explicit allowlist entry

Warnings:

- bare `pub` in a nested child file where the parent module is private and does not publicly
  re-export the item

This is intentionally a heuristic tool, not a full compiler-resolved truth engine.

## Config

The tool looks for `vischeck.toml` at the target root.

```toml
[visibility]
allow_pub_mod = [
  "mcp/src/brp_tools/tools/mod.rs",
]
allow_pub_items = [
  "src/example/private_child.rs::SomeIntentionalFacadeItem",
]
```

`visibility_audit.toml` and `.visibility-audit.toml` are also accepted as legacy fallback names.

## Usage

```bash
cargo vischeck
cargo vischeck --fail-on-warn
cargo vischeck --json
cargo vischeck --manifest-path path/to/Cargo.toml
```

Behavior:

- run at a workspace root: audit all workspace members
- run in a member crate directory: audit just that package
- pass `--manifest-path` to choose an explicit crate/workspace root

## Intended workflow

Use this as a migration aid and CI guard:

1. fail immediately on forbidden visibility forms
2. review suspicious bare `pub`
3. compare heuristic findings against manual review
4. keep repo-specific exceptions small and explicit
