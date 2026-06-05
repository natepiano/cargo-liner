use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use super::offsets;
use super::scope;
use super::scope::ScopeInfo;
use super::visitor::InlinePathOccurrence;
use crate::config::DiagnosticCode;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

pub(super) struct OccurrenceContext<'a> {
    pub(super) path:            &'a Path,
    pub(super) display_path:    &'a str,
    pub(super) text:            &'a str,
    pub(super) offsets:         &'a [usize],
    pub(super) scopes:          &'a [ScopeInfo],
    pub(super) collision_names: &'a BTreeSet<String>,
}

pub(super) fn process_occurrence(
    occ: &InlinePathOccurrence,
    ctx: &OccurrenceContext<'_>,
    inserted_use_paths: &mut BTreeSet<(usize, String)>,
    findings: &mut Vec<Finding>,
    fixes: &mut Vec<UseFix>,
) {
    if ctx.collision_names.contains(&occ.import_name) {
        return;
    }

    // Importing a name that already has prelude meaning (`Result`, `Option`,
    // `Vec`, ...) at the top of a file silently changes what every future
    // bare reference to that name resolves to. Even when nothing in the file
    // currently writes bare `Result<T, E>`, adding `use io::Result;` is a
    // correctness footgun the moment anyone edits the file. Skip these
    // outright rather than rely on shadow detection.
    if shadows_prelude(&occ.import_name) {
        return;
    }

    let byte_start = offsets::offset(ctx.text, ctx.offsets, occ.span_start);
    let byte_end = offsets::offset(ctx.text, ctx.offsets, occ.span_end);

    let scope_id = scope::find_innermost_scope(ctx.scopes, byte_start);
    let scope = scope_id.map(|id| &ctx.scopes[id]);

    // Resolve a partial path (`fmt::Display` written because `use std::fmt;`
    // is in scope) to its absolute form (`std::fmt::Display`) so the inserted
    // `use` is self-contained — i.e. it doesn't silently break if the parent
    // module import is later removed or reordered.
    let import_path = scope.map_or_else(
        || occ.import_path.clone(),
        |scope| absolutize_import_path(&occ.import_path, &scope.existing_imports),
    );

    let source_line = ctx
        .text
        .lines()
        .nth(occ.span_start.line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    findings.push(Finding {
        severity: Severity::Warning,
        diagnostic_code: DiagnosticCode::InlinePathQualifiedType,
        path: ctx.display_path.to_string(),
        line: occ.span_start.line,
        column: occ.span_start.column + 1,
        highlight_len: occ.full_path.len().max(1),
        source_line,
        item: None,
        message: format!(
            "use a `use` import for `{}` instead of inline path",
            occ.import_name
        ),
        suggestion: Some(format!("consider adding: `use {import_path};`")),
        fix_support: FixSupport::InlinePathQualifiedType,
        related: None,
    });

    // Group the rewrite and its companion `use` insertion so the combining
    // layer can drop them together on cross-pass name collisions.
    let group = Some(ImportGroup {
        bare_name: occ.import_name.clone(),
        full_path: import_path.clone(),
    });

    fixes.push(UseFix {
        path:         ctx.path.to_path_buf(),
        start:        byte_start,
        end:          byte_end,
        replacement:  occ.replacement.clone(),
        import_group: group.clone(),
    });

    let Some(scope_id) = scope_id else {
        return;
    };
    let scope = &ctx.scopes[scope_id];

    if !scope.existing_imports.contains(&import_path)
        && !scope.existing_reexport_names.contains(&occ.import_name)
        && inserted_use_paths.insert((scope_id, import_path.clone()))
    {
        let use_path = scope::canonicalize_inserted_use_path(scope, &import_path);
        let use_text = format!("{}use {use_path};\n", scope.indent);
        fixes.push(UseFix {
            path:         ctx.path.to_path_buf(),
            start:        scope.insertion_offset,
            end:          scope.insertion_offset,
            replacement:  use_text,
            import_group: group,
        });
    }
}

/// Names with prelude meaning. Importing any of these from a non-prelude
/// path silently shadows the prelude binding for the rest of the file —
/// `use io::Result;` makes future `Result<T, E>` references resolve to the
/// `std::io::Result<T>` type alias instead of the generic prelude `Result`.
/// Conservative list: prelude types and the most commonly-derived prelude
/// traits, all from std prelude v1 / 2021 / 2024.
fn shadows_prelude(name: &str) -> bool {
    matches!(
        name,
        "Box"
            | "Option"
            | "Result"
            | "String"
            | "Vec"
            | "Clone"
            | "Copy"
            | "Debug"
            | "Default"
            | "Drop"
            | "Eq"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "Hash"
            | "Into"
            | "IntoIterator"
            | "Iterator"
            | "PartialEq"
            | "PartialOrd"
            | "Send"
            | "Sized"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
    )
}

/// Resolve a partial path like `fmt::Display` against the file's existing
/// imports. If `use std::fmt;` is already in scope, `fmt::Display` becomes
/// `std::fmt::Display`. The returned import is self-contained — it doesn't
/// rely on a sibling module import staying in place.
fn absolutize_import_path(import_path: &str, existing_imports: &BTreeSet<String>) -> String {
    let Some((leading, rest)) = import_path.split_once("::") else {
        return import_path.to_string();
    };
    if leading == "crate" || leading == "super" || leading == "self" {
        return import_path.to_string();
    }
    // Look for an existing `use a::b::<leading>;` (i.e. an import whose final
    // segment matches `leading` and which has at least one parent segment).
    // Without a parent segment, the existing import is itself a top-level
    // crate name — already absolute.
    for existing in existing_imports {
        let Some((parent, last)) = existing.rsplit_once("::") else {
            continue;
        };
        if last == leading {
            return format!("{parent}::{leading}::{rest}");
        }
    }
    import_path.to_string()
}

/// Finds type names that cannot be safely imported because they either:
/// - map to multiple distinct paths (ambiguous), or
/// - are already used bare in the file (importing would shadow the existing usage, e.g. prelude
///   `Result<T, E>` shadowed by `use crate::error::Result;`).
pub(super) fn find_collision_names(
    occurrences: &[InlinePathOccurrence],
    bare_type_names: &BTreeSet<String>,
    existing_imports: &BTreeSet<String>,
) -> BTreeSet<String> {
    // Group by the name that will be brought into scope by the `use` (the
    // `import_name`), and track the set of distinct import paths per name.
    // If more than one distinct path maps to the same import name, the
    // imports would collide — skip them all.
    let mut name_to_paths: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for occ in occurrences {
        name_to_paths
            .entry(&occ.import_name)
            .or_default()
            .insert(&occ.import_path);
    }

    let mut collisions = BTreeSet::new();
    for (name, paths) in &name_to_paths {
        let ambiguous = paths.len() > 1;
        // If the name is already used bare somewhere in the file (e.g.
        // `use super::*` brings in a struct `Package`), introducing a new
        // `use crate::other::Package;` would shadow it.
        let would_shadow =
            bare_type_names.contains(*name) && !paths.iter().all(|p| existing_imports.contains(*p));
        if ambiguous || would_shadow {
            collisions.insert((*name).to_owned());
        }
    }
    collisions
}
