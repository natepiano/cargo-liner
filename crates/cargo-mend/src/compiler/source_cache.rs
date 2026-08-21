use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::Delimiter;
use proc_macro2::Group;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use syn::Attribute;
use syn::Expr;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::Lit;
use syn::Macro;
use syn::Meta;
use syn::MetaList;
use syn::Path as SynPath;
use syn::Token;
use syn::UseTree;
use syn::ext::IdentExt;
use syn::parse_file;
use syn::punctuated::Punctuated;
use syn::visit;
use syn::visit::Visit;

use super::constants::SOURCE_DIR_BENCHES;
use super::constants::SOURCE_DIR_EXAMPLES;
use super::constants::SOURCE_DIR_SRC;
use super::constants::SOURCE_DIR_TESTS;
use super::sweep_counters;
use crate::reporting::AllFeaturesCoverage;
use crate::rust_syntax::LexicalRegions;
use crate::rust_syntax::PathAnchor;
#[cfg(test)]
use crate::selection::CARGO_TARGET_KIND_LIB;
#[cfg(test)]
use crate::selection::CARGO_TARGET_KIND_MAIN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathOrigin {
    Relative,
    Crate,
}

pub(super) struct ExtractedPaths {
    /// Flattened use-tree paths with their origin and lexical inline module.
    pub use_paths:         Vec<ExtractedPath>,
    /// All `syn::Path` nodes with their origin and lexical inline module.
    pub expr_paths:        Vec<ExtractedPath>,
    /// Module-level renames (`use path::to::module as alias`): maps alias → original path.
    pub use_renames:       Vec<UseRename>,
    /// `use_paths` and `expr_paths` grouped by the name each could resolve to.
    pub export_candidates: ExportCandidateIndex,
}

/// The file's paths grouped by the export name each one could name.
///
/// A facade scan asks whether a file references one of a parent module's
/// exports, and runs once per re-export occurrence in the crate. Answering by
/// walking every path in the file costs `re-exports × files × paths-per-file`,
/// which dominated whole-crate runs. Grouping by final segment turns each
/// answer into one lookup per exported name.
pub(super) struct ExportCandidateIndex {
    pub use_paths:  PathsByFinalSegment,
    pub expr_paths: PathsByFinalSegment,
}

/// Indices into one of [`ExtractedPaths`]'s path lists, grouped by the final
/// segment of each path.
pub(super) struct PathsByFinalSegment {
    by_segment:   FxHashMap<String, Vec<usize>>,
    /// Indices whose path is empty or ends in `crate`, `self`, or `super`.
    /// Resolution replaces that segment with one taken from the current module
    /// path, so the name such a path reaches is not visible here and every
    /// lookup has to include them.
    anchor_final: Vec<usize>,
}

impl PathsByFinalSegment {
    /// The indices of every path that could name one of `segments`.
    ///
    /// A path can appear twice when two of `segments` reach it — through its own
    /// final segment and through a `use ... as` alias. Callers merge by name, so
    /// visiting one twice yields the same result as visiting it once.
    pub fn candidates<'index>(
        &'index self,
        segments: impl Iterator<Item = &'index str> + 'index,
    ) -> impl Iterator<Item = usize> + 'index {
        segments
            .filter_map(|segment| self.by_segment.get(segment))
            .flatten()
            .chain(&self.anchor_final)
            .copied()
    }

    fn insert(&mut self, index: usize, final_segment: Option<&str>) {
        match final_segment.map(PathAnchor::from) {
            Some(PathAnchor::SelfType | PathAnchor::Name) => {
                // `Self` is retained by resolution like any other name, so it is
                // grouped rather than treated as an anchor; no export can be
                // named `Self`, so its group is never looked up.
                let Some(segment) = final_segment else {
                    return;
                };
                self.by_segment
                    .entry(normalized_name(segment).to_string())
                    .or_default()
                    .push(index);
            },
            Some(PathAnchor::Crate | PathAnchor::Super | PathAnchor::SelfMod) | None => {
                self.anchor_final.push(index);
            },
        }
    }
}

pub(super) struct ExtractedPath {
    pub segments:      Vec<String>,
    pub origin:        PathOrigin,
    pub module_suffix: Vec<String>,
}

pub(super) struct UseRename {
    pub alias:         String,
    pub original_path: Vec<String>,
    pub module_suffix: Vec<String>,
}

/// Whether a name can appear anywhere in a source file.
///
/// The answer comes from a hash set, so `Present` is approximate — a hash
/// collision reports it for a name the file does not contain, and the caller
/// still has to run the real scan. `Absent` is exact: the name occurs nowhere in
/// the file's text, so no scan of that file can match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameMention {
    Present,
    Absent,
}

pub(super) struct SourceCache {
    contents:                       FxHashMap<PathBuf, String>,
    files_by_dir:                   FxHashMap<PathBuf, Vec<PathBuf>>,
    parsed:                         FxHashMap<PathBuf, File>,
    extracted_paths:                FxHashMap<PathBuf, ExtractedPaths>,
    mentionable_names:              FxHashMap<PathBuf, FxHashSet<u64>>,
    lexical_regions:                FxHashMap<PathBuf, LexicalRegions>,
    structural_parent_module_paths: FxHashMap<PathBuf, Vec<Vec<String>>>,
    all_features_coverage:          AllFeaturesCoverage,
    /// Memo for [`SourceCache::source_files_under`]; see that method for why the
    /// same directory is asked for hundreds of thousands of times.
    files_under:                    RefCell<FxHashMap<PathBuf, Rc<[PathBuf]>>>,
}

impl SourceCache {
    #[cfg(test)]
    pub fn build(roots: &[&Path], target_directory: &Path) -> Result<Self> {
        let mut source_files = Vec::new();
        for root in roots {
            source_files.extend(rust_source_files(root, target_directory)?);
        }
        Self::build_files(&source_files)
    }

    pub fn build_files(source_files: &[PathBuf]) -> Result<Self> {
        let mut contents = FxHashMap::default();
        for file in source_files {
            contents.entry(file.clone()).or_insert(
                fs::read_to_string(file).with_context(|| {
                    format!("failed to pre-read source file {}", file.display())
                })?,
            );
        }
        let mut files_by_dir: FxHashMap<PathBuf, Vec<PathBuf>> = FxHashMap::default();
        for path in contents.keys() {
            if let Some(parent) = path.parent() {
                files_by_dir
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(path.clone());
            }
        }
        let mut parsed = FxHashMap::default();
        for (path, source) in &contents {
            if let Ok(ast) = parse_file(source) {
                parsed.insert(path.clone(), ast);
            }
        }
        let mut extracted_paths = FxHashMap::default();
        for (path, ast) in &parsed {
            extracted_paths.insert(path.clone(), extract_paths(ast));
        }
        // Only the name index and the lexical scan cross threads: both read
        // `&str` and return owned data. `syn::File` is not `Send` —
        // `proc_macro2` embeds a `PhantomData<Rc<()>>` marker in its spans and
        // builds token streams on `Rc<Vec<TokenTree>>`, both unconditional — so
        // parsing and path extraction stay sequential. Walking a `Vec` snapshot
        // and zipping the results back keeps the insertion order identical to
        // the sequential build, because rayon's indexed `collect()` preserves
        // order.
        let content_entries: Vec<(&PathBuf, &String)> = contents.iter().collect();
        let scans: Vec<(FxHashSet<u64>, LexicalRegions)> = content_entries
            .par_iter()
            .map(|(_, source)| {
                (
                    mentionable_names_in(source),
                    LexicalRegions::from(source.as_str()),
                )
            })
            .collect();
        let mut mentionable_names = FxHashMap::default();
        let mut lexical_regions = FxHashMap::default();
        for ((path, _), (names, regions)) in content_entries.iter().zip(scans) {
            mentionable_names.insert((*path).clone(), names);
            lexical_regions.insert((*path).clone(), regions);
        }
        let all_features_coverage = source_all_features_coverage(&contents, &parsed);
        Ok(Self {
            contents,
            files_by_dir,
            parsed,
            extracted_paths,
            mentionable_names,
            lexical_regions,
            structural_parent_module_paths: FxHashMap::default(),
            all_features_coverage,
            files_under: RefCell::new(FxHashMap::default()),
        })
    }

    pub fn build_crate(crate_root_file: &Path, compiler_files: &[PathBuf]) -> Result<Self> {
        let crate_root_file =
            fs::canonicalize(crate_root_file).unwrap_or_else(|_| crate_root_file.to_path_buf());
        let mut source_files = compiler_files
            .iter()
            .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            .collect::<Vec<_>>();
        let mut visited = FxHashSet::default();
        let mut structural_parent_module_paths = FxHashMap::default();
        collect_declared_module_files(
            &crate_root_file,
            crate_root_file
                .parent()
                .unwrap_or(crate_root_file.as_path()),
            &[],
            &mut visited,
            &mut source_files,
            &mut structural_parent_module_paths,
        )?;
        let mut source_cache = Self::build_files(&source_files)?;
        source_cache.structural_parent_module_paths = structural_parent_module_paths;
        Ok(source_cache)
    }

    pub fn source_files(&self) -> Vec<&Path> {
        self.contents.keys().map(PathBuf::as_path).collect()
    }

    /// The source files under `dir`, collected once per directory.
    ///
    /// The exposure scan asks for the crate root once per analyzed item — over
    /// 395,000 times on a 168-file crate — and the answer cannot change, because
    /// the cache is immutable for the run. Without the memo each call rescanned
    /// `files_by_dir` with a path-prefix comparison per directory and collected
    /// the same list into a fresh allocation.
    ///
    /// Callers get an [`Rc`] rather than a borrow so the memo's [`RefCell`] is
    /// not held across the loop bodies, which call back into analysis and reach
    /// this function again.
    pub fn source_files_under(&self, dir: &Path) -> Rc<[PathBuf]> {
        sweep_counters::record_file_list_request();
        if let Some(files) = self.files_under.borrow().get(dir) {
            return Rc::clone(files);
        }

        let files: Rc<[PathBuf]> = self
            .files_by_dir
            .iter()
            .filter(|(candidate, _)| candidate.starts_with(dir))
            .flat_map(|(_, files)| files.iter().cloned())
            .collect();
        sweep_counters::record_file_list_build(files.len());
        self.files_under
            .borrow_mut()
            .insert(dir.to_path_buf(), Rc::clone(&files));
        files
    }

    pub fn read_source(&self, path: &Path) -> Result<&str> {
        self.contents
            .get(path)
            .map(String::as_str)
            .with_context(|| format!("source file not in cache: {}", path.display()))
    }

    pub fn parsed_file(&self, path: &Path) -> Option<&File> { self.parsed.get(path) }

    /// Where the comments and literals in `path` are, scanned once at build
    /// time.
    ///
    /// A text scan that walks backward out of a match — over `::` separators and
    /// the whitespace around them — needs to know whether a byte is code, and
    /// that answer only comes from a lex that starts at the front of the file.
    /// Sharing one scan keeps the whole-crate sweep from re-lexing a file per
    /// question asked of it.
    pub fn lexical_regions(&self, path: &Path) -> Result<&LexicalRegions> {
        self.lexical_regions
            .get(path)
            .with_context(|| format!("source file not in cache: {}", path.display()))
    }

    /// Whether `name` can be mentioned anywhere in `path`.
    ///
    /// Callers that scan a file's syntax tree for one name use this first: the
    /// scan walks every item in the file, so skipping a file that cannot match
    /// turns a whole-crate sweep into a handful of files. A path the cache never
    /// indexed answers [`NameMention::Present`] so the caller still scans it.
    pub fn name_mention(&self, path: &Path, name: &str) -> NameMention {
        self.mentionable_names
            .get(path)
            .filter(|names| !names.contains(&name_hash(name)))
            .map_or(NameMention::Present, |_| NameMention::Absent)
    }

    pub fn extracted_paths(&self, path: &Path) -> Option<&ExtractedPaths> {
        self.extracted_paths.get(path)
    }

    pub const fn structural_parent_module_paths(&self) -> &FxHashMap<PathBuf, Vec<Vec<String>>> {
        &self.structural_parent_module_paths
    }

    pub const fn all_features_coverage(&self) -> AllFeaturesCoverage { self.all_features_coverage }
}

/// Hash every name that a scan of `source` could compare an item name against.
///
/// Two forms reach such a comparison. A path segment or attribute identifier is
/// spelled verbatim, so the identifier-shaped words of the text cover it. A
/// literal is compared after
/// `to_string().trim_matches('"').trim_matches('r').trim_matches('#')`, and only
/// the `'r'` trim can reach inside a word — `"error"` is compared as `erro` —
/// so each word contributes its trimmed form as well.
fn mentionable_names_in(source: &str) -> FxHashSet<u64> {
    source
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .flat_map(|word| [word, word.trim_matches('r')])
        .map(name_hash)
        .collect()
}

fn name_hash(name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

fn collect_declared_module_files(
    source_file: &Path,
    module_directory: &Path,
    module_path: &[String],
    visited: &mut FxHashSet<PathBuf>,
    source_files: &mut Vec<PathBuf>,
    structural_parent_module_paths: &mut FxHashMap<PathBuf, Vec<Vec<String>>>,
) -> Result<()> {
    let source_file = fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf());
    if !visited.insert(source_file.clone()) {
        return Ok(());
    }
    if !source_files.contains(&source_file) {
        source_files.push(source_file.clone());
    }

    let source = fs::read_to_string(&source_file)
        .with_context(|| format!("failed to read source file {}", source_file.display()))?;
    let Ok(file) = parse_file(&source) else {
        return Ok(());
    };
    collect_module_items(
        &file.items,
        module_directory,
        module_path,
        visited,
        source_files,
        structural_parent_module_paths,
    )
}

fn collect_module_items(
    items: &[Item],
    module_directory: &Path,
    module_path: &[String],
    visited: &mut FxHashSet<PathBuf>,
    source_files: &mut Vec<PathBuf>,
    structural_parent_module_paths: &mut FxHashMap<PathBuf, Vec<Vec<String>>>,
) -> Result<()> {
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        let module_name = item_mod.ident.unraw().to_string();
        let mut child_module_path = module_path.to_vec();
        child_module_path.push(module_name.clone());
        if let Some((_, child_items)) = &item_mod.content {
            collect_module_items(
                child_items,
                &module_directory.join(module_name),
                &child_module_path,
                visited,
                source_files,
                structural_parent_module_paths,
            )?;
            continue;
        }

        for module_file in external_module_files(item_mod, module_directory) {
            if !module_file.is_file() {
                continue;
            }
            let child_directory =
                if module_file.file_name().and_then(OsStr::to_str) == Some("mod.rs") {
                    module_file
                        .parent()
                        .map_or_else(|| module_directory.to_path_buf(), Path::to_path_buf)
                } else {
                    module_file.with_extension("")
                };
            let canonical_module_file =
                fs::canonicalize(&module_file).unwrap_or_else(|_| module_file.clone());
            let parent_paths = structural_parent_module_paths
                .entry(canonical_module_file.clone())
                .or_default();
            if !parent_paths.iter().any(|path| path == module_path) {
                parent_paths.push(module_path.to_vec());
            }
            collect_declared_module_files(
                &canonical_module_file,
                &child_directory,
                &child_module_path,
                visited,
                source_files,
                structural_parent_module_paths,
            )?;
        }
    }
    Ok(())
}

fn external_module_files(item_mod: &ItemMod, module_directory: &Path) -> Vec<PathBuf> {
    let module_name = item_mod.ident.unraw().to_string();
    let direct_paths = item_mod
        .attrs
        .iter()
        .filter_map(direct_path_attribute)
        .map(|path| module_directory.join(path))
        .collect::<Vec<_>>();
    let mut candidates = if direct_paths.is_empty() {
        vec![
            module_directory.join(format!("{module_name}.rs")),
            module_directory.join(module_name).join("mod.rs"),
        ]
    } else {
        direct_paths
    };
    for path in item_mod.attrs.iter().flat_map(conditional_path_attributes) {
        let candidate = module_directory.join(path);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn direct_path_attribute(attribute: &Attribute) -> Option<String> {
    if !attribute.path().is_ident("path") {
        return None;
    }
    path_from_meta(&attribute.meta)
}

fn conditional_path_attributes(attribute: &Attribute) -> Vec<String> {
    if !attribute.path().is_ident("cfg_attr") {
        return Vec::new();
    }
    let Meta::List(list) = &attribute.meta else {
        return Vec::new();
    };
    let Ok(metas) = parse_meta_list(list) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for meta in metas.iter().skip(1) {
        collect_conditional_paths(meta, &mut paths);
    }
    paths
}

fn collect_conditional_paths(meta: &Meta, paths: &mut Vec<String>) {
    if meta.path().is_ident("path") {
        if let Some(path) = path_from_meta(meta) {
            paths.push(path);
        }
        return;
    }
    if !meta.path().is_ident("cfg_attr") {
        return;
    }
    let Meta::List(list) = meta else {
        return;
    };
    let Ok(metas) = parse_meta_list(list) else {
        return;
    };
    for nested in metas.iter().skip(1) {
        collect_conditional_paths(nested, paths);
    }
}

fn path_from_meta(meta: &Meta) -> Option<String> {
    let Meta::NameValue(name_value) = meta else {
        return None;
    };
    let Expr::Lit(expr_lit) = &name_value.value else {
        return None;
    };
    let Lit::Str(path) = &expr_lit.lit else {
        return None;
    };
    Some(path.value())
}

fn parse_meta_list(list: &MetaList) -> syn::Result<Punctuated<Meta, Token![,]>> {
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
}

fn source_all_features_coverage(
    contents: &FxHashMap<PathBuf, String>,
    parsed: &FxHashMap<PathBuf, File>,
) -> AllFeaturesCoverage {
    if contents.len() != parsed.len() {
        return AllFeaturesCoverage::NotGuaranteed;
    }
    let mut visitor = NegatedFeatureGateVisitor {
        coverage: AllFeaturesCoverage::Superset,
    };
    for file in parsed.values() {
        visitor.visit_file(file);
    }
    visitor.coverage
}

struct NegatedFeatureGateVisitor {
    coverage: AllFeaturesCoverage,
}

impl<'ast> Visit<'ast> for NegatedFeatureGateVisitor {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.coverage = self
            .coverage
            .merge(attribute_all_features_coverage(attribute));
        visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, item_macro: &'ast Macro) {
        self.coverage = self
            .coverage
            .merge(macro_tokens_all_features_coverage(&item_macro.tokens));
        visit::visit_macro(self, item_macro);
    }
}

fn macro_tokens_all_features_coverage(tokens: &TokenStream) -> AllFeaturesCoverage {
    let token_trees = tokens.clone().into_iter().collect::<Vec<_>>();
    token_trees.iter().enumerate().fold(
        AllFeaturesCoverage::Superset,
        |coverage, (index, token_tree)| {
            let nested_coverage = match token_tree {
                TokenTree::Group(group) => macro_tokens_all_features_coverage(&group.stream()),
                TokenTree::Punct(punct) if punct.as_char() == '#' => token_trees
                    .get(index + 1)
                    .and_then(token_attribute_group)
                    .map_or(
                        AllFeaturesCoverage::Superset,
                        token_attribute_all_features_coverage,
                    ),
                TokenTree::Ident(_) | TokenTree::Punct(_) | TokenTree::Literal(_) => {
                    AllFeaturesCoverage::Superset
                },
            };
            coverage.merge(nested_coverage)
        },
    )
}

fn token_attribute_group(token_tree: &TokenTree) -> Option<&Group> {
    let TokenTree::Group(group) = token_tree else {
        return None;
    };
    (group.delimiter() == Delimiter::Bracket).then_some(group)
}

fn token_attribute_all_features_coverage(group: &Group) -> AllFeaturesCoverage {
    syn::parse2::<Meta>(group.stream()).map_or(AllFeaturesCoverage::Superset, |meta| {
        attribute_meta_all_features_coverage(&meta)
    })
}

fn attribute_all_features_coverage(attribute: &Attribute) -> AllFeaturesCoverage {
    attribute_meta_all_features_coverage(&attribute.meta)
}

fn attribute_meta_all_features_coverage(meta: &Meta) -> AllFeaturesCoverage {
    let Meta::List(list) = meta else {
        return AllFeaturesCoverage::Superset;
    };
    if meta.path().is_ident("cfg") {
        return cfg_predicates_all_features_coverage(list);
    }
    if meta.path().is_ident("cfg_attr") {
        return cfg_attr_all_features_coverage(list);
    }
    AllFeaturesCoverage::Superset
}

fn cfg_predicates_all_features_coverage(list: &MetaList) -> AllFeaturesCoverage {
    let Ok(metas) = parse_meta_list(list) else {
        return AllFeaturesCoverage::NotGuaranteed;
    };
    metas
        .iter()
        .fold(AllFeaturesCoverage::Superset, |coverage, meta| {
            coverage.merge(cfg_predicate_all_features_coverage(
                meta,
                PredicatePolarity::Positive,
            ))
        })
}

fn cfg_attr_all_features_coverage(list: &MetaList) -> AllFeaturesCoverage {
    let Ok(metas) = parse_meta_list(list) else {
        return AllFeaturesCoverage::NotGuaranteed;
    };
    let Some(predicate) = metas.first() else {
        return AllFeaturesCoverage::NotGuaranteed;
    };
    metas.iter().skip(1).fold(
        cfg_predicate_all_features_coverage(predicate, PredicatePolarity::Positive),
        |coverage, meta| coverage.merge(attribute_meta_all_features_coverage(meta)),
    )
}

#[derive(Clone, Copy)]
enum PredicatePolarity {
    Positive,
    Negated,
}

fn cfg_predicate_all_features_coverage(
    meta: &Meta,
    polarity: PredicatePolarity,
) -> AllFeaturesCoverage {
    if meta.path().is_ident("feature") {
        return match polarity {
            PredicatePolarity::Positive => AllFeaturesCoverage::Superset,
            PredicatePolarity::Negated => AllFeaturesCoverage::NotGuaranteed,
        };
    }
    let Meta::List(list) = meta else {
        return AllFeaturesCoverage::Superset;
    };
    let Ok(metas) = parse_meta_list(list) else {
        return AllFeaturesCoverage::NotGuaranteed;
    };
    let nested_polarity = if meta.path().is_ident("not") {
        PredicatePolarity::Negated
    } else {
        polarity
    };
    metas
        .iter()
        .fold(AllFeaturesCoverage::Superset, |coverage, nested| {
            coverage.merge(cfg_predicate_all_features_coverage(nested, nested_polarity))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseItemPosition {
    Outside,
    Inside,
}

struct PathExtractor {
    use_paths:       Vec<ExtractedPath>,
    expr_paths:      Vec<ExtractedPath>,
    use_renames:     Vec<UseRename>,
    inside_use_item: UseItemPosition,
    inline_modules:  Vec<String>,
}

impl<'ast> Visit<'ast> for PathExtractor {
    fn visit_item_use(&mut self, item_use: &'ast ItemUse) {
        let mut flat = Vec::new();
        flatten_use_tree(Vec::new(), &item_use.tree, &mut flat);
        for segments in flat {
            let origin = path_origin(&segments);
            self.use_paths.push(ExtractedPath {
                segments,
                origin,
                module_suffix: self.inline_modules.clone(),
            });
        }
        extract_use_renames(
            Vec::new(),
            &item_use.tree,
            &self.inline_modules,
            &mut self.use_renames,
        );
        self.inside_use_item = UseItemPosition::Inside;
        visit::visit_item_use(self, item_use);
        self.inside_use_item = UseItemPosition::Outside;
    }

    fn visit_item_mod(&mut self, item_mod: &'ast ItemMod) {
        if item_mod.content.is_none() {
            visit::visit_item_mod(self, item_mod);
            return;
        }
        self.inline_modules.push(item_mod.ident.unraw().to_string());
        visit::visit_item_mod(self, item_mod);
        self.inline_modules.pop();
    }

    fn visit_path(&mut self, path: &'ast SynPath) {
        if self.inside_use_item == UseItemPosition::Outside {
            let segments: Vec<String> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let origin = path_origin(&segments);
            self.expr_paths.push(ExtractedPath {
                segments,
                origin,
                module_suffix: self.inline_modules.clone(),
            });
        }
        visit::visit_path(self, path);
    }
}

pub(super) fn analysis_source_root_for(
    crate_root_file: &Path,
    package_root: &Path,
) -> Option<PathBuf> {
    let source_root = crate_root_file.parent()?.to_path_buf();
    let canonical_crate_root =
        fs::canonicalize(crate_root_file).unwrap_or_else(|_| crate_root_file.to_path_buf());
    let canonical_package_root =
        fs::canonicalize(package_root).unwrap_or_else(|_| package_root.to_path_buf());
    let relative = canonical_crate_root
        .strip_prefix(&canonical_package_root)
        .ok()?;
    let first_component = relative.components().next()?.as_os_str().to_str()?;
    [
        SOURCE_DIR_SRC,
        SOURCE_DIR_EXAMPLES,
        SOURCE_DIR_TESTS,
        SOURCE_DIR_BENCHES,
    ]
    .contains(&first_component)
    .then_some(source_root)
}

#[cfg(test)]
pub(super) fn module_path_from_boundary_file(
    source_root: &Path,
    boundary_file: &Path,
) -> Option<Vec<String>> {
    let relative = boundary_file.strip_prefix(source_root).ok()?;
    let mut components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let last = components.last_mut()?;
    *last = last.strip_suffix(".rs")?.to_string();
    if matches!(
        components.as_slice(),
        [name] if name == CARGO_TARGET_KIND_LIB || name == CARGO_TARGET_KIND_MAIN
    ) {
        Some(Vec::new())
    } else {
        Some(components)
    }
}

#[cfg(test)]
pub(super) fn module_path_from_source_file(
    source_root: &Path,
    source_file: &Path,
) -> Option<Vec<String>> {
    if source_file.file_name().and_then(OsStr::to_str) == Some("mod.rs") {
        module_path_from_dir(source_root, source_file.parent()?)
    } else {
        module_path_from_boundary_file(source_root, source_file)
    }
}

#[cfg(test)]
pub(super) fn module_path_from_dir(source_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(source_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

pub(super) fn flatten_use_tree(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            flatten_use_tree(next, &path.tree, out);
        },
        UseTree::Name(name) => {
            let mut next = prefix;
            next.push(name.ident.to_string());
            out.push(next);
        },
        UseTree::Rename(rename) => {
            let mut next = prefix;
            next.push(rename.ident.to_string());
            next.push(rename.rename.to_string());
            out.push(next);
        },
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(prefix.clone(), item, out);
            }
        },
        UseTree::Glob(_) => {
            let mut next = prefix;
            next.push("*".to_string());
            out.push(next);
        },
    }
}

#[cfg(test)]
fn rust_source_files(source_root: &Path, target_directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(source_root, target_directory, &mut files)?;
    Ok(files)
}

#[cfg(test)]
fn collect_rust_source_files(
    dir: &Path,
    target_directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path == target_directory {
            continue;
        }
        if path.is_dir() {
            collect_rust_source_files(&path, target_directory, files)?;
        } else if path.extension().and_then(OsStr::to_str) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

pub(super) fn path_origin(raw: &[String]) -> PathOrigin {
    match PathAnchor::first(raw) {
        Some(PathAnchor::Crate) => PathOrigin::Crate,
        Some(PathAnchor::Super | PathAnchor::SelfMod | PathAnchor::SelfType | PathAnchor::Name)
        | None => PathOrigin::Relative,
    }
}

pub(super) fn extract_paths(file: &File) -> ExtractedPaths {
    let mut extractor = PathExtractor {
        use_paths:       Vec::new(),
        expr_paths:      Vec::new(),
        use_renames:     Vec::new(),
        inside_use_item: UseItemPosition::Outside,
        inline_modules:  Vec::new(),
    };
    extractor.visit_file(file);

    let export_candidates = ExportCandidateIndex {
        use_paths:  index_by_final_segment(&extractor.use_paths, &[]),
        expr_paths: index_by_final_segment(&extractor.expr_paths, &extractor.use_renames),
    };
    ExtractedPaths {
        use_paths: extractor.use_paths,
        expr_paths: extractor.expr_paths,
        use_renames: extractor.use_renames,
        export_candidates,
    }
}

/// `name` with any raw-identifier `r#` prefix removed, so that `r#type` and
/// `type` group and compare as the same name.
pub(super) fn normalized_name(name: &str) -> &str { name.strip_prefix("r#").unwrap_or(name) }

/// Groups `paths` by the final segment each one could resolve to.
///
/// A single-segment path that is a module alias also reaches the final segment
/// of the path the alias stands for, so it is grouped under both. A longer
/// aliased path keeps its own final segment — the alias only rewrites the first.
fn index_by_final_segment(paths: &[ExtractedPath], renames: &[UseRename]) -> PathsByFinalSegment {
    let mut index = PathsByFinalSegment {
        by_segment:   FxHashMap::default(),
        anchor_final: Vec::new(),
    };
    for (path_index, path) in paths.iter().enumerate() {
        index.insert(path_index, path.segments.last().map(String::as_str));
        let [alias] = path.segments.as_slice() else {
            continue;
        };
        let aliased_final = renames
            .iter()
            .find(|rename| rename.module_suffix == path.module_suffix && rename.alias == *alias)
            .map(|rename| rename.original_path.last().map(String::as_str));
        if let Some(aliased_final) = aliased_final
            && aliased_final != Some(alias.as_str())
        {
            index.insert(path_index, aliased_final);
        }
    }
    index
}

fn extract_use_renames(
    prefix: Vec<String>,
    tree: &UseTree,
    module_suffix: &[String],
    out: &mut Vec<UseRename>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            extract_use_renames(next, &path.tree, module_suffix, out);
        },
        UseTree::Rename(rename) => {
            let mut original_path = prefix;
            original_path.push(rename.ident.to_string());
            out.push(UseRename {
                alias: rename.rename.to_string(),
                original_path,
                module_suffix: module_suffix.to_vec(),
            });
        },
        UseTree::Group(group) => {
            for item in &group.items {
                extract_use_renames(prefix.clone(), item, module_suffix, out);
            }
        },
        UseTree::Name(_) | UseTree::Glob(_) => {},
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::SourceCache;
    use super::analysis_source_root_for;
    use super::module_path_from_source_file;
    use crate::reporting::AllFeaturesCoverage;

    #[test]
    fn source_cache_excludes_cargo_target_directory() -> Result<()> {
        let temp = tempdir()?;
        let workspace_root = temp.path().join("workspace");
        let source_directory = workspace_root.join("app/src");
        let target_directory = workspace_root.join("target");
        let source_file = source_directory.join("lib.rs");
        let generated_source_directory = target_directory.join("debug/build");
        let generated_source_file = generated_source_directory.join("generated.rs");
        fs::create_dir_all(&source_directory)?;
        fs::create_dir_all(&generated_source_directory)?;
        fs::write(&source_file, "pub fn workspace_source() {}\n")?;
        fs::write(&generated_source_file, "pub fn generated_source() {}\n")?;

        let source_cache = SourceCache::build(&[&workspace_root], &target_directory)?;

        assert!(source_cache.read_source(&source_file).is_ok());
        assert!(source_cache.read_source(&generated_source_file).is_err());
        Ok(())
    }

    #[test]
    fn crate_cache_includes_feature_gated_module_without_sibling_binary() -> Result<()> {
        assert_crate_cache_includes_declared_module(
            "#[cfg(feature = \"hidden\")]\nmod hidden;\n",
            "hidden.rs",
        )
    }

    #[test]
    fn crate_cache_includes_cfg_test_module_without_sibling_binary() -> Result<()> {
        assert_crate_cache_includes_declared_module("#[cfg(test)]\nmod tests;\n", "tests.rs")
    }

    #[test]
    fn macro_body_negated_feature_prevents_all_features_coverage() -> Result<()> {
        assert_macro_body_coverage(
            r#"macro_rules! emit {
    () => {
        #[cfg(not(feature = "hidden"))]
        fn generated() {}
    };
}
emit!();
"#,
            AllFeaturesCoverage::NotGuaranteed,
        )
    }

    #[test]
    fn macro_body_non_feature_negation_preserves_all_features_coverage() -> Result<()> {
        assert_macro_body_coverage(
            r"macro_rules! emit {
    () => {
        #[cfg(not(unix))]
        fn generated() {}
    };
}
emit!();
",
            AllFeaturesCoverage::Superset,
        )
    }

    fn assert_macro_body_coverage(
        crate_root_source: &str,
        expected_coverage: AllFeaturesCoverage,
    ) -> Result<()> {
        let temp = tempdir()?;
        let source_directory = temp.path().join("src");
        fs::create_dir_all(&source_directory)?;
        let crate_root = source_directory.join("lib.rs");
        fs::write(&crate_root, crate_root_source)?;

        let source_cache =
            SourceCache::build_crate(&crate_root, std::slice::from_ref(&crate_root))?;

        assert_eq!(source_cache.all_features_coverage(), expected_coverage);
        Ok(())
    }

    fn assert_crate_cache_includes_declared_module(
        crate_root_source: &str,
        module_name: &str,
    ) -> Result<()> {
        let temp = tempdir()?;
        let source_directory = temp.path().join("src");
        let binary_directory = source_directory.join("bin");
        fs::create_dir_all(&binary_directory)?;
        let crate_root = source_directory.join("lib.rs");
        let declared_module = source_directory.join(module_name);
        let sibling_binary = binary_directory.join("probe.rs");
        fs::write(&crate_root, crate_root_source)?;
        fs::write(&declared_module, "const MARKER: () = ();\n")?;
        fs::write(&sibling_binary, "fn main() {}\n")?;

        let source_cache =
            SourceCache::build_crate(&crate_root, std::slice::from_ref(&crate_root))?;
        let declared_module = fs::canonicalize(declared_module)?;
        let sibling_binary = fs::canonicalize(sibling_binary)?;

        assert!(source_cache.read_source(&declared_module).is_ok());
        assert!(source_cache.read_source(&sibling_binary).is_err());
        Ok(())
    }

    #[test]
    fn analysis_source_root_ignores_build_scripts() {
        let package_root = Path::new("/tmp/example-crate");

        assert_eq!(
            analysis_source_root_for(&package_root.join("src/lib.rs"), package_root),
            Some(package_root.join("src"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("src/bin/demo.rs"), package_root),
            Some(package_root.join("src/bin"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("examples/demo.rs"), package_root),
            Some(package_root.join("examples"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("build.rs"), package_root),
            None
        );
    }

    #[test]
    fn module_path_from_source_file_treats_main_rs_as_crate_root() -> Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_dir = env::temp_dir().join(format!("mend-main-root-test-{unique}"));
        let source_dir = temp_dir.join("src");
        fs::create_dir_all(&source_dir)?;
        let main_rs = source_dir.join("main.rs");
        fs::write(&main_rs, "fn main() {}\n")?;

        assert_eq!(
            module_path_from_source_file(&source_dir, &main_rs),
            Some(Vec::new())
        );

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }
}
