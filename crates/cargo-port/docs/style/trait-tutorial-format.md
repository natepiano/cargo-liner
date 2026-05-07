# Trait tutorial format

When walking through a Rust trait as a tutorial — explaining its surface to a
reader for the first time — present the trait's items in a 4-column markdown
table. One trait per table.

## Columns

| Column | Content |
|---|---|
| **Item** | Full signature: `type Foo: Bound`, `const NAME: &'static str`, `fn name(...) -> ReturnType`. Include the bound on associated types and the return type on methods. |
| **Description** | One short sentence on what the item is for. |
| **Default implementation?** | `Yes` (with a brief note on what the default is) or `No`. |
| **bin / lib** | Who supplies the value: `bin` (always app-supplied), `bin override only when ...` (lib provides default; binary overrides in named cases), `lib` (framework-only, app never touches), etc. |

## Section layout

1. `# TraitName<Ctx>` heading — name the trait and its main type parameter.
2. One short paragraph (1–3 sentences) on the trait's role: what kind of impl
   it expects (one per app, one per pane type, etc.) and what the framework
   uses it for.
3. The 4-column table above.
4. Any trait-level bound that didn't fit in the table (e.g.
   `Shortcuts<Ctx>: 'static`) gets a one-line callout below the table with the
   reason.
5. End with a short prompt offering the next type in the series
   (`Continue to NextTrait?`). Do not chain multiple traits in one turn.

## Why

The reader can scan the trait's surface in one block — required vs optional,
who fills in each item, and what each item is for — without prose paragraphs
that re-state the same facts. The `bin / lib` column is the one the reader
returns to most: it tells them which items they own as the app author and
which the framework already handles.

## Example

```markdown
# `Globals<Ctx>` — app-wide global scope

One impl per app. Holds globals like Quit, Find, Rescan that fire regardless
of focus.

| Item | Description | Default implementation? | bin / lib |
|---|---|---|---|
| `type Variant: Action` | The app's global action enum. | No | bin |
| `const SCOPE_NAME: &'static str` | TOML table name. | Yes — `"global"` | bin override rarely |
| `fn render_order() -> &'static [Self::Variant]` | Bar render order for the Global region. | No | bin |
| `fn defaults() -> Bindings<Self::Variant>` | Default keybindings for app globals. | No | bin |
| `fn dispatcher() -> fn(Self::Variant, &mut Ctx)` | Framework calls this on every app-global action. | No | bin |
```
