## Style guide tooling lines for new diagnostics

Every cargo-mend diagnostic must be connected to a shared style doc in `~/rust/nate_style/rust/`.

### When adding a new diagnostic

1. Search `~/rust/nate_style/rust/*.md` for an existing doc that describes the rule.
2. If found, append or update the `**Tooling:**` line at the end.
3. If none exists, create one via `/rust_style` with the rule and the `**Tooling:**` line.

### Format

```
**Tooling:** `cargo mend` detects this as `diagnostic_code` (severity). Run `cargo mend --fix` to auto-fix.
```

Omit the fix sentence if not auto-fixable. Use the specific flag if it differs (e.g., `--fix-pub-use`).
