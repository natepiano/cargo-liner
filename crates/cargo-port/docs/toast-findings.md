# Toast Plan Findings

This review covers the updated toast plan after the latest amendments were folded into
`docs/tui-pane-lib.md` and `docs/core-api.md`. The major architectural direction is now in good
shape: toasts are framework-owned, overlay/focus ids are split, `ListNavigation` decouples framework
toasts from the binary's `NavigationAction`, and `ToastSettings` is intended to have one mutable
owner.

## Findings

1. **`ListNavigation` exists, but the `Navigation` trait still cannot produce it.**

   The toast API now correctly takes a framework-owned navigation enum:

   ```rust
   pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome;
   ```

   See `docs/tui-pane-lib.md:2240` and `docs/core-api.md:932`.

   However, the current `Navigation` trait shape still only exposes `UP`, `DOWN`, `LEFT`, and
   `RIGHT` constants (`docs/tui-pane-lib.md:284`, `docs/core-api.md:415`). The plan text says the
   dispatcher translates via `up()` / `down()` / `home()` / `end()` accessors, but those accessors do
   not exist, and the trait does not currently require `HOME` or `END`.

   Recommendation: add one explicit trait-level translation method:

   ```rust
   fn list_navigation(action: Self::Actions) -> Option<ListNavigation>;
   ```

   cargo-port maps `Up`, `Down`, `Home`, and `End`; unsupported actions such as `Left`, `Right`,
   `PageUp`, and `PageDown` return `None`. This keeps unsupported toast movement unrepresentable
   while avoiding a framework dependency on the app's concrete `NavigationAction`.

2. **Toast settings ownership is now correct, but `core-api.md` has a stale `get_mut` comment.**

   The updated plan makes `Framework` the sole mutable owner of `ToastSettings` and changes the
   app binding to `load` / `save`, which is the right model (`docs/tui-pane-lib.md:2790`).

   One stale comment remains in `docs/core-api.md:823`: it still describes `toast_settings_mut()` as
   the path that the binding's `get_mut` re-clones into. That no longer matches the `load` / `save`
   model.

   Recommendation: update that comment to say the settings pane mutates framework-owned settings
   directly, and dispatch calls the optional binding `save(ctx, framework.toast_settings())` after
   the framework borrow ends.

3. **Phase wording around `toast_settings` defaulting is still slightly contradictory.**

   `docs/tui-pane-lib.md:2790` says the `toast_settings` field is added in Phase 21, but also says
   it is "defaulted at `Framework::new` in Phase 11". Since Phase 11 is already complete in the plan
   sequence, that phrasing reads as if Phase 11 needs to know about a Phase 21 field.

   Recommendation: rephrase to: "Phase 21 adds the field and updates `Framework::new` to initialize
   it with `ToastSettings::default()` for apps that do not register a binding." That preserves the
   compatibility intent without implying a retroactive Phase 11 change.

4. **The public surface checklist is missing `ListNavigation`.**

   Phase 12 says `ListNavigation` is re-exported from the crate root (`docs/tui-pane-lib.md:2390`),
   but the Definition of Done public-surface list does not include it (`docs/tui-pane-lib.md:2998`).

   Recommendation: add `ListNavigation` to the final exported-type checklist so implementation and
   docs stay aligned.

## Bottom line

The remaining architectural blocker is the first finding: the plan needs a concrete, trait-level
way to translate app navigation actions into framework `ListNavigation`. The settings model and
toast action row issues from the previous review are now largely resolved; the remaining work there
is doc cleanup and checklist sync.
