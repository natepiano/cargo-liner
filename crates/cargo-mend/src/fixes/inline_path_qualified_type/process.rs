use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;

use super::scope;
use super::scope::ScopeInfo;
use super::visitor::InlinePathOccurrence;
use crate::config::DiagnosticCode;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;
use crate::rust_syntax::PathAnchor;

/// Resolution needs scopes before collision analysis has resolved every import path.
/// It cannot process occurrences or pretend that an empty set is completed analysis.
pub(super) struct OccurrenceResolutionContext<'a> {
    pub(super) scopes: &'a [ScopeInfo],
}

/// Processing is available only after resolving paths and analyzing collision names.
/// The private collision set prevents resolution from supplying provisional empty state.
pub(super) struct OccurrenceContext<'a> {
    path:            &'a Path,
    display_path:    &'a str,
    scopes:          &'a [ScopeInfo],
    collision_names: BTreeSet<String>,
}

impl<'a> OccurrenceResolutionContext<'a> {
    pub(super) fn analyze_collisions(
        self,
        path: &'a Path,
        display_path: &'a str,
        occurrences: &[InlinePathOccurrence],
        resolved: &[ResolvedOccurrence<'_>],
        bare_type_names: &BTreeSet<String>,
    ) -> OccurrenceContext<'a> {
        let resolved_import_paths = resolved
            .iter()
            .map(|item| item.import_path.clone())
            .collect::<Vec<_>>();
        let existing_imports = self
            .scopes
            .iter()
            .flat_map(|scope| scope.existing_imports.keys().cloned())
            .collect();
        let collision_names = find_collision_names(
            occurrences,
            &resolved_import_paths,
            bare_type_names,
            &existing_imports,
        );
        OccurrenceContext {
            path,
            display_path,
            scopes: self.scopes,
            collision_names,
        }
    }
}

type ImportKey = (usize, String);

pub(super) struct ImportAttributePlan {
    attributes: BTreeMap<ImportKey, ConditionalAttributes>,
}

impl ImportAttributePlan {
    fn get(&self, key: &ImportKey) -> Option<&ConditionalAttributes> { self.attributes.get(key) }
}

/// An occurrence span checked against the source that supplies its diagnostic line.
/// Missing lines must be rejected during resolution, rather than reported with fabricated
/// empty diagnostic text. Byte offsets and character coordinates describe that same span.
struct OccurrenceLocation<'a> {
    byte_start:  usize,
    byte_end:    usize,
    start:       LineColumn,
    source_line: &'a str,
}

impl<'a> OccurrenceLocation<'a> {
    fn new(source: &ImportSource<'a>, start: LineColumn, end: LineColumn) -> Result<Self> {
        let location = source
            .locate(start, end)
            .context("invalid inline path source span")?;
        Ok(Self {
            byte_start: location.start(),
            byte_end: location.end(),
            start,
            source_line: location.source_line(),
        })
    }
}

pub(super) struct ResolvedOccurrence<'a> {
    occurrence:             &'a InlinePathOccurrence,
    location:               OccurrenceLocation<'a>,
    scope_id:               Option<usize>,
    import_path:            String,
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
    occurrences: &[ResolvedOccurrence<'_>],
) -> ImportAttributePlan {
    let mut grouped: BTreeMap<ImportKey, BTreeSet<ConditionalAttributes>> = BTreeMap::new();
    for occurrence in occurrences {
        let Some(scope_id) = occurrence.scope_id else {
            continue;
        };
        grouped
            .entry((scope_id, occurrence.import_path.clone()))
            .or_default()
            .insert(occurrence.conditional_attributes.clone());
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
    resolved: &ResolvedOccurrence<'_>,
    ctx: &OccurrenceContext<'_>,
    import_attribute_plan: &ImportAttributePlan,
    inserted_use_paths: &mut BTreeSet<(usize, String)>,
    findings: &mut Vec<Finding>,
    fixes: &mut Vec<UseFix>,
) {
    let occ = resolved.occurrence;
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
            record_occurrence(ctx, resolved, findings, fixes);
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
            let group = record_occurrence(ctx, resolved, findings, fixes);
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
    ctx: &OccurrenceContext<'_>,
    resolved: &ResolvedOccurrence<'_>,
    findings: &mut Vec<Finding>,
    fixes: &mut Vec<UseFix>,
) -> ImportGroup {
    let occ = resolved.occurrence;
    let import_path = &resolved.import_path;

    findings.push(Finding {
        severity:        Severity::Warning,
        diagnostic_code: DiagnosticCode::InlinePathQualifiedType,
        path:            ctx.display_path.to_string(),
        line:            resolved.location.start.line,
        column:          resolved.location.start.column + 1,
        highlight_len:   occ.full_path.len().max(1),
        source_line:     resolved.location.source_line.to_string(),
        item:            None,
        message:         format!(
            "use a `use` import for `{}` instead of inline path",
            occ.import_name
        ),
        suggestion:      Some(format!("consider adding: `use {import_path};`")),
        fix_support:     FixSupport::InlinePathQualifiedType,
        related:         None,
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
        start:        resolved.location.byte_start,
        end:          resolved.location.byte_end,
        replacement:  occ.replacement.clone(),
        import_group: Some(group.clone()),
    });

    group
}

pub(super) fn resolve_occurrence<'a>(
    occurrence: &'a InlinePathOccurrence,
    ctx: &OccurrenceResolutionContext<'_>,
    source: &ImportSource<'a>,
) -> Result<ResolvedOccurrence<'a>> {
    let location = OccurrenceLocation::new(source, occurrence.span_start, occurrence.span_end)?;
    let scope_id = scope::find_innermost_scope(ctx.scopes, location.byte_start);
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

    Ok(ResolvedOccurrence {
        occurrence,
        location,
        scope_id,
        import_path,
        conditional_attributes,
    })
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
fn find_collision_names(
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
    use super::OccurrenceLocation;
    use super::OccurrenceResolutionContext;
    use super::absolutize_import_path;
    use super::process_occurrence;
    use super::resolve_occurrence;
    use crate::fixes::imports::ConditionalAttributes;
    use crate::fixes::imports::ImportSource;
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
    fn an_unscoped_occurrence_rewrites_without_inserting_an_import() -> anyhow::Result<()> {
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
        let ctx = OccurrenceResolutionContext { scopes: &[] };
        let mut findings = Vec::new();
        let mut fixes = Vec::new();
        let source = ImportSource::new(text);
        let resolved = resolve_occurrence(&occurrence, &ctx, &source)?;
        let blocked = OccurrenceResolutionContext { scopes: &[] }.analyze_collisions(
            Path::new("src/lib.rs"),
            "src/lib.rs",
            std::slice::from_ref(&occurrence),
            std::slice::from_ref(&resolved),
            &BTreeSet::from(["Widget".to_string()]),
        );
        process_occurrence(
            &resolved,
            &blocked,
            &ImportAttributePlan {
                attributes: BTreeMap::new(),
            },
            &mut BTreeSet::new(),
            &mut findings,
            &mut fixes,
        );
        assert!(blocked.collision_names.contains("Widget"));
        assert!(findings.is_empty());
        assert!(fixes.is_empty());
        let ctx = ctx.analyze_collisions(
            Path::new("src/lib.rs"),
            "src/lib.rs",
            std::slice::from_ref(&occurrence),
            std::slice::from_ref(&resolved),
            &BTreeSet::new(),
        );
        process_occurrence(
            &resolved,
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
        assert_eq!(findings[0].source_line, text);
        Ok(())
    }

    #[test]
    fn occurrence_locations_reject_invalid_coordinates_and_keep_real_lines() -> anyhow::Result<()> {
        let text = "// é\nα::Widget\n";
        let source = ImportSource::new(text);
        let at = |line, column| LineColumn { line, column };
        for (start, end) in [
            (at(0, 0), at(1, 0)),
            (at(4, 0), at(4, 0)),
            (at(2, 10), at(2, 10)),
            (at(2, 2), at(2, 1)),
        ] {
            assert!(OccurrenceLocation::new(&source, start, end).is_err());
        }
        let location = OccurrenceLocation::new(&source, at(2, 0), at(2, 9))?;
        assert_eq!(location.source_line, "α::Widget");
        assert_eq!(&text[location.byte_start..location.byte_end], "α::Widget");
        let empty = OccurrenceLocation::new(&source, at(3, 0), at(3, 0))?;
        assert_eq!(empty.source_line, "");
        assert_eq!(empty.byte_start, text.len());
        Ok(())
    }
}
