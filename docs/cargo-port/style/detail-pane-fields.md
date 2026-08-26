# Detail-pane fields store the value, not a placeholder

Fields on the detail-pane structs (`PackageData`, `GitData`) that may not be loaded yet — type, version, disk — are `Option<T>` or a typed enum, never a `String` pre-filled with `""` / `"-"` / `"0 B"`. A pre-filled `String` can't tell "not loaded" from "loaded but empty".

Choose the placeholder once, at the render getter (`DetailField::package_value`); mirror `or_dash` and the `Option<u64>` disk fields.

```rust
// bad
pub types: String,                   // "" = pending, or no targets? unknowable
// good — None = pending, Some([]) = no lib/bin target
pub types: Option<Vec<ProjectType>>,
```
