# cargo-liner-port — next items

Backlog the showrunner approved. Not started; each becomes a phase when the showrunner says so.

## cargo-mend style items (showrunner, 2026-11-02)

1. **A comment in mend's module import fixer uses a forbidden word.**
   Target: `crates/cargo-mend/src/fixes/imports/module_imports.rs:166`.
   Done when: the line uses no word from `~/rust/nate_style/rust/forbidden-words.md`, and the meaning stays.
2. **Replace `ImportCondition::from_attributes` with the `From` impl the style guide requires.**
   Target: `crates/cargo-mend/src/fixes/imports/import_condition.rs:24` and its callers.
   Done when: `From<&[syn::Attribute]>` replaces the named conversion, behavior stays the same, and `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` pass.

## cargo-mend wrapper follow-ups (phase 2p review, 2026-11-02)

Phase 2p reached its three repair rounds and landed. These two review findings are the top of the next cargo-mend phase.

1. **The copy-failure test repeats the staging path rule (F004).**
   Target: `failed_wrapper_copy_keeps_the_published_executable` in `crates/cargo-mend/src/compiler/build/execute.rs`. It rebuilds the staging path with `WRAPPER_ALIAS_STAGING_EXTENSION` and the process id, a copy of the rule in `StagedWrapperCopy::create`.
   Done when: the test reads the entries of `alias.parent()` into `io::Result<Vec<_>>`, propagates a listing error, and asserts one entry. It keeps the copy-failure and alias-content assertions, and it does not rebuild the staging path (Astra review 2026-11-02).
2. **Help, version and build-info work without read access to the executable (F005).**
   Target: `crates/cargo-mend/src/main.rs` (`RunningExecutable::open` at the first line of `run()`) and its startup regression tests.
   Done when: the executable opens after argument parsing and the build-info return, and before cargo metadata resolution. Regression tests show that `--help`, `--version` and `--build-info` succeed when the executable cannot be opened. The phase 2p replacement and removal tests stay, and `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` pass.

## Live capture follow-up (phase 4o design check, 2026-11-09)

The showrunner placed this item in the next file on 2026-11-09.

1. **A live capture can show the frame before git data arrives.**
   Target: the cargo-port live capture script used for design checks, and the git probe start in `crates/cargo-port/src/tui/`.
   Need: the first frame already draws Sync and Main values from cached git data, so no capture shows the loading layout.
   Done when: a capture run can delay the git probe and save a first-paint frame with no Sync or Main values.
