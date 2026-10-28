use std::cell::OnceCell;
#[cfg(feature = "test-counters")]
use std::cell::RefCell;
use std::cmp::Ordering;
#[cfg(feature = "test-counters")]
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;
use rustc_span::def_id::LocalDefId;
#[cfg(test)]
use syn::File;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::spanned::Spanned;

use super::boundary;
use super::boundary::ModuleSourceMap;
use super::reference;
use super::reference::ParentFacadeUsage;
use super::reference::ParentFacadeUsageByName;
use crate::compiler::settings::DriverSettings;
#[cfg(test)]
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;
use crate::fixes;
use crate::rust_syntax;

#[cfg(feature = "test-counters")]
#[derive(Default)]
struct FacadePerformanceCounters {
    resolution_requests: HashMap<LocalDefId, usize>,
    resolutions:         HashMap<LocalDefId, usize>,
    usage_scans:         HashMap<LocalDefId, usize>,
}

#[cfg(feature = "test-counters")]
thread_local! {
    static PERFORMANCE_COUNTERS: RefCell<FacadePerformanceCounters> =
        RefCell::new(FacadePerformanceCounters::default());
}

#[cfg(all(test, feature = "test-counters"))]
pub fn reset_performance_counters() {
    PERFORMANCE_COUNTERS.with(|counters| {
        *counters.borrow_mut() = FacadePerformanceCounters::default();
    });
}

#[cfg(feature = "test-counters")]
pub fn record_facade_resolution(item_def_id: LocalDefId) {
    PERFORMANCE_COUNTERS.with(|counters| {
        *counters
            .borrow_mut()
            .resolutions
            .entry(item_def_id)
            .or_default() += 1;
    });
}

#[cfg(feature = "test-counters")]
pub fn record_facade_resolution_request(item_def_id: LocalDefId) {
    PERFORMANCE_COUNTERS.with(|counters| {
        *counters
            .borrow_mut()
            .resolution_requests
            .entry(item_def_id)
            .or_default() += 1;
    });
}

#[cfg(feature = "test-counters")]
pub fn record_facade_usage_scan(occurrence: LocalDefId) {
    PERFORMANCE_COUNTERS.with(|counters| {
        *counters
            .borrow_mut()
            .usage_scans
            .entry(occurrence)
            .or_default() += 1;
    });
}

#[cfg(all(test, feature = "test-counters"))]
pub fn facade_resolution_count(item_def_id: LocalDefId) -> usize {
    PERFORMANCE_COUNTERS.with(|counters| {
        counters
            .borrow()
            .resolutions
            .get(&item_def_id)
            .copied()
            .unwrap_or(0)
    })
}

#[cfg(all(test, feature = "test-counters"))]
pub fn facade_resolution_request_count(item_def_id: LocalDefId) -> usize {
    PERFORMANCE_COUNTERS.with(|counters| {
        counters
            .borrow()
            .resolution_requests
            .get(&item_def_id)
            .copied()
            .unwrap_or(0)
    })
}

#[cfg(all(test, feature = "test-counters"))]
pub fn facade_usage_scan_count(occurrence: LocalDefId) -> usize {
    PERFORMANCE_COUNTERS.with(|counters| {
        counters
            .borrow()
            .usage_scans
            .get(&occurrence)
            .copied()
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::compiler) enum ParentFacadeFixSupport {
    #[default]
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ParentFacadeSpelling {
    Public,
    Crate,
    Super,
    Other,
}

/// Whether a re-export's visibility reaches its owning module's parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ParentReach {
    ReachesParent,
    RestrictedToChild,
}

impl ParentReach {
    pub(in crate::compiler) const fn from_comparison(comparison: Option<Ordering>) -> Self {
        match comparison {
            Some(Ordering::Equal | Ordering::Greater) => Self::ReachesParent,
            Some(Ordering::Less) | None => Self::RestrictedToChild,
        }
    }
}

/// Conflicting source spellings cannot supply exact facade syntax or an exact repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum FacadeSpellingAgreement {
    Consistent,
    Conflicting,
}

/// A nonempty selection has one or several exports; only one supports a single-export repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ExportMultiplicity {
    Single,
    Multiple,
}

impl ExportMultiplicity {
    pub(in crate::compiler) const fn from_count(count: NonZeroUsize) -> Self {
        if count.get() == 1 {
            Self::Single
        } else {
            Self::Multiple
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) struct ParentFacadeReach {
    pub parent_reach:       ParentReach,
    pub spelling:           ParentFacadeSpelling,
    pub spelling_agreement: FacadeSpellingAgreement,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ParentFacadeExports {
    pub explicit:    Vec<String>,
    pub fix_support: ParentFacadeFixSupport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::compiler) struct ParentFacadeExportStatus {
    pub usage:               ParentFacadeUsage,
    pub fix_support:         ParentFacadeFixSupport,
    pub parent_facade_reach: ParentFacadeReach,
    pub parent_path:         PathBuf,
    pub parent_rel_path:     String,
    pub parent_line:         usize,
    /// Def-path of the module holding the `use` item — the subtree the facade
    /// is claimed to serve internally. Spelled by `TyCtxt::def_path_str`, the
    /// same way `use_sites::def_path_string` spells caller modules, so
    /// `caller_aware` can test callers against it directly.
    pub boundary_def_path:   String,
}

impl ParentFacadeExportStatus {
    pub(in crate::compiler) const fn use_syntax(&self) -> Option<&'static str> {
        match self.parent_facade_reach.spelling_agreement {
            FacadeSpellingAgreement::Consistent => {},
            FacadeSpellingAgreement::Conflicting => return None,
        }
        match self.parent_facade_reach.spelling {
            ParentFacadeSpelling::Public => Some("pub use"),
            ParentFacadeSpelling::Crate => Some("pub(crate) use"),
            ParentFacadeSpelling::Super => Some("pub(super) use"),
            ParentFacadeSpelling::Other => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::compiler) struct ParentFacadeExportRequest<'tcx, 'source> {
    pub source_cache:        &'source SourceCache,
    pub settings:            &'source DriverSettings,
    pub source_root:         &'source Path,
    pub tcx:                 TyCtxt<'tcx>,
    pub module_sources:      &'source ModuleSourceMap,
    #[cfg(feature = "test-counters")]
    pub use_def_id:          LocalDefId,
    pub owner_module:        LocalDefId,
    pub use_span:            Span,
    pub parent_facade_reach: ParentFacadeReach,
    pub usage_by_name:       &'source OnceCell<ParentFacadeUsageByName>,
    pub export_names:        &'source [String],
    pub export_multiplicity: ExportMultiplicity,
    pub child_file:          &'source Path,
    pub item_name:           &'source str,
}

pub(in crate::compiler) fn parent_facade_export_status(
    request: ParentFacadeExportRequest<'_, '_>,
) -> Result<Option<ParentFacadeExportStatus>> {
    let ParentFacadeExportRequest {
        source_cache,
        settings,
        source_root,
        tcx,
        module_sources,
        #[cfg(feature = "test-counters")]
        use_def_id,
        owner_module,
        use_span,
        parent_facade_reach,
        usage_by_name,
        export_names,
        export_multiplicity,
        child_file,
        item_name,
    } = request;
    let Some(parent_boundary) = boundary::parent_boundary_for_reexport(tcx, owner_module, use_span)
    else {
        return Ok(None);
    };
    let parent_rel_path = parent_boundary
        .boundary_file
        .strip_prefix(source_root)
        .unwrap_or(&parent_boundary.boundary_file)
        .to_string_lossy()
        .replace('\\', "/");
    let parent_line = tcx.sess.source_map().lookup_char_pos(use_span.lo()).line;
    let exported_names = ParentFacadeExports {
        explicit:    export_names.to_vec(),
        fix_support: parent_facade_fix_support(
            source_cache,
            &parent_boundary.boundary_file,
            child_file,
            item_name,
            tcx,
            use_span,
            export_multiplicity,
        ),
    };
    let usage_by_name = initialized_facade_usage(usage_by_name, || {
        #[cfg(feature = "test-counters")]
        record_facade_usage_scan(use_def_id);
        reference::scan_facade_usage(
            source_cache,
            settings,
            tcx,
            module_sources,
            &parent_boundary,
            &exported_names.explicit,
        )
    })?;
    let usage = usage_by_name
        .get(reference::normalized_export_name(item_name))
        .or_else(|| match export_names {
            [export_name] => usage_by_name.get(reference::normalized_export_name(export_name)),
            _ => None,
        })
        .copied()
        .unwrap_or(ParentFacadeUsage::Unused);

    Ok(Some(ParentFacadeExportStatus {
        usage,
        fix_support: exported_names.fix_support,
        parent_facade_reach,
        parent_path: parent_boundary.boundary_file,
        parent_rel_path,
        parent_line,
        boundary_def_path: tcx.def_path_str(owner_module.to_def_id()),
    }))
}

fn initialized_facade_usage(
    usage_by_name: &OnceCell<ParentFacadeUsageByName>,
    initialize: impl FnOnce() -> Result<ParentFacadeUsageByName>,
) -> Result<&ParentFacadeUsageByName> {
    if let Some(usage_by_name) = usage_by_name.get() {
        return Ok(usage_by_name);
    }
    let computed_usage_by_name = initialize()?;
    Ok(usage_by_name.get_or_init(|| computed_usage_by_name))
}

fn parent_facade_fix_support(
    source_cache: &SourceCache,
    parent_path: &Path,
    child_file: &Path,
    item_name: &str,
    tcx: TyCtxt<'_>,
    use_span: Span,
    export_multiplicity: ExportMultiplicity,
) -> ParentFacadeFixSupport {
    if export_multiplicity == ExportMultiplicity::Multiple {
        return ParentFacadeFixSupport::Unsupported;
    }
    let Some(child_module_name) = rust_syntax::module_name_for_child_boundary_file(child_file)
    else {
        return ParentFacadeFixSupport::Unsupported;
    };
    let Some(file) = source_cache.parsed_file(parent_path) else {
        return ParentFacadeFixSupport::Unsupported;
    };
    let Some(source) = source_cache.read_source(parent_path).ok() else {
        return ParentFacadeFixSupport::Unsupported;
    };
    let use_offset = tcx
        .sess
        .source_map()
        .lookup_byte_offset(use_span.lo())
        .pos
        .0 as usize;
    if file
        .items
        .iter()
        .filter_map(|item| {
            let Item::Use(item_use) = item else {
                return None;
            };
            Some(item_use)
        })
        .find(|item_use| item_use_start_offset(source, item_use) == Some(use_offset))
        .is_some_and(|item_use| {
            fixes::facade_use_prefix(&item_use.vis).is_some()
                && pub_use_fix_support(&item_use.tree, child_module_name, item_name)
                    == ParentFacadeFixSupport::Supported
        })
    {
        ParentFacadeFixSupport::Supported
    } else {
        ParentFacadeFixSupport::Unsupported
    }
}

fn item_use_start_offset(source: &str, item_use: &ItemUse) -> Option<usize> {
    let start = item_use.span().start();
    let line_offset = source
        .split_inclusive('\n')
        .take(start.line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    (line_offset + start.column <= source.len()).then_some(line_offset + start.column)
}

#[cfg(test)]
pub(super) fn exported_names_from_parent_boundary(
    file: &File,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeExports {
    let mut exported = ParentFacadeExports::default();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        collect_matching_pub_use_exports(item_use, child_module_name, item_name, &mut exported);
    }
    exported.explicit.sort();
    exported.explicit.dedup();
    exported
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportMatch {
    Missing,
    Found,
}

#[cfg(test)]
fn collect_matching_pub_use_exports(
    item_use: &ItemUse,
    child_module_name: &str,
    item_name: &str,
    exported: &mut ParentFacadeExports,
) {
    let mut paths = Vec::new();
    source_cache::flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
    let mut matched = ExportMatch::Missing;
    for path in paths {
        let normalized = rust_syntax::trim_leading_self(&path);
        if normalized.len() >= 2
            && normalized[0] == child_module_name
            && normalized[1..].iter().any(|segment| segment == item_name)
            && let Some(export_name) = normalized.last()
        {
            exported.explicit.push(export_name.clone());
            matched = ExportMatch::Found;
        }
    }
    if matched == ExportMatch::Found
        && fixes::facade_use_prefix(&item_use.vis).is_some()
        && pub_use_fix_support(&item_use.tree, child_module_name, item_name)
            == ParentFacadeFixSupport::Supported
    {
        exported.fix_support = ParentFacadeFixSupport::Supported;
    }
}

fn pub_use_fix_support(
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeFixSupport {
    pub_use_fix_support_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn pub_use_fix_support_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeFixSupport {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            pub_use_fix_support_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = rust_syntax::trim_leading_self(&prefix);
            if normalized.len() == 1
                && normalized[0] == child_module_name
                && name.ident == item_name
            {
                ParentFacadeFixSupport::Supported
            } else {
                ParentFacadeFixSupport::Unsupported
            }
        },
        UseTree::Group(group) => {
            if group.items.iter().any(|item| {
                pub_use_fix_support_with_prefix(prefix.clone(), item, child_module_name, item_name)
                    == ParentFacadeFixSupport::Supported
            }) {
                ParentFacadeFixSupport::Supported
            } else {
                ParentFacadeFixSupport::Unsupported
            }
        },
        UseTree::Rename(_) | UseTree::Glob(_) => ParentFacadeFixSupport::Unsupported,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cell::Cell;
    use std::cell::OnceCell;
    use std::cmp::Ordering;

    use anyhow::anyhow;
    use syn::parse_file;

    use super::ParentFacadeExports;
    use super::ParentFacadeFixSupport;
    use super::ParentFacadeUsage;
    use super::ParentFacadeUsageByName;
    use super::exported_names_from_parent_boundary;
    use super::initialized_facade_usage;

    #[test]
    fn parent_reach_requires_equal_or_wider_visibility() {
        for (comparison, expected) in [
            (Some(Ordering::Equal), super::ParentReach::ReachesParent),
            (Some(Ordering::Greater), super::ParentReach::ReachesParent),
            (Some(Ordering::Less), super::ParentReach::RestrictedToChild),
            (None, super::ParentReach::RestrictedToChild),
        ] {
            assert_eq!(super::ParentReach::from_comparison(comparison), expected);
        }
    }

    #[test]
    fn export_multiplicity_admits_exactly_one_matching_occurrence() {
        for (count, expected) in [
            (1, super::ExportMultiplicity::Single),
            (2, super::ExportMultiplicity::Multiple),
            (usize::MAX, super::ExportMultiplicity::Multiple),
        ] {
            assert_eq!(
                super::ExportMultiplicity::from_count(super::NonZeroUsize::new(count).unwrap()),
                expected
            );
        }
    }

    #[test]
    fn facade_syntax_requires_consistent_spelling() {
        for (spelling, syntax) in [
            (super::ParentFacadeSpelling::Public, Some("pub use")),
            (super::ParentFacadeSpelling::Crate, Some("pub(crate) use")),
            (super::ParentFacadeSpelling::Super, Some("pub(super) use")),
            (super::ParentFacadeSpelling::Other, None),
        ] {
            for (agreement, expected) in [
                (super::FacadeSpellingAgreement::Consistent, syntax),
                (super::FacadeSpellingAgreement::Conflicting, None),
            ] {
                for parent_reach in [
                    super::ParentReach::ReachesParent,
                    super::ParentReach::RestrictedToChild,
                ] {
                    let status = super::ParentFacadeExportStatus {
                        usage:               ParentFacadeUsage::Unused,
                        fix_support:         ParentFacadeFixSupport::Unsupported,
                        parent_facade_reach: super::ParentFacadeReach {
                            parent_reach,
                            spelling,
                            spelling_agreement: agreement,
                        },
                        parent_path:         "src/lib.rs".into(),
                        parent_rel_path:     "src/lib.rs".into(),
                        parent_line:         1,
                        boundary_def_path:   String::from("crate"),
                    };
                    assert_eq!(status.use_syntax(), expected);
                }
            }
        }
    }

    #[test]
    fn use_tree_support_preserves_group_anchor_and_leaf_policy() -> anyhow::Result<()> {
        for (source, expected) in [
            ("pub use child::Thing;", ParentFacadeFixSupport::Supported),
            (
                "pub use self::child::Thing;",
                ParentFacadeFixSupport::Supported,
            ),
            (
                "pub use child::{Else, Thing};",
                ParentFacadeFixSupport::Supported,
            ),
            (
                "pub use child::{Thing, Else};",
                ParentFacadeFixSupport::Supported,
            ),
            (
                "pub use child::{Thing as Alias};",
                ParentFacadeFixSupport::Unsupported,
            ),
            ("pub use child::*;", ParentFacadeFixSupport::Unsupported),
            ("pub use child::{};", ParentFacadeFixSupport::Unsupported),
            ("pub use child::{{}};", ParentFacadeFixSupport::Unsupported),
            (
                "pub use child::{{}, Thing};",
                ParentFacadeFixSupport::Supported,
            ),
            (
                "pub use child::{{{}}, Thing};",
                ParentFacadeFixSupport::Supported,
            ),
            (
                "pub use child::nested::Thing;",
                ParentFacadeFixSupport::Unsupported,
            ),
            (
                "pub use sibling::Thing;",
                ParentFacadeFixSupport::Unsupported,
            ),
            ("pub use child::Else;", ParentFacadeFixSupport::Unsupported),
        ] {
            let item_use = syn::parse_str::<syn::ItemUse>(source)?;
            assert_eq!(
                super::pub_use_fix_support(&item_use.tree, "child", "Thing"),
                expected,
                "{source}",
            );
        }
        Ok(())
    }

    #[test]
    fn missing_export_does_not_promote_support() -> anyhow::Result<()> {
        let file = parse_file("pub use child::{Else, Another};")?;
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports, ParentFacadeExports::default());
        Ok(())
    }

    #[test]
    fn initialized_facade_usage_scans_once_and_preserves_export_usage() {
        let cache = OnceCell::new();
        let scans = Cell::new(0);
        let initialize = || {
            scans.set(scans.get() + 1);
            Ok(ParentFacadeUsageByName::from_iter([(
                "Thing".to_string(),
                ParentFacadeUsage::UsedOutsideSubtree,
            )]))
        };
        let first = initialized_facade_usage(&cache, initialize).unwrap();
        let second = initialized_facade_usage(&cache, initialize).unwrap();

        assert_eq!(scans.get(), 1);
        assert!(std::ptr::eq(first, second));
        assert_eq!(
            first.get("Thing"),
            Some(&ParentFacadeUsage::UsedOutsideSubtree)
        );
        assert_eq!(first.get("Missing"), None);
    }

    #[test]
    fn initialized_facade_usage_propagates_failure_and_retries() {
        let cache = OnceCell::new();
        let failure = initialized_facade_usage(&cache, || Err(anyhow!("usage scan failed")));

        assert_eq!(failure.unwrap_err().to_string(), "usage scan failed");
        assert!(cache.get().is_none());

        let ready =
            initialized_facade_usage(&cache, || Ok(ParentFacadeUsageByName::default())).unwrap();
        assert!(ready.is_empty());
        assert!(std::ptr::eq(cache.get().unwrap(), ready));
    }

    #[test]
    fn grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
        let file = parse_file(source).unwrap();
        let exports =
            exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
        assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
        assert_eq!(exports.fix_support, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn mixed_pub_uses_keep_matching_export_names_separate() {
        // `mixed_pub_uses_keep_matching_export_names_separate` parses a parent file
        // with both `pub(crate) use` and `pub use` lines pointing at
        // different children. The matching names must come from the line that
        // re-exports the queried item, not whichever `use` appears first.
        let source = "\
pub(crate) use first_child::Alpha;
pub use second_child::Beta;
";
        let file = parse_file(source).unwrap();

        let exports = exported_names_from_parent_boundary(&file, "first_child", "Alpha");
        assert_eq!(exports.explicit, vec!["Alpha".to_string()]);

        let exports = exported_names_from_parent_boundary(&file, "second_child", "Beta");
        assert_eq!(exports.explicit, vec!["Beta".to_string()]);
    }

    #[test]
    fn duplicate_re_exports_keep_all_export_names() {
        // `duplicate_re_exports_keep_all_export_names` re-exports the same item twice.
        // HIR-derived `ReexportIndex` owns resolved visibility;
        // `exported_names_from_parent_boundary` only exposes written names.
        let source = "\
pub(crate) use child::Thing;
pub use child::Thing;
";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.explicit, vec!["Thing".to_string()]);
    }

    #[test]
    fn multiline_grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.explicit, vec!["Thing".to_string()]);
        assert_eq!(exports.fix_support, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn grouped_parent_pub_use_with_rename_is_manual_only() {
        let source = "pub use child::{Thing as RenamedThing, Other};\n";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(
            exports,
            ParentFacadeExports {
                explicit:    vec!["RenamedThing".to_string()],
                fix_support: ParentFacadeFixSupport::Unsupported,
            }
        );

        let exports = exported_names_from_parent_boundary(&file, "child", "Other");
        assert_eq!(exports.explicit, vec!["Other".to_string()]);
        assert_eq!(exports.fix_support, ParentFacadeFixSupport::Supported);
    }
}
