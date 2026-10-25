use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use super::offsets;
use super::scope;
use super::scope::ScopeInfo;
use super::visitor::InlinePathOccurrence;
use crate::config::DiagnosticCode;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;
use crate::rust_syntax::PathAnchor;

pub(super) struct OccurrenceContext<'a> {
    pub(super) path:            &'a Path,
    pub(super) display_path:    &'a str,
    pub(super) text:            &'a str,
    pub(super) offsets:         &'a [usize],
    pub(super) scopes:          &'a [ScopeInfo],
    pub(super) collision_names: &'a BTreeSet<String>,
}

type ImportKey = (usize, String);

pub(super) struct ImportAttributePlan {
    attributes: BTreeMap<ImportKey, ConditionalAttributes>,
}

impl ImportAttributePlan {
    fn get(&self, key: &ImportKey) -> Option<&ConditionalAttributes> { self.attributes.get(key) }
}

pub(super) struct ResolvedOccurrence {
    byte_start:             usize,
    byte_end:               usize,
    scope_id:               Option<usize>,
    pub(super) import_path: String,
    conditional_attributes: ConditionalAttributes,
}

enum OccurrenceScope<'a> {
    Scoped {
        id:         usize,
        scope:      &'a ScopeInfo,
        attributes: &'a ConditionalAttributes,
    },
    Unscoped,
}

enum ImportResolution {
    Resolved {
        path:       String,
        attributes: ConditionalAttributes,
    },
    Unresolved {
        spelling: String,
    },
}

impl ImportResolution {
    fn into_parts(self) -> (String, ConditionalAttributes) {
        match self {
            Self::Resolved { path, attributes } => (path, attributes),
            Self::Unresolved { spelling } => (spelling, ConditionalAttributes::default()),
        }
    }
}

pub(super) fn plan_import_attributes(
    occurrences: &[InlinePathOccurrence],
    ctx: &OccurrenceContext<'_>,
) -> ImportAttributePlan {
    let mut grouped: BTreeMap<ImportKey, BTreeSet<ConditionalAttributes>> = BTreeMap::new();
    for occurrence in occurrences {
        let resolved = resolve_occurrence(occurrence, ctx);
        let Some(scope_id) = resolved.scope_id else {
            continue;
        };
        grouped
            .entry((scope_id, resolved.import_path))
            .or_default()
            .insert(resolved.conditional_attributes);
    }

    let mut attributes = BTreeMap::new();
    for (key, candidates) in grouped {
        if let Some(covering) = ConditionalAttributes::covering(&candidates) {
            attributes.insert(key, covering);
        }
    }
    ImportAttributePlan { attributes }
}

pub(super) fn process_occurrence(
    occ: &InlinePathOccurrence,
    ctx: &OccurrenceContext<'_>,
    import_attribute_plan: &ImportAttributePlan,
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

    let resolved = resolve_occurrence(occ, ctx);
    let import_path = &resolved.import_path;
    let occurrence_scope = match resolved.scope_id {
        Some(id) => {
            let Some(attributes) = import_attribute_plan.get(&(id, import_path.clone())) else {
                return;
            };
            OccurrenceScope::Scoped {
                id,
                scope: &ctx.scopes[id],
                attributes,
            }
        },
        None => OccurrenceScope::Unscoped,
    };

    match occurrence_scope {
        OccurrenceScope::Unscoped => {
            record_occurrence(occ, ctx, &resolved, findings, fixes);
        },
        OccurrenceScope::Scoped {
            id,
            scope,
            attributes,
        } => {
            if scope.has_private_import_conflict(&occ.import_name, import_path) {
                return;
            }
            // A gated binding must cover the occurrence before it can replace the path.
            if scope.existing_binding_gated_out(
                import_path,
                &occ.import_name,
                &resolved.conditional_attributes,
            ) {
                return;
            }
            let group = record_occurrence(occ, ctx, &resolved, findings, fixes);
            if !scope.existing_imports.contains_key(import_path)
                && !scope.existing_reexport_names.contains_key(&occ.import_name)
                && inserted_use_paths.insert((id, import_path.clone()))
            {
                let use_path = scope::canonicalize_inserted_use_path(scope, import_path);
                let attributes = attributes.render(&scope.indent);
                let use_text = format!("{}{}use {use_path};\n", attributes, scope.indent);
                fixes.push(UseFix {
                    path:         ctx.path.to_path_buf(),
                    start:        scope.insertion_offset,
                    end:          scope.insertion_offset,
                    replacement:  use_text,
                    import_group: Some(group),
                });
            }
        },
    }
}

fn record_occurrence(
    occ: &InlinePathOccurrence,
    ctx: &OccurrenceContext<'_>,
    resolved: &ResolvedOccurrence,
    findings: &mut Vec<Finding>,
    fixes: &mut Vec<UseFix>,
) -> ImportGroup {
    let import_path = &resolved.import_path;

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
        item_visibility: ItemVisibility::default(),
    });

    // `ImportGroup` groups the rewrite and its companion `use` insertion so the combining
    // layer can drop them together on cross-pass name collisions.
    let group = ImportGroup {
        bare_name: occ.import_name.clone(),
        full_path: import_path.clone(),
    };

    fixes.push(UseFix {
        path:         ctx.path.to_path_buf(),
        start:        resolved.byte_start,
        end:          resolved.byte_end,
        replacement:  occ.replacement.clone(),
        import_group: Some(group.clone()),
    });

    group
}

pub(super) fn resolve_occurrence(
    occurrence: &InlinePathOccurrence,
    ctx: &OccurrenceContext<'_>,
) -> ResolvedOccurrence {
    let byte_start = offsets::offset(ctx.text, ctx.offsets, occurrence.span_start);
    let byte_end = offsets::offset(ctx.text, ctx.offsets, occurrence.span_end);
    let scope_id = scope::find_innermost_scope(ctx.scopes, byte_start);
    let scope = scope_id.map(|id| &ctx.scopes[id]);

    // `absolutize_import_path` resolves a partial path (`fmt::Display` written because `use
    // std::fmt;` is in scope) to its absolute form (`std::fmt::Display`) so the inserted
    // `use` does not depend on a sibling module import remaining in place.
    let import_path = scope.map_or_else(
        || ImportResolution::Unresolved {
            spelling: occurrence.import_path.clone(),
        },
        |scope| absolutize_import_path(&occurrence.import_path, &scope.existing_imports),
    );
    // `absolutize_through_ancestors` handles a nested module seeing the leading segment only
    // through a glob (`use super::*;`), so its own scope cannot absolutize. Resolve through
    // ancestor imports before falling back to a `super::`-qualified path:
    // every occurrence of `io::ErrorKind` then agrees on `std::io::ErrorKind`
    // instead of the nested scope alone producing `super::io::ErrorKind` —
    // a same-name/different-path group the combining layer would drop whole.
    let import_path = match import_path {
        ImportResolution::Unresolved { spelling } => {
            absolutize_through_ancestors(ctx.scopes, scope_id, &spelling)
        },
        resolved @ ImportResolution::Resolved { .. } => resolved,
    };
    let import_path = match import_path {
        ImportResolution::Unresolved { spelling } => {
            match scope_id
                .and_then(|id| scope::qualify_through_parent_scope(ctx.scopes, id, &spelling))
            {
                Some(parent_import) => ImportResolution::Resolved {
                    path:       parent_import.path,
                    attributes: parent_import.conditional_attributes,
                },
                None => ImportResolution::Unresolved { spelling },
            }
        },
        resolved @ ImportResolution::Resolved { .. } => resolved,
    };
    let (import_path, mut conditional_attributes) = import_path.into_parts();
    conditional_attributes.extend(occurrence.conditional_attributes.clone());

    ResolvedOccurrence {
        byte_start,
        byte_end,
        scope_id,
        import_path,
        conditional_attributes,
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

/// Resolve a partial path against ancestor scopes' imports, nearest ancestor
/// first, mirroring how the name shadows outward at the occurrence site. The
/// produced import is absolute, so it does not depend on the ancestor's
/// import staying in place.
fn absolutize_through_ancestors(
    scopes: &[ScopeInfo],
    scope_id: Option<usize>,
    import_path: &str,
) -> ImportResolution {
    let Some(scope_id) = scope_id else {
        return ImportResolution::Unresolved {
            spelling: import_path.to_string(),
        };
    };
    let current = &scopes[scope_id];
    let mut ancestors: Vec<&ScopeInfo> = scopes
        .iter()
        .filter(|scope| {
            scope.module_path.len() < current.module_path.len()
                && current.module_path.starts_with(&scope.module_path)
        })
        .collect();
    ancestors.sort_by_key(|scope| Reverse(scope.module_path.len()));
    for ancestor in ancestors {
        let candidate = absolutize_import_path(import_path, &ancestor.existing_imports);
        match candidate {
            resolved @ ImportResolution::Resolved { .. } => return resolved,
            ImportResolution::Unresolved { .. } => {},
        }
    }
    ImportResolution::Unresolved {
        spelling: import_path.to_string(),
    }
}

/// Resolve a partial path like `fmt::Display` against the file's existing
/// imports. If `use std::fmt;` is already in scope, `fmt::Display` becomes
/// `std::fmt::Display`. The returned import is self-contained — it doesn't
/// rely on a sibling module import staying in place.
fn absolutize_import_path(
    import_path: &str,
    existing_imports: &BTreeMap<String, BTreeSet<ConditionalAttributes>>,
) -> ImportResolution {
    let Some((leading, rest)) = import_path.split_once("::") else {
        return ImportResolution::Unresolved {
            spelling: import_path.to_string(),
        };
    };
    match PathAnchor::from(leading) {
        PathAnchor::Crate | PathAnchor::Super | PathAnchor::SelfMod => {
            return ImportResolution::Unresolved {
                spelling: import_path.to_string(),
            };
        },
        PathAnchor::SelfType | PathAnchor::Name => {},
    }
    // `absolutize_import_path` searches `existing_imports` for `use a::b::<leading>;` (i.e. an
    // import whose final segment matches `leading` and which has at least one parent segment).
    // Without a parent segment, the existing import is itself a top-level
    // crate name — already absolute.
    for existing in existing_imports.keys() {
        let Some((parent, last)) = existing.rsplit_once("::") else {
            continue;
        };
        if last == leading {
            return ImportResolution::Resolved {
                path:       format!("{parent}::{leading}::{rest}"),
                attributes: ConditionalAttributes::default(),
            };
        }
    }
    ImportResolution::Unresolved {
        spelling: import_path.to_string(),
    }
}

/// Finds type names that cannot be safely imported because they either:
/// - map to multiple distinct paths (ambiguous), or
/// - are already used bare in the file (importing would shadow the existing usage, e.g. prelude
///   `Result<T, E>` shadowed by `use crate::error::Result;`).
///
/// Compares the resolved import paths, not the paths as written: two scopes
/// writing the same `io::ErrorKind` can still resolve through different `io`
/// bindings, and detecting that here keeps the findings out of the report
/// instead of advertising fixes the combining layer would silently drop.
pub(super) fn find_collision_names(
    occurrences: &[InlinePathOccurrence],
    resolved_import_paths: &[String],
    bare_type_names: &BTreeSet<String>,
    existing_imports: &BTreeSet<String>,
) -> BTreeSet<String> {
    // `name_to_paths` groups by the name that will be brought into scope by the `use` (the
    // `import_name`), and tracks the set of distinct import paths per name.
    // If more than one distinct path maps to the same import name, the
    // imports would collide — skip them all.
    let mut name_to_paths: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (occ, resolved) in occurrences.iter().zip(resolved_import_paths) {
        name_to_paths
            .entry(&occ.import_name)
            .or_default()
            .insert(resolved.as_str());
    }

    let mut collisions = BTreeSet::new();
    for (name, paths) in &name_to_paths {
        let ambiguous = paths.len() > 1;
        // `would_shadow` tests `bare_type_names` because if the name is already used bare somewhere
        // in the file (e.g. `use super::*` brings in a struct `Package`), introducing a new
        // `use crate::other::Package;` would shadow it.
        let would_shadow =
            bare_type_names.contains(*name) && !paths.iter().all(|p| existing_imports.contains(*p));
        if ambiguous || would_shadow {
            collisions.insert((*name).to_owned());
        }
    }
    collisions
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::path::Path;

    use proc_macro2::LineColumn;

    use super::ImportAttributePlan;
    use super::ImportResolution;
    use super::OccurrenceContext;
    use super::absolutize_import_path;
    use super::process_occurrence;
    use crate::fixes::imports::ConditionalAttributes;
    use crate::fixes::inline_path_qualified_type::visitor::InlinePathOccurrence;

    #[test]
    fn resolution_preserves_explicit_anchors_and_expands_imported_modules() {
        let imports = BTreeMap::from([("std::io".to_string(), BTreeSet::new())]);
        for (path, expected) in [
            ("io::ErrorKind", "std::io::ErrorKind"),
            ("crate::io::ErrorKind", "crate::io::ErrorKind"),
            ("super::io::ErrorKind", "super::io::ErrorKind"),
            ("self::io::ErrorKind", "self::io::ErrorKind"),
            ("unknown::Widget", "unknown::Widget"),
        ] {
            let resolution = absolutize_import_path(path, &imports);
            assert_eq!(
                matches!(&resolution, ImportResolution::Resolved { .. }),
                path == "io::ErrorKind"
            );
            assert_eq!(resolution.into_parts().0, expected);
        }
    }

    #[test]
    fn an_unscoped_occurrence_rewrites_without_inserting_an_import() {
        let text = "ext::Widget";
        let occurrence = InlinePathOccurrence {
            full_path:              text.to_string(),
            import_path:            text.to_string(),
            import_name:            "Widget".to_string(),
            replacement:            "Widget".to_string(),
            span_start:             LineColumn {
                line:   1,
                column: 0,
            },
            span_end:               LineColumn {
                line:   1,
                column: text.len(),
            },
            conditional_attributes: ConditionalAttributes::default(),
        };
        let ctx = OccurrenceContext {
            path: Path::new("src/lib.rs"),
            display_path: "src/lib.rs",
            text,
            offsets: &[0],
            scopes: &[],
            collision_names: &BTreeSet::new(),
        };
        let mut findings = Vec::new();
        let mut fixes = Vec::new();
        process_occurrence(
            &occurrence,
            &ctx,
            &ImportAttributePlan {
                attributes: BTreeMap::new(),
            },
            &mut BTreeSet::new(),
            &mut findings,
            &mut fixes,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(fixes.len(), 1);
        assert_eq!((fixes[0].start, fixes[0].end), (0, text.len()));
        assert_eq!(fixes[0].replacement, "Widget");
    }
}
