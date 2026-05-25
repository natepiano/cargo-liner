## Narrowing must follow every reachability indirection

`unused_pub`, `narrow_to_pub_crate`, and `suspicious_pub` all answer one
question: is a `pub` item reachable from outside its module/subtree? A narrowing
fix that makes a *reachable* item more private breaks the build (E0446
private-in-public, or `private_interfaces`) and rolls the whole `--fix` batch
back. Every such bug is the same mistake: reachability missed an indirection
between the use site and the item.

Indirections that count as reachability — each has a regression test; keep them:

- **Re-exports** (`pub use` / `pub(crate) use`) —
  `narrow_pub_crate::re_exported_item_is_not_flagged`
- **Enum variant payloads** of a reachable enum —
  `narrow_pub_crate::type_reachable_via_reexported_enum_variant_is_not_flagged`
- **`cfg(test)` callers** (lib compilation strips them) —
  `narrow_pub_crate::fix_does_not_narrow_pub_fn_used_only_from_cfg_test_caller`
- **Macro-wrapped callers** (paths live in token streams) —
  `narrow_pub_crate::fix_does_not_narrow_pub_fn_called_only_from_cfg_test_assert_macro`
- **Type aliases** — a type named only inside `type Alias = Wrapper<Inner>` is
  reachable wherever `Alias` is used —
  `unused_pub::type_reachable_only_through_pub_crate_alias_is_not_flagged_unused`
- **Public field graphs** — a `pub` field of a reachable type transitively
  exposes the field's type — same test

### Rule

When you add or change visibility-narrowing logic:

1. Reachability is transitive through every indirection above, not just direct
   path references. The HIR use-site collector
   (`src/compiler/visibility/use_sites.rs`) is the single place these resolve —
   extend it there, not in per-item source scans.
2. Stop at a real visibility boundary: a module-private field caps its type's
   reach, so don't follow it. Confirm narrowing still fires for a control type
   reachable only through that boundary.
3. Every regression test needs both halves — the reachable item is NOT flagged
   **and** a genuinely-unused sibling (or one reachable only through a private
   boundary) IS still flagged. A suppression-only test can't catch
   over-suppression.
