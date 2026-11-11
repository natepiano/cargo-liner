# cargo-port next items

The [current contracts](as-built/overview.md) describe shipped behavior.
These items remain proposed follow-ups.

## Import-condition style

1. Remove the forbidden word from the comment in `crates/cargo-mend/src/fixes/imports/module_imports.rs`.
2. Replace `ImportCondition::from_attributes` with `From<&[syn::Attribute]>` and update its callers.
3. Preserve behavior and validate cargo-mend after either change.

## Retained executable follow-ups

The [compiler contract](../cargo-mend/as-built/compiler-validation.md) describes wrapper ownership.

1. Remove the copied staging-path rule from `failed_wrapper_copy_keeps_the_published_executable` in `compiler/build/execute.rs`.
2. List the alias parent's entries and propagate listing errors instead.
3. Preserve the copy-failure and alias-content assertions.

Help, version, and build-info still open the running executable before their early returns.
A future change can move that open after argument parsing and the build-info return, but before metadata resolution.
Regression tests must retain the wrapper replacement and removal guarantees.

## Live captures during Git loading

A capture can contain the frame before Git data arrives.
The capture fixture should permit delaying the probe and saving that first-paint frame.
Keep the existing loading states and cached-data behavior explicit in the fixture.
See [Git pane data](as-built/git-pane.md) and [rendering](as-built/rendering.md).
