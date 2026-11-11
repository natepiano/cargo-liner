# Safe import and visibility fixes

cargo-mend plans import and visibility edits against syntax, module identity, and compiler evidence.
An automatic edit must remain valid for every observed mount of a shared source file.

Paths below are relative to `crates/cargo-mend/src/`.

## Module identity and shared sources

`rust_syntax/module_map.rs` follows module declarations, literal `include!` paths, and `#[path]` mounts.
It recognizes raw identifier spellings and declarations inside supported nested syntax.
`ModuleMap` derives module ancestry from these mounts rather than the physical folder hierarchy.
Plain `mod` declarations and top-level `#[path]` attributes can resolve relative to different directories.
The existing sibling module file takes precedence where the syntax permits it.

`FileModulePath::{Known, SeveralParents}` records whether one relative module path is unambiguous.
A file with several parents cannot safely receive a rewrite that assumes one meaning for `super`.
Unknown package reach, including an unresolved nonliteral include, prevents an unsafe shared-source edit.
Renamed custom macros and attribute or derive expansion lie outside this syntax-only mount guarantee.

`compiler/persistence/visibility_constraint.rs` defines `SourceModuleMount` and visibility agreement.
A mount combines a package root with a module path.
Library, binary, and test compilations of the same module are evidence for one mount.
An automatic annotation change requires agreement across all relevant mounts.
Missing, unknown, silent, or conflicting proposals keep the current visibility.
Child declarations changed by a parent facade edit receive the same agreement checks.
`PubUseNarrowingSite` in `compiler/persistence/schema.rs` identifies these affected sites.
Manual diagnostics remain available once per finding.
`FINDINGS_SCHEMA_VERSION` in `compiler/constants.rs` rejects incompatible stored report shapes.

`fixes/subtree_reexport/scan.rs` applies the same cross-package agreement to re-export edits.
Source membership distinguishes nested packages; a directory-prefix test does not.
The planner preserves module declarations even when their rewritten file becomes empty.

## One import owner

`fixes/imports/module_imports.rs` defines `ModuleImports`.
Prefer-module-import, imports-at-top, inline path, and facade edits use that shared owner.
It identifies a binding by scope, resolved target, and name.
Equal syntactic paths to the same module therefore do not create duplicate imports.

Public and cfg-gated re-exports retain their coverage.
A private import covers only configurations the existing binding does not already cover.
`ImportCondition` represents those conditions.
A destination glob that can provide the binding or trait prevents redundant import relocation.

A declined import edit returns to the subtree re-export fact-exclusion loop.
The planner repeats without that finding and keeps the affected re-export.
Independent valid fixes remain eligible.
Filtering duplicate lines after rendering would miss these dependencies.

`LocalImportRelation::for_paths` requires a nonempty descendant suffix.
A self-module import is therefore not a descendant with an empty path.
This prevents an empty `use ;` edit.

## Protected uses and configuration gates

`fixes/prefer_module_import/scan.rs` preserves imports whose uses cannot accept a qualified replacement.
These include macro bindings, possible macro field names, field shorthand, and descendant-module access.
Lexical scopes identify shorthand bindings.
`descendant_imports.rs` records named imports, globs, qualified paths, and supported macro-token paths below the parent.
A child using `super::name` therefore keeps the parent binding it needs.
The macro-token walker treats ambiguous field uses conservatively.

`fixes/imports/gate_tracking.rs` supplies `gate_tracking_visit!`.
It collects enclosing cfg attributes from items, parameters, fields, generics, patterns, and attribute-bearing expressions.
Both imports-at-top and inline-path fixes use it.
Adding a cfg-bearing syntax node requires coverage in this tracker.
Otherwise moving its import can change which platforms compile it.

`fixes/inline_path_qualified_type/visitor.rs` includes a leading `::` in an absolute path's replacement span.
The replacement ends at the last identifier and leaves generic arguments intact.
`process.rs` preserves external-crate identity when resolving the inserted import.
A local alias must not capture the absolute external name.
Diagnostic highlighting uses the replacement's byte span.

## Function-length protection

`fixes/runner/function_length/` measures functions changed by a proposed fix.
It formats proposed text with workspace rustfmt settings when the measurement requires it.
Body counting excludes blank lines and comment lines as clippy does.
The threshold comes from `clippy.toml`, or the clippy default.
Formatter detection is cached, and short functions avoid unnecessary formatter processes.

A function that crosses the threshold retains its original names and function-scope imports.
Other functions can retain their independent rewrites.
A function already over the limit keeps its existing eligibility.
An explicit allow or expect for the lint also keeps that eligibility.
The notice names the function and measured count without making the run fail.
A second run remains stable.
The tool does not split a function automatically.
A relocation pass that depends on a protected function stays out as one unit.

## Invariants and rationale

1. Resolve module identity from source mounts.
2. Require shared-source agreement before automatic visibility edits.
3. Route import changes through `ModuleImports`.
4. Preserve enclosing cfg conditions and unqualified-only uses.
5. Preserve absolute-path identity and generic arguments.
6. Reject an import-dependent plan as a whole when its binding cannot be written safely.
7. Keep valid independent fixes when another edit is declined.

These constraints prioritize compilable edits over the number of automatic rewrites.
See [compiler validation](compiler-validation.md) for build checks, rollback, and wrapper lifetime.
