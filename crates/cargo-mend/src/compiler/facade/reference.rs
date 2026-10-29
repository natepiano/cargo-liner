use std::cmp::Reverse;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use anyhow::Result;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::LocalDefId;
use rustc_span::def_id::LocalModId;

use super::boundary;
use super::boundary::ModuleContext;
use super::boundary::ModuleDefinitionPresence;
use super::boundary::ModuleSourceMap;
use super::boundary::ParentBoundary;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::ExtractedPaths;
use crate::compiler::source_cache::NameMention;
use crate::compiler::source_cache::PathOrigin;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::source_cache::UseRename;
use crate::rust_syntax;
use crate::rust_syntax::PathAnchor;
use crate::rust_syntax::SourceEndpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ParentFacadeUsage {
    Unused,
    UsedInsideSubtreeByRelativeImport,
    UsedInsideSubtreeByRelativePath,
    UsedInsideSubtreeByCrateImport,
    UsedInsideSubtreeByCratePath,
    UsedOutsideSubtree,
}

pub(in crate::compiler) type ParentFacadeUsageByName = FxHashMap<String, ParentFacadeUsage>;

pub(super) fn normalized_export_name(name: &str) -> &str { source_cache::normalized_name(name) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FacadeCallerLocation {
    InsideSubtree,
    OutsideSubtree,
}

impl FacadeCallerLocation {
    fn for_module(current_module: &[String], facade_module: &[String]) -> Self {
        if current_module == facade_module
            || (current_module.len() > facade_module.len()
                && current_module[..facade_module.len()] == *facade_module)
        {
            Self::InsideSubtree
        } else {
            Self::OutsideSubtree
        }
    }

    const fn usage(self, reference: ParentFacadeReferenceUsage) -> ParentFacadeUsage {
        match (self, reference) {
            (_, ParentFacadeReferenceUsage::None) => ParentFacadeUsage::Unused,
            (
                Self::OutsideSubtree,
                ParentFacadeReferenceUsage::Import(_) | ParentFacadeReferenceUsage::DirectPath(_),
            ) => ParentFacadeUsage::UsedOutsideSubtree,
            (Self::InsideSubtree, ParentFacadeReferenceUsage::Import(PathOrigin::Relative)) => {
                ParentFacadeUsage::UsedInsideSubtreeByRelativeImport
            },
            (Self::InsideSubtree, ParentFacadeReferenceUsage::Import(PathOrigin::Crate)) => {
                ParentFacadeUsage::UsedInsideSubtreeByCrateImport
            },
            (Self::InsideSubtree, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)) => {
                ParentFacadeUsage::UsedInsideSubtreeByRelativePath
            },
            (Self::InsideSubtree, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)) => {
                ParentFacadeUsage::UsedInsideSubtreeByCratePath
            },
        }
    }
}

/// Paths rejected here cannot name an export, so resolution can be skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportPathCandidate {
    Possible,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFileCandidate {
    Excluded,
    Possible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralIdentifierMatch {
    Bounded,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildItemReference {
    Found,
    Missing,
}

struct LiteralModuleContext {
    root_path:     Vec<String>,
    inline_ranges: Vec<ModuleRange>,
}

impl LiteralModuleContext {
    fn path_at(&self, offset: usize) -> &[String] {
        self.inline_ranges
            .iter()
            .filter(|module_range| module_range.bytes.contains(&offset))
            .min_by_key(|module_range| {
                (
                    module_range
                        .bytes
                        .end
                        .saturating_sub(module_range.bytes.start),
                    Reverse(module_range.path.len()),
                )
            })
            .map_or(self.root_path.as_slice(), |module_range| {
                module_range.path.as_slice()
            })
    }
}

struct ModuleRange {
    path:  Vec<String>,
    bytes: Range<usize>,
}

pub(super) fn scan_facade_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    parent_boundary: &ParentBoundary,
    exported_names: &[String],
) -> Result<ParentFacadeUsageByName> {
    let mut usage_by_name = exported_names
        .iter()
        .map(|name| {
            (
                normalized_export_name(name).to_string(),
                ParentFacadeUsage::Unused,
            )
        })
        .collect::<ParentFacadeUsageByName>();
    let explicit_names = ExportNameIndex::new(exported_names);

    for source_path in source_cache.source_files() {
        if export_file_candidate(source_cache, source_path, &explicit_names)
            == ExportFileCandidate::Excluded
        {
            continue;
        }
        let Some(extracted) = source_cache.extracted_paths(source_path) else {
            continue;
        };
        let module_contexts = active_module_contexts(extracted, module_sources, tcx, source_path);
        for module_context in module_contexts.iter() {
            if source_path == parent_boundary.boundary_file
                && module_context.path == parent_boundary.module_path
            {
                continue;
            }
            let reference_usage_by_name = source_references_parent_exports(
                extracted,
                &module_context.path,
                &module_context.suffix,
                &parent_boundary.module_path,
                &explicit_names,
            );
            let caller_location = FacadeCallerLocation::for_module(
                &module_context.path,
                &parent_boundary.module_path,
            );
            for (name, reference_usage) in reference_usage_by_name {
                let next_usage = caller_location.usage(reference_usage);
                merge_usage_for_name(&mut usage_by_name, &name, next_usage);
            }
        }
    }

    let literal_usage_by_name = workspace_source_parent_export_literal_usage(
        source_cache,
        settings,
        tcx,
        module_sources,
        &parent_boundary.module_path,
        exported_names,
    )?;
    for (name, literal_usage) in literal_usage_by_name {
        merge_usage_for_name(&mut usage_by_name, &name, literal_usage);
    }

    Ok(usage_by_name)
}

pub(in crate::compiler) fn workspace_source_parent_export_literal_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    module_path: &[String],
    exported_names: &[String],
) -> Result<ParentFacadeUsageByName> {
    let mut usage_by_name = exported_names
        .iter()
        .map(|name| {
            (
                normalized_export_name(name).to_string(),
                ParentFacadeUsage::Unused,
            )
        })
        .collect::<ParentFacadeUsageByName>();
    if module_path.is_empty() {
        return Ok(usage_by_name);
    }

    let export_spellings = export_spellings(exported_names);

    for file in source_cache.source_files() {
        if file.starts_with(&settings.analyzing_dir) {
            continue;
        }
        if export_spellings
            .iter()
            .all(|export| source_cache.name_mention(file, export.name) == NameMention::Absent)
        {
            continue;
        }
        let root_modules = module_sources.root_modules_for_file(tcx, file);
        if root_modules.is_empty() {
            continue;
        }
        let lexical_regions = source_cache.lexical_regions(file)?;
        let source = lexical_regions.source();
        let mut matches = Vec::new();
        for export in &export_spellings {
            if source_cache.name_mention(file, export.name) == NameMention::Absent {
                continue;
            }
            for item_spelling in &export.spellings {
                matches.extend(source.match_indices(item_spelling.as_str()).filter_map(
                    |(item_offset, matched_item)| {
                        let item_start = lexical_regions.endpoint(item_offset)?;
                        let path_start = literal_crate_path_start(item_start, module_path)?;
                        let path_offset = path_start.offset();
                        let path_length = item_offset + matched_item.len() - path_offset;
                        match literal_identifier_match(source, path_offset, path_length) {
                            LiteralIdentifierMatch::Bounded => Some((path_offset, export.name)),
                            LiteralIdentifierMatch::Rejected => None,
                        }
                    },
                ));
            }
        }
        if matches.is_empty() {
            continue;
        }
        let module_contexts = literal_module_contexts(module_sources, tcx, file, &root_modules);
        for (offset, name) in matches {
            for module_context in &module_contexts {
                let current_module_path = module_context.path_at(offset);
                let literal_usage =
                    FacadeCallerLocation::for_module(current_module_path, module_path)
                        .usage(ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate));
                merge_usage_for_name(&mut usage_by_name, name, literal_usage);
            }
        }
    }

    Ok(usage_by_name)
}

/// The exported names of one facade, with any `r#` prefix stripped, indexed for
/// constant-time lookup.
///
/// [`matching_export_name_indexed`] accepts a resolved path only when its final
/// segment is one of these names, so the index also answers that test before
/// [`resolve_module_relative_paths`] allocates a single candidate path.
struct ExportNameIndex<'name> {
    normalized: FxHashSet<&'name str>,
}

impl<'name> ExportNameIndex<'name> {
    fn new(exported_names: &'name [String]) -> Self {
        Self {
            normalized: exported_names
                .iter()
                .map(|name| normalized_export_name(name))
                .collect(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &'name str> + '_ { self.normalized.iter().copied() }

    /// The exported name equal to `segment`, ignoring any `r#` prefix.
    fn matching(&self, segment: &str) -> Option<&'name str> {
        self.normalized
            .get(normalized_export_name(segment))
            .copied()
    }

    /// Whether any path [`resolve_module_relative_paths`] derives from `raw`
    /// could name an export.
    ///
    /// [`matching_export_name_indexed`] keeps a candidate only when its length is
    /// `module_path.len() + 1`, so the segment it compares against this index is
    /// always the candidate's final one. Every resolution of `raw` ends in
    /// `raw`'s own final segment whenever that segment is a plain
    /// [`PathAnchor::Name`] — `crate`/`self`/`super` resolution only rewrites the
    /// prefix. A final segment absent from the index therefore cannot match, and
    /// the caller can skip resolving `raw` entirely.
    ///
    /// Returns `Possible` for an anchor-final path such as `super::super`, where
    /// resolution drops the anchor and the final segment comes from the current
    /// module path instead. Those fall through to the full scan.
    fn path_candidate(&self, raw: &[String]) -> ExportPathCandidate {
        if raw.last().is_none_or(|segment| {
            PathAnchor::from(segment.as_str()) != PathAnchor::Name
                || self.normalized.contains(normalized_export_name(segment))
        }) {
            ExportPathCandidate::Possible
        } else {
            ExportPathCandidate::Excluded
        }
    }
}

/// Whether `source_file` can be skipped without changing the scan's result.
///
/// A name reaches a facade scan only as a path segment or an identifier-bounded
/// literal, both of which are verbatim words of the source, so a file the name
/// index does not list cannot contribute a match. Skipping it avoids parsing its
/// module contexts and resolving its paths — the dominant cost of a whole-crate
/// sweep that runs once per re-export occurrence.
fn export_file_candidate(
    source_cache: &SourceCache,
    source_file: &Path,
    export_names: &ExportNameIndex<'_>,
) -> ExportFileCandidate {
    if export_names
        .iter()
        .all(|name| source_cache.name_mention(source_file, name) == NameMention::Absent)
    {
        ExportFileCandidate::Excluded
    } else {
        ExportFileCandidate::Possible
    }
}

/// One exported name and the two ways source can spell it.
struct ExportSpellings<'name> {
    name:      &'name str,
    spellings: [String; 2],
}

/// The spellings to search for, built once instead of once per scanned file.
fn export_spellings(exported_names: &[String]) -> Vec<ExportSpellings<'_>> {
    exported_names
        .iter()
        .map(|name| {
            let name = normalized_export_name(name);
            ExportSpellings {
                name,
                spellings: [name.to_string(), format!("r#{name}")],
            }
        })
        .collect()
}

fn literal_crate_path_start<'source>(
    item_start: SourceEndpoint<'source>,
    module_path: &[String],
) -> Option<SourceEndpoint<'source>> {
    let mut separator = path_separator_before(item_start)?;
    for expected_segment in module_path.iter().rev() {
        let segment_end = separator.trivia_start_before();
        let segment_start = literal_identifier_start(segment_end);
        let source_segment = &segment_end.prefix()[segment_start.offset()..];
        if normalized_export_name(source_segment) != normalized_export_name(expected_segment) {
            return None;
        }
        separator = path_separator_before(segment_start)?;
    }

    let crate_end = separator.trivia_start_before();
    let crate_start = literal_identifier_start(crate_end);
    (&crate_end.prefix()[crate_start.offset()..] == "crate").then_some(crate_start)
}

fn path_separator_before(segment_start: SourceEndpoint<'_>) -> Option<SourceEndpoint<'_>> {
    segment_start.trivia_start_before().strip_suffix("::")
}

fn literal_identifier_start(segment_end: SourceEndpoint<'_>) -> SourceEndpoint<'_> {
    let start = segment_end.trim_end_matches(rust_syntax::identifier_character);
    start.strip_suffix("r#").unwrap_or(start)
}

/// Reject unavailable byte boundaries and literals adjoining identifier characters.
fn literal_identifier_match(source: &str, offset: usize, length: usize) -> LiteralIdentifierMatch {
    let Some(before) = source.get(..offset) else {
        return LiteralIdentifierMatch::Rejected;
    };
    let Some(after) = source.get(offset + length..) else {
        return LiteralIdentifierMatch::Rejected;
    };
    if before
        .chars()
        .next_back()
        .is_none_or(|character| !rust_syntax::identifier_character(character))
        && after
            .chars()
            .next()
            .is_none_or(|character| !rust_syntax::identifier_character(character))
    {
        LiteralIdentifierMatch::Bounded
    } else {
        LiteralIdentifierMatch::Rejected
    }
}

fn literal_module_contexts(
    module_sources: &ModuleSourceMap,
    tcx: TyCtxt<'_>,
    source_file: &Path,
    root_modules: &[LocalDefId],
) -> Vec<LiteralModuleContext> {
    root_modules
        .iter()
        .map(|root_module| {
            let root_path = boundary::module_path(tcx, *root_module);
            let mut inline_ranges = Vec::new();
            tcx.hir_for_each_module(|module| {
                let module_path = boundary::module_path(tcx, module.to_local_def_id());
                if module_path.len() <= root_path.len()
                    || !module_path.starts_with(&root_path)
                    || module_sources.module_definition_presence(tcx, source_file, &module_path)
                        == ModuleDefinitionPresence::Absent
                {
                    return;
                }
                let Some(bytes) = module_byte_range_in_file(tcx, module, source_file) else {
                    return;
                };
                inline_ranges.push(ModuleRange {
                    path: module_path,
                    bytes,
                });
            });
            LiteralModuleContext {
                root_path,
                inline_ranges,
            }
        })
        .collect()
}

fn module_byte_range_in_file(
    tcx: TyCtxt<'_>,
    module: LocalModId,
    source_file: &Path,
) -> Option<Range<usize>> {
    let (_, span, _) = tcx.hir_get_module(module);
    span_byte_range_in_file(tcx, span, source_file)
}

fn span_byte_range_in_file(
    tcx: TyCtxt<'_>,
    span: Span,
    source_file: &Path,
) -> Option<Range<usize>> {
    let source_map = tcx.sess.source_map();
    let start = source_map.lookup_byte_offset(span.lo());
    let end = source_map.lookup_byte_offset(span.hi());
    if start.sf.stable_id != end.sf.stable_id {
        return None;
    }
    let FileName::Real(real_file_name) = &start.sf.name else {
        return None;
    };
    let span_file = real_file_name.local_path()?;
    let canonical_span_file =
        fs::canonicalize(span_file).unwrap_or_else(|_| span_file.to_path_buf());
    let canonical_source_file =
        fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf());
    if canonical_span_file != canonical_source_file {
        return None;
    }
    let start_offset = start.sf.original_relative_byte_pos(span.lo()).0 as usize;
    let end_offset = end.sf.original_relative_byte_pos(span.hi()).0 as usize;
    Some(start_offset..end_offset)
}

fn source_references_parent_exports(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    module_path: &[String],
    exported_names: &ExportNameIndex<'_>,
) -> FxHashMap<String, ParentFacadeReferenceUsage> {
    let mut usage_by_name = FxHashMap::default();
    let candidates = &extracted.export_candidates;
    for path_index in candidates.expr_paths.candidates(exported_names.iter()) {
        let extracted_path = &extracted.expr_paths[path_index];
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        let matching_name = matching_export_name_indexed(
            &extracted_path.segments,
            current_module_path,
            module_path,
            exported_names,
        )
        .or_else(|| {
            resolve_alias_expr_path(
                &extracted_path.segments,
                module_suffix,
                &extracted.use_renames,
            )
            .and_then(|resolved| {
                matching_export_name_indexed(
                    &resolved,
                    current_module_path,
                    module_path,
                    exported_names,
                )
            })
        });
        if let Some(name) = matching_name {
            merge_reference_usage_for_name(
                &mut usage_by_name,
                name,
                ParentFacadeReferenceUsage::DirectPath(extracted_path.origin),
            );
        }
    }

    for path_index in candidates.use_paths.candidates(exported_names.iter()) {
        let extracted_path = &extracted.use_paths[path_index];
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        if let Some(name) = matching_export_name_indexed(
            &extracted_path.segments,
            current_module_path,
            module_path,
            exported_names,
        ) {
            merge_reference_usage_for_name(
                &mut usage_by_name,
                name,
                ParentFacadeReferenceUsage::Import(extracted_path.origin),
            );
        }
    }

    usage_by_name
}

/// Resolves the first segment of an `expr_path` through module aliases.
///
/// Given `["test_utils", "assert_test_case"]` and a rename mapping
/// `test_utils → ["crate", "test_support"]`, returns
/// `["crate", "test_support", "assert_test_case"]`.
fn resolve_alias_expr_path(
    raw: &[String],
    module_suffix: &[String],
    renames: &[UseRename],
) -> Option<Vec<String>> {
    let first = raw.first()?;
    let rename = renames
        .iter()
        .find(|rename| rename.module_suffix == module_suffix && rename.alias == *first)?;
    let mut resolved = rename.original_path.clone();
    resolved.extend(raw[1..].iter().cloned());
    Some(resolved)
}

/// A resolution of a source path, held as the two borrowed halves it is built
/// from — a prefix of the current module path and a tail of the raw path.
///
/// [`resolve_module_relative_paths`] materializes the same resolutions as owned
/// `Vec<String>`s for callers that need to keep them. The whole-crate sweep in
/// [`source_references_parent_exports`] only compares them, so it borrows
/// instead: cloning every segment of every candidate dominated the sweep.
struct ResolvedPathParts<'segments> {
    prefix: &'segments [String],
    tail:   &'segments [String],
}

impl ResolvedPathParts<'_> {
    const fn len(&self) -> usize { self.prefix.len() + self.tail.len() }

    fn segment(&self, index: usize) -> Option<&String> {
        self.prefix
            .get(index)
            .or_else(|| self.tail.get(index - self.prefix.len()))
    }
}

fn matching_export_name_indexed<'name>(
    raw: &[String],
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &ExportNameIndex<'name>,
) -> Option<&'name str> {
    // `matching_export_name_indexed` uses `ExportNameIndex::path_candidate`
    // to reject paths before resolution because they cannot name an export,
    // the overwhelming majority on a whole-crate sweep.
    if exported_names.path_candidate(raw) == ExportPathCandidate::Excluded {
        return None;
    }
    let candidate = resolved_path_of_len(raw, current_module_path, module_path.len() + 1)?;
    module_path
        .iter()
        .enumerate()
        .all(|(index, compiler_segment)| {
            candidate.segment(index).is_some_and(|source_segment| {
                normalized_export_name(source_segment) == normalized_export_name(compiler_segment)
            })
        })
        .then(|| candidate.segment(module_path.len()))
        .flatten()
        .and_then(|segment| exported_names.matching(segment))
}

/// The one resolution of `raw` whose segment count is `candidate_len`, or `None`
/// when no resolution has that length.
///
/// A `crate`, `self`, or `super` anchor resolves to a single path. A plain
/// [`PathAnchor::Name`] resolves to one path per prefix of
/// `current_module_path`, but each has length `prefix_len + raw.len()`, so at
/// most one can be `candidate_len` — solving for that prefix length skips
/// building the rest.
fn resolved_path_of_len<'segments>(
    raw: &'segments [String],
    current_module_path: &'segments [String],
    candidate_len: usize,
) -> Option<ResolvedPathParts<'segments>> {
    let parts = match PathAnchor::first(raw)? {
        PathAnchor::Crate => ResolvedPathParts {
            prefix: &[],
            tail:   raw.get(1..)?,
        },
        PathAnchor::SelfMod => ResolvedPathParts {
            prefix: current_module_path,
            tail:   raw.get(1..)?,
        },
        PathAnchor::Super => {
            let mut index = 0usize;
            let mut retained = current_module_path.len();
            while raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
            {
                retained = retained.checked_sub(1)?;
                index += 1;
            }
            if raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::SelfMod)
            {
                index += 1;
            }
            ResolvedPathParts {
                prefix: current_module_path.get(..retained)?,
                tail:   raw.get(index..)?,
            }
        },
        PathAnchor::SelfType | PathAnchor::Name => ResolvedPathParts {
            prefix: current_module_path.get(..candidate_len.checked_sub(raw.len())?)?,
            tail:   raw,
        },
    };
    (parts.len() == candidate_len).then_some(parts)
}

fn merge_reference_usage_for_name(
    usage_by_name: &mut FxHashMap<String, ParentFacadeReferenceUsage>,
    name: &str,
    next: ParentFacadeReferenceUsage,
) {
    let current = usage_by_name
        .get(name)
        .copied()
        .unwrap_or(ParentFacadeReferenceUsage::None);
    usage_by_name.insert(name.to_string(), merge_reference_usage(current, next));
}

pub(super) fn resolve_module_relative_paths(
    raw: &[String],
    current_module_path: &[String],
) -> Vec<Vec<String>> {
    if raw.is_empty() {
        return Vec::new();
    }

    let Some(path_anchor) = PathAnchor::first(raw) else {
        return Vec::new();
    };
    match path_anchor {
        PathAnchor::Crate => return vec![raw[1..].to_vec()],
        PathAnchor::SelfMod => {
            let mut resolved = current_module_path.to_vec();
            resolved.extend(raw[1..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::Super => {
            let mut index = 0usize;
            let mut resolved = current_module_path.to_vec();
            while raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
            {
                if resolved.pop().is_none() {
                    return Vec::new();
                }
                index += 1;
            }
            if raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::SelfMod)
            {
                index += 1;
            }
            resolved.extend(raw[index..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::SelfType | PathAnchor::Name => {},
    }

    (0..=current_module_path.len())
        .map(|prefix_len| {
            let mut resolved = current_module_path[..prefix_len].to_vec();
            resolved.extend(raw.iter().cloned());
            resolved
        })
        .collect()
}

const fn merge_reference_usage(
    current: ParentFacadeReferenceUsage,
    next: ParentFacadeReferenceUsage,
) -> ParentFacadeReferenceUsage {
    match (current, next) {
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate)
        },
        _ => ParentFacadeReferenceUsage::None,
    }
}

const fn merge_parent_facade_usage(
    current: ParentFacadeUsage,
    next: ParentFacadeUsage,
) -> ParentFacadeUsage {
    match (current, next) {
        (ParentFacadeUsage::UsedOutsideSubtree, _) | (_, ParentFacadeUsage::UsedOutsideSubtree) => {
            ParentFacadeUsage::UsedOutsideSubtree
        },
        (ParentFacadeUsage::UsedInsideSubtreeByRelativePath, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByRelativePath) => {
            ParentFacadeUsage::UsedInsideSubtreeByRelativePath
        },
        (ParentFacadeUsage::UsedInsideSubtreeByRelativeImport, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByRelativeImport) => {
            ParentFacadeUsage::UsedInsideSubtreeByRelativeImport
        },
        (ParentFacadeUsage::UsedInsideSubtreeByCratePath, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByCratePath) => {
            ParentFacadeUsage::UsedInsideSubtreeByCratePath
        },
        (ParentFacadeUsage::UsedInsideSubtreeByCrateImport, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByCrateImport) => {
            ParentFacadeUsage::UsedInsideSubtreeByCrateImport
        },
        _ => ParentFacadeUsage::Unused,
    }
}

fn merge_usage_for_name(
    usage_by_name: &mut ParentFacadeUsageByName,
    name: &str,
    next: ParentFacadeUsage,
) {
    if let Some(current) = usage_by_name.get_mut(name) {
        *current = merge_parent_facade_usage(*current, next);
        return;
    }
    usage_by_name.insert(
        name.to_string(),
        merge_parent_facade_usage(ParentFacadeUsage::Unused, next),
    );
}

/// References inside the child subtree do not require keeping an external annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ExternalChildUsage {
    Present,
    Absent,
}

/// Only references outside the facade subtree contribute external exposure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ExternalFacadeUsage {
    Present,
    Absent,
}

impl ExternalChildUsage {
    fn in_context(
        extracted: &ExtractedPaths,
        context: &ModuleContext,
        child_module_path: &[String],
        item_name: &str,
    ) -> Self {
        if FacadeCallerLocation::for_module(&context.path, child_module_path)
            == FacadeCallerLocation::InsideSubtree
        {
            return Self::Absent;
        }
        match extracted_child_item_reference(
            extracted,
            &context.path,
            &context.suffix,
            child_module_path,
            item_name,
        ) {
            ChildItemReference::Found => Self::Present,
            ChildItemReference::Missing => Self::Absent,
        }
    }
}

impl ExternalFacadeUsage {
    fn in_context(
        extracted: &ExtractedPaths,
        context: &ModuleContext,
        module_path: &[String],
        item_names: &ExportNameIndex<'_>,
    ) -> Self {
        if FacadeCallerLocation::for_module(&context.path, module_path)
            == FacadeCallerLocation::InsideSubtree
        {
            return Self::Absent;
        }
        if source_references_parent_exports(
            extracted,
            &context.path,
            &context.suffix,
            module_path,
            item_names,
        )
        .values()
        .any(|usage| *usage != ParentFacadeReferenceUsage::None)
        {
            Self::Present
        } else {
            Self::Absent
        }
    }
}

pub(in crate::compiler) fn external_child_usage(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    child_module_path: &[String],
    item_name: &str,
) -> ExternalChildUsage {
    for source_file in source_cache.source_files_under(source_root).iter() {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        let module_contexts = active_module_contexts(extracted, module_sources, tcx, source_file);
        for module_context in module_contexts.iter() {
            if ExternalChildUsage::in_context(
                extracted,
                module_context,
                child_module_path,
                item_name,
            ) == ExternalChildUsage::Present
            {
                return ExternalChildUsage::Present;
            }
        }
    }

    ExternalChildUsage::Absent
}

pub(in crate::compiler) fn external_facade_usage(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    module_path: &[String],
    item_names: &[String],
) -> ExternalFacadeUsage {
    let item_names = ExportNameIndex::new(item_names);
    for source_file in source_cache.source_files_under(source_root).iter() {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        let module_contexts = active_module_contexts(extracted, module_sources, tcx, source_file);
        for module_context in module_contexts.iter() {
            if ExternalFacadeUsage::in_context(extracted, module_context, module_path, &item_names)
                == ExternalFacadeUsage::Present
            {
                return ExternalFacadeUsage::Present;
            }
        }
    }
    ExternalFacadeUsage::Absent
}

fn extracted_child_item_reference(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> ChildItemReference {
    if extracted.use_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && resolved_child_item_reference(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            ) == ChildItemReference::Found
    }) || extracted.expr_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && (resolved_child_item_reference(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            ) == ChildItemReference::Found
                || resolve_alias_expr_path(
                    &extracted_path.segments,
                    module_suffix,
                    &extracted.use_renames,
                )
                .is_some_and(|resolved| {
                    resolved_child_item_reference(
                        &resolved,
                        current_module_path,
                        child_module_path,
                        item_name,
                    ) == ChildItemReference::Found
                }))
    }) {
        ChildItemReference::Found
    } else {
        ChildItemReference::Missing
    }
}

fn active_module_contexts(
    extracted: &ExtractedPaths,
    module_sources: &ModuleSourceMap,
    tcx: TyCtxt<'_>,
    source_file: &Path,
) -> Rc<[ModuleContext]> {
    module_sources.module_contexts(source_file, || {
        let mut contexts: Vec<ModuleContext> = Vec::new();
        for root_module in module_sources.root_modules_for_file(tcx, source_file) {
            let root_path = boundary::module_path(tcx, root_module);
            for module_suffix in lexical_module_suffixes(extracted) {
                let mut current_module_path = root_path.clone();
                current_module_path.extend(module_suffix.iter().cloned());
                if module_sources.module_definition_presence(tcx, source_file, &current_module_path)
                    == ModuleDefinitionPresence::Present
                    && !contexts
                        .iter()
                        .any(|context| context.path == current_module_path)
                {
                    contexts.push(ModuleContext {
                        path:   current_module_path,
                        suffix: module_suffix.to_vec(),
                    });
                }
            }
        }
        contexts
    })
}

fn lexical_module_suffixes(extracted: &ExtractedPaths) -> Vec<&[String]> {
    let mut suffixes = Vec::new();
    for extracted_path in extracted.use_paths.iter().chain(&extracted.expr_paths) {
        let module_suffix = extracted_path.module_suffix.as_slice();
        if !suffixes.contains(&module_suffix) {
            suffixes.push(module_suffix);
        }
    }
    suffixes
}

fn resolved_child_item_reference(
    path: &[String],
    current_module_path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> ChildItemReference {
    if resolve_module_relative_paths(path, current_module_path)
        .into_iter()
        .any(|resolved| {
            child_item_reference(&resolved, child_module_path, item_name)
                == ChildItemReference::Found
        })
    {
        ChildItemReference::Found
    } else {
        ChildItemReference::Missing
    }
}

fn child_item_reference(
    path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> ChildItemReference {
    if path.len() > child_module_path.len()
        && path[..child_module_path.len()] == *child_module_path
        && (path[child_module_path.len()] == item_name || path[child_module_path.len()] == "*")
    {
        ChildItemReference::Found
    } else {
        ChildItemReference::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::ChildItemReference;
    use super::ExportNameIndex;
    use super::FacadeCallerLocation;
    use super::LiteralIdentifierMatch;
    use super::ParentFacadeReferenceUsage;
    use super::ParentFacadeUsage;
    use super::PathOrigin;
    use super::matching_export_name_indexed;

    #[test]
    fn external_usage_preserves_scope_name_and_caller_location() -> anyhow::Result<()> {
        let target = ["parent", "child"].map(String::from);
        let names = [String::from("Thing")];
        let index = ExportNameIndex::new(&names);
        for (source, current, suffix, child_usage, facade_usage) in [
            (
                "use crate::parent::child::Thing;",
                vec![],
                vec![],
                super::ExternalChildUsage::Present,
                super::ExternalFacadeUsage::Present,
            ),
            (
                "fn f() { crate::parent::child::Thing::new(); }",
                vec!["sibling"],
                vec![],
                super::ExternalChildUsage::Present,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                "fn f() { let _ = crate::parent::child::Thing; }",
                vec!["sibling"],
                vec![],
                super::ExternalChildUsage::Present,
                super::ExternalFacadeUsage::Present,
            ),
            (
                "use crate::parent::child::Thing;",
                vec!["parent", "child"],
                vec![],
                super::ExternalChildUsage::Absent,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                "use crate::parent::child::Thing;",
                vec!["parent", "child", "nested"],
                vec![],
                super::ExternalChildUsage::Absent,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                concat!(
                    "mod nested { use crate::parent::child as alias; ",
                    "fn f() { let _ = alias::Thing; } }",
                ),
                vec!["nested"],
                vec!["nested"],
                super::ExternalChildUsage::Present,
                super::ExternalFacadeUsage::Present,
            ),
            (
                concat!(
                    "mod nested { use crate::parent::child as alias; } ",
                    "fn f() { let _ = alias::Thing; }",
                ),
                vec![],
                vec![],
                super::ExternalChildUsage::Absent,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                "use crate::parent::child::Else;",
                vec![],
                vec![],
                super::ExternalChildUsage::Absent,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                "mod nested { use crate::parent::child::Thing; }",
                vec![],
                vec![],
                super::ExternalChildUsage::Absent,
                super::ExternalFacadeUsage::Absent,
            ),
            (
                "mod nested { use crate::parent::child::Thing; }",
                vec!["nested"],
                vec!["nested"],
                super::ExternalChildUsage::Present,
                super::ExternalFacadeUsage::Present,
            ),
        ] {
            let extracted = super::source_cache::extract_paths(&syn::parse_file(source)?);
            let context = super::ModuleContext {
                path:   current.into_iter().map(String::from).collect(),
                suffix: suffix.into_iter().map(String::from).collect(),
            };
            assert_eq!(
                super::ExternalChildUsage::in_context(&extracted, &context, &target, "Thing"),
                child_usage,
                "{source} {context:?}",
            );
            assert_eq!(
                super::ExternalFacadeUsage::in_context(&extracted, &context, &target, &index),
                facade_usage,
                "{source} {context:?}",
            );
        }
        Ok(())
    }

    #[test]
    fn path_candidate_filter_keeps_anchor_finals_and_normalized_names() {
        let names = [String::from("r#Thing")];
        let exports = ExportNameIndex::new(&names);
        for (raw, expected) in [
            (vec![], super::ExportPathCandidate::Possible),
            (vec!["crate", "Thing"], super::ExportPathCandidate::Possible),
            (
                vec!["crate", "r#Thing"],
                super::ExportPathCandidate::Possible,
            ),
            (vec!["super", "super"], super::ExportPathCandidate::Possible),
            (vec!["self"], super::ExportPathCandidate::Possible),
            (vec!["crate", "Else"], super::ExportPathCandidate::Excluded),
        ] {
            let raw = raw.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(exports.path_candidate(&raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn file_candidate_filter_keeps_mentions_and_excludes_absent_names() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "use crate::parent::r#Thing;\n")?;
        let cache = super::SourceCache::build_files(std::slice::from_ref(&file))?;
        for (names, expected) in [
            (vec!["Thing"], super::ExportFileCandidate::Possible),
            (
                vec!["Missing", "r#Thing"],
                super::ExportFileCandidate::Possible,
            ),
            (vec!["Missing"], super::ExportFileCandidate::Excluded),
            (vec![], super::ExportFileCandidate::Excluded),
        ] {
            let names = names.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(
                super::export_file_candidate(&cache, &file, &ExportNameIndex::new(&names)),
                expected,
                "{names:?}",
            );
        }
        Ok(())
    }

    #[test]
    fn literal_boundaries_reject_adjoining_names_and_unavailable_bytes() {
        for (source, offset, length, expected) in [
            ("Thing", 0, 5, LiteralIdentifierMatch::Bounded),
            ("(Thing)", 1, 5, LiteralIdentifierMatch::Bounded),
            ("xThing", 1, 5, LiteralIdentifierMatch::Rejected),
            ("Thingx", 0, 5, LiteralIdentifierMatch::Rejected),
            ("éThing", 2, 5, LiteralIdentifierMatch::Rejected),
            ("Thingé", 0, 5, LiteralIdentifierMatch::Rejected),
            ("é", 1, 1, LiteralIdentifierMatch::Rejected),
            ("é", 0, 1, LiteralIdentifierMatch::Rejected),
            ("Thing", 6, 0, LiteralIdentifierMatch::Rejected),
            ("Thing", 0, 6, LiteralIdentifierMatch::Rejected),
        ] {
            assert_eq!(
                super::literal_identifier_match(source, offset, length),
                expected,
                "{source} {offset} {length}",
            );
        }
    }

    #[test]
    fn child_item_references_keep_alias_scope_glob_and_descendant_policy() -> anyhow::Result<()> {
        let child = ["parent", "child"].map(String::from);
        let current = [String::from("parent")];
        for (source, expected) in [
            (
                "use crate::parent::child::Thing;",
                ChildItemReference::Found,
            ),
            ("use crate::parent::child::*;", ChildItemReference::Found),
            (
                "use crate::parent::child::Else;",
                ChildItemReference::Missing,
            ),
            ("fn f() { child::Thing::new(); }", ChildItemReference::Found),
            (
                "use crate::parent::child as alias; fn f() { alias::Thing::new(); }",
                ChildItemReference::Found,
            ),
            (
                "mod nested { use crate::parent::child::Thing; }",
                ChildItemReference::Missing,
            ),
        ] {
            let extracted = super::source_cache::extract_paths(&syn::parse_file(source)?);
            assert_eq!(
                super::extracted_child_item_reference(&extracted, &current, &[], &child, "Thing"),
                expected,
                "{source}",
            );
        }
        for (candidate, expected) in [
            (vec!["parent", "child"], FacadeCallerLocation::InsideSubtree),
            (
                vec!["parent", "child", "nested"],
                FacadeCallerLocation::InsideSubtree,
            ),
            (vec!["parent"], FacadeCallerLocation::OutsideSubtree),
            (
                vec!["parent", "childish"],
                FacadeCallerLocation::OutsideSubtree,
            ),
        ] {
            let candidate = candidate.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(
                FacadeCallerLocation::for_module(&candidate, &child),
                expected
            );
        }
        assert_eq!(
            FacadeCallerLocation::for_module(&child, &[]),
            FacadeCallerLocation::InsideSubtree,
        );
        Ok(())
    }

    #[test]
    fn facade_reference_usage_preserves_location_and_origin() {
        let facade = [String::from("parent")];
        let descendant = [String::from("parent"), String::from("child")];
        let sibling = [String::from("sibling")];
        let cases = [
            (ParentFacadeReferenceUsage::None, ParentFacadeUsage::Unused),
            (
                ParentFacadeReferenceUsage::Import(PathOrigin::Relative),
                ParentFacadeUsage::UsedInsideSubtreeByRelativeImport,
            ),
            (
                ParentFacadeReferenceUsage::Import(PathOrigin::Crate),
                ParentFacadeUsage::UsedInsideSubtreeByCrateImport,
            ),
            (
                ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative),
                ParentFacadeUsage::UsedInsideSubtreeByRelativePath,
            ),
            (
                ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate),
                ParentFacadeUsage::UsedInsideSubtreeByCratePath,
            ),
        ];
        for (reference, expected) in cases {
            assert_eq!(
                FacadeCallerLocation::for_module(&facade, &facade).usage(reference),
                expected
            );
            assert_eq!(
                FacadeCallerLocation::for_module(&descendant, &facade).usage(reference),
                expected
            );
        }
        let outside = FacadeCallerLocation::for_module(&sibling, &facade);
        assert_eq!(
            outside.usage(ParentFacadeReferenceUsage::None),
            ParentFacadeUsage::Unused
        );
        for origin in [PathOrigin::Relative, PathOrigin::Crate] {
            assert_eq!(
                outside.usage(ParentFacadeReferenceUsage::Import(origin)),
                ParentFacadeUsage::UsedOutsideSubtree
            );
            assert_eq!(
                outside.usage(ParentFacadeReferenceUsage::DirectPath(origin)),
                ParentFacadeUsage::UsedOutsideSubtree
            );
        }
    }

    #[test]
    fn literal_paths_walk_checked_endpoints_through_trivia_and_raw_identifiers() {
        let module_path = ["type", "café"].map(String::from);
        for (source, expected) in [
            ("crate::r#type::café::Thing", Some(0)),
            (
                "x crate /* c */ :: r#type :: café // line\n :: Thing",
                Some(2),
            ),
            ("crate::wrong::café::Thing", None),
            ("other::r#type::café::Thing", None),
            ("crate::r#type::café Thing", None),
            ("Thing", None),
        ] {
            let regions = crate::rust_syntax::LexicalRegions::from(source.to_owned());
            let result = source
                .find("Thing")
                .and_then(|offset| regions.endpoint(offset))
                .and_then(|end| super::literal_crate_path_start(end, &module_path))
                .map(crate::rust_syntax::SourceEndpoint::offset);
            assert_eq!(result, expected, "{source}");
        }
        let root = crate::rust_syntax::LexicalRegions::from("crate :: Thing".to_owned());
        assert_eq!(
            root.endpoint(9)
                .and_then(|end| super::literal_crate_path_start(end, &[]))
                .map(crate::rust_syntax::SourceEndpoint::offset),
            Some(0),
        );
        assert!(root.endpoint(root.source().len() + 1).is_none());
    }

    #[test]
    fn parsed_path_matches_raw_module_segments() {
        let source_segments = ["crate", "a", "r#type", "r#inner", "Thing"].map(String::from);
        let compiler_segments = ["a", "type", "inner"].map(String::from);
        let exported_names = [String::from("Thing")];

        assert_eq!(
            matching_export_name_indexed(
                &source_segments,
                &[],
                &compiler_segments,
                &ExportNameIndex::new(&exported_names),
            ),
            Some("Thing")
        );
    }

    #[test]
    fn raw_identifier_export_matches_a_plainly_spelled_final_segment() {
        let source_segments = ["crate", "a", "Thing"].map(String::from);
        let compiler_segments = ["a"].map(String::from);
        let exported_names = [String::from("r#Thing")];

        assert_eq!(
            matching_export_name_indexed(
                &source_segments,
                &[],
                &compiler_segments,
                &ExportNameIndex::new(&exported_names),
            ),
            Some("Thing")
        );
    }

    #[test]
    fn super_relative_path_resolves_against_the_current_module() {
        let source_segments = ["super", "Thing"].map(String::from);
        let current_module_path = ["a", "b"].map(String::from);
        let compiler_segments = ["a"].map(String::from);
        let exported_names = [String::from("Thing")];

        assert_eq!(
            matching_export_name_indexed(
                &source_segments,
                &current_module_path,
                &compiler_segments,
                &ExportNameIndex::new(&exported_names),
            ),
            Some("Thing")
        );
    }

    #[test]
    fn path_whose_final_segment_is_not_exported_does_not_match() {
        let source_segments = ["crate", "a", "Other"].map(String::from);
        let compiler_segments = ["a"].map(String::from);
        let exported_names = [String::from("Thing")];

        assert_eq!(
            matching_export_name_indexed(
                &source_segments,
                &[],
                &compiler_segments,
                &ExportNameIndex::new(&exported_names),
            ),
            None
        );
    }
}
