# berth-ceiling

> **Production: remove-running-section** — unit `berth-ceiling-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-14 14:44 EDT

cargo-berth: GIT_CEILING_DIRECTORIES entries are not canonicalized, so on macOS a ceiling under /var never matches the canonical /private/var walk (src/ledger/worktree_context.rs discover_with_ceilings canonicalizes only the invocation directory). Canonicalize each ceiling as git does, keep entries that cannot be canonicalized as given, and make the ledger::worktree_context ceiling tests pass on the Mac (ssh mac) as well as natedev. Small fix: one phase, fast; then cargo-berth tests can be offloaded to the Mac.
