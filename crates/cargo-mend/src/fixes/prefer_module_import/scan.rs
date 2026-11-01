use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::visit::visit_item_mod;

use super::attribute_references;
use super::descendant_globs;
use super::function_imports::ImportDetector;
use super::function_imports::ImportTarget;
use super::function_imports::RawCandidate;
use super::inline_calls;
use super::inline_calls::InlineCallCandidate;
use super::inline_calls::InlineCallDetector;
use super::references::BareReference;
use super::references::ReferenceCollector;
use super::support;
use super::support::CrateSources;
use super::support::SourceSpan;
use crate::compiler::SOURCE_DIR_SRC;
use crate::config::DiagnosticCode;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::ImportProposal;
use crate::fixes::imports::ImportReservation;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::ModuleImports;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

pub(crate) struct PreferModuleImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

/// File text and its checked index are constructed together and shared by every scan pass.
pub(super) struct ScanFileContext<'a> {
    analysis_root:   &'a Path,
    pub(super) path: &'a Path,
    text:            &'a str,
    source:          ImportSource<'a>,
}

impl<'a> ScanFileContext<'a> {
    pub(super) fn new(analysis_root: &'a Path, path: &'a Path, text: &'a str) -> Self {
        Self {
            analysis_root,
            path,
            text,
            source: ImportSource::new(text),
        }
    }

    pub(super) const fn source(&self) -> &ImportSource<'a> { &self.source }

    pub(super) const fn text(&self) -> &'a str { self.text }

    pub(super) fn display_path(&self) -> String {
        self.path
            .strip_prefix(self.analysis_root)
            .unwrap_or(self.path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

#[derive(Clone, Copy)]
enum FunctionImportPolicy<'a> {
    Parent,
    Module { name: &'a str },
}

impl<'a> From<(&'a str, ImportTarget)> for FunctionImportPolicy<'a> {
    fn from((name, target): (&'a str, ImportTarget)) -> Self {
        match target {
            ImportTarget::ParentModule => Self::Parent,
            ImportTarget::OtherModule => Self::Module { name },
        }
    }
}

impl FunctionImportPolicy<'_> {
    fn finding_text(self, function: &RawCandidate) -> (String, String) {
        match self {
            Self::Parent => (
                format!(
                    "drop the import and call `super::{}` directly",
                    function.function_name
                ),
                format!(
                    "remove this `use` and call `super::{}` at the use sites",
                    function.function_name
                ),
            ),
            Self::Module { name } => (
                format!(
                    "import the module `{name}` instead of the function `{}`",
                    function.function_name
                ),
                format!("consider using: `{}`", function.replacement_use),
            ),
        }
    }

    fn reference(self, name: &str, inline_mod_depth: usize) -> String {
        match self {
            Self::Parent => format!("{}{name}", "super::".repeat(inline_mod_depth + 1)),
            Self::Module { name: module } => format!("{module}::{name}"),
        }
    }

    fn import_plan(
        self,
        owner: &mut ModuleImports,
        offset: usize,
        path: &str,
        gates: &ConditionalAttributes,
    ) -> ImportRewritePlan {
        match self {
            Self::Parent => ImportRewritePlan::DeleteRedundant,
            Self::Module { .. } => match owner.reserve_module(offset, path, gates) {
                ImportReservation::Existing => ImportRewritePlan::DeleteRedundant,
                ImportReservation::Planned => ImportRewritePlan::DeleteDuplicate,
                ImportReservation::Added => ImportRewritePlan::ReplaceWithModule,
            },
        }
    }
}

enum ImportRewritePlan {
    DeleteRedundant,
    ReplaceWithModule,
    DeleteDuplicate,
}

/// A function import whose coordinates and diagnostic line belong to the scanned source.
/// Missing lines cannot become empty diagnostic text.
struct FunctionSourceLocation<'a> {
    span: SourceSpan<'a>,
}

/// References stay attached to the candidate and import group that justify their edits.
/// Edit construction cannot silently omit a reference or lose its group in a second lookup.
struct RewriteBinding<'a> {
    function:   &'a RawCandidate,
    location:   FunctionSourceLocation<'a>,
    group:      ImportGroup,
    references: Vec<&'a BareReference>,
}

impl<'a> RewriteBinding<'a> {
    fn collect(
        source: &ImportSource<'a>,
        candidates: &'a BTreeMap<String, Vec<RawCandidate>>,
        references: &'a [BareReference],
    ) -> Result<Vec<Self>> {
        let mut bindings = Vec::new();
        let mut by_name = BTreeMap::new();
        for function in candidates.values().flatten() {
            by_name.insert(function.function_name.as_str(), bindings.len());
            bindings.push(Self {
                function,
                location: FunctionSourceLocation {
                    span: support::SourceSpan::new(source, function.span_start, function.span_end)?,
                },
                group: ImportGroup {
                    bare_name: function.module_name.clone(),
                    full_path: function.absolute_module.join("::"),
                },
                references: Vec::new(),
            });
        }
        for reference in references {
            let index = by_name
                .get(reference.name.as_str())
                .context("reference has no function import candidate")?;
            bindings[*index].references.push(reference);
        }
        Ok(bindings)
    }

    fn policy(&self) -> FunctionImportPolicy<'_> {
        FunctionImportPolicy::from((
            self.function.module_name.as_str(),
            self.function.import_target,
        ))
    }
}

pub(super) struct InlineCallFindingInputs<'a> {
    pub(super) candidates:     &'a [InlineCallCandidate],
    pub(super) imports:        &'a mut ModuleImports,
    pub(super) file_insertion: FileInsertionSite,
}

/// Where a file-level `use` goes: after the last top-level `use`, before the first item,
/// or at the start of an empty file. `BeforeFirstItem` and `EmptyFile` have no existing
/// top-level `use`, so inserted imports need a blank line separating them from any following item.
#[derive(Clone, Copy)]
pub(super) enum FileInsertionSite {
    AfterImport { offset: usize },
    BeforeFirstItem { offset: usize },
    EmptyFile,
}

impl FileInsertionSite {
    pub(super) const fn offset(self) -> usize {
        match self {
            Self::AfterImport { offset } | Self::BeforeFirstItem { offset } => offset,
            Self::EmptyFile => 0,
        }
    }
}

pub(crate) fn scan_selection(
    selection: &Selection,
    sources: &ParsedSources,
) -> Result<PreferModuleImportScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        let module_map = sources.module_map(&source_root);
        for path in ModuleImports::source_files(sources, package_root) {
            let (findings, fixes) = scan_file(
                selection.analysis_root.as_path(),
                CrateSources {
                    source_root: &source_root,
                    sources,
                },
                &path,
                &module_map,
            )?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|left, right| {
        (&left.path, left.line, left.column).cmp(&(&right.path, right.line, right.column))
    });
    all_findings.dedup_by(|left, right| {
        left.path == right.path && left.line == right.line && left.column == right.column
    });
    Ok(PreferModuleImportScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::try_from(all_fixes)?,
    })
}

fn scan_file(
    analysis_root: &Path,
    crate_sources: CrateSources<'_>,
    path: &Path,
    module_map: &ModuleMap,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let Some(current_module_path) =
        module_map.scannable_module_path(crate_sources.source_root, path)?
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let source = crate_sources
        .sources
        .source(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let syntax = source
        .syntax()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let text = source.text();
    let file_context = ScanFileContext::new(analysis_root, path, text);
    let indexed = file_context.source();

    let (raw_candidates, mut inline_candidates) =
        detect_candidates(crate_sources, indexed, &current_module_path, syntax);
    if raw_candidates.is_empty() && inline_candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut imports = ModuleImports::from_syntax(syntax, indexed, &current_module_path)?;

    let module_to_functions = fixable_function_candidates(
        &FileScope {
            syntax,
            module_map,
            sources: crate_sources.sources,
            current_module_path: &current_module_path,
            imports: &imports,
        },
        raw_candidates,
        &mut inline_candidates,
    );

    if module_to_functions.is_empty() && inline_candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let imported_names: BTreeSet<String> = module_to_functions
        .values()
        .flatten()
        .map(|candidate| candidate.function_name.clone())
        .collect();

    let mut collector = ReferenceCollector::new(indexed, &imported_names);
    Visit::visit_file(&mut collector, syntax);

    let references = collector.references?;
    let bindings = RewriteBinding::collect(indexed, &module_to_functions, &references)?;

    let (mut findings, mut fixes) =
        build_findings_and_fixes(&file_context, &bindings, &mut imports);

    if !inline_candidates.is_empty() {
        let file_insertion = file_level_insertion(syntax, text, indexed)?;
        let (inline_findings, inline_use_fixes) =
            inline_calls::build_inline_call_findings_and_fixes(
                &file_context,
                &mut InlineCallFindingInputs {
                    candidates: &inline_candidates,
                    imports: &mut imports,
                    file_insertion,
                },
            )?;
        findings.extend(inline_findings);
        fixes.extend(inline_use_fixes);
    }

    Ok((findings, fixes))
}

/// The function import and inline call candidates the two detectors find in
/// `syntax`.
fn detect_candidates<'a>(
    crate_sources: CrateSources<'a>,
    source: &ImportSource<'a>,
    current_module_path: &'a [String],
    syntax: &File,
) -> (Vec<RawCandidate>, Vec<InlineCallCandidate>) {
    let declared_modules = collect_declared_modules(syntax);
    let mut import_detector = ImportDetector {
        crate_sources,
        current_module_path: current_module_path.to_vec(),
        inline_scope: Vec::new(),
        declared_modules: &declared_modules,
        candidates: Vec::new(),
    };
    import_detector.visit_file(syntax, source);
    let mut inline_call_detector = InlineCallDetector {
        crate_sources,
        current_module_path,
        declared_modules: &declared_modules,
        candidates: Vec::new(),
        inline_mod_depth: 0,
        conditional_attributes: ConditionalAttributes::default(),
    };
    inline_call_detector.visit_file(syntax, source);
    (import_detector.candidates, inline_call_detector.candidates)
}

/// What the candidate drops read about the scanned file.
struct FileScope<'a> {
    syntax:              &'a File,
    module_map:          &'a ModuleMap,
    sources:             &'a ParsedSources,
    current_module_path: &'a [String],
    imports:             &'a ModuleImports,
}

/// The function import candidates grouped by module, without those whose
/// rewrite would not compile; drops the colliding inline call candidates too.
fn fixable_function_candidates(
    scope: &FileScope<'_>,
    candidates: Vec<RawCandidate>,
    inline_candidates: &mut Vec<InlineCallCandidate>,
) -> BTreeMap<String, Vec<RawCandidate>> {
    let mut module_to_functions = group_candidates_by_module(candidates);

    drop_colliding_candidates(scope.imports, &mut module_to_functions, inline_candidates);

    drop_attribute_referenced_candidates(scope.syntax, &mut module_to_functions);

    drop_candidates_reimported_by_inline_modules(scope.syntax, &mut module_to_functions);

    descendant_globs::drop_candidates_reached_by_descendant_globs(
        scope.module_map,
        scope.sources,
        scope.current_module_path,
        &mut module_to_functions,
    );

    // `drop_candidates_colliding_with_each_other` runs last so the filters above can remove one
    // side and eliminate a collision.
    drop_candidates_colliding_with_each_other(&mut module_to_functions, inline_candidates);

    module_to_functions
}

fn group_candidates_by_module(
    candidates: Vec<RawCandidate>,
) -> BTreeMap<String, Vec<RawCandidate>> {
    let mut grouped: BTreeMap<String, Vec<RawCandidate>> = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.module_path.clone())
            .or_default()
            .push(candidate);
    }
    grouped
}

fn collect_declared_modules(syntax: &File) -> BTreeSet<String> {
    syntax
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Mod(item_mod) = item
                && item_mod.content.is_none()
            {
                Some(item_mod.ident.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Drop candidates that would bind one module name to two different modules in
/// the same scope. Two deep imports whose modules share a leaf name — say
/// `crate::orbit_cam::controller` alongside `crate::free_cam::controller` — each
/// want `use <leaf>;`, which collides (E0252) and leaves the call sites
/// resolving to whichever import won (E0425).
///
/// The combining layer drops these fixes before anything is written, so
/// reporting them advertised a fix that never arrived: every run named the same
/// two imports as fixable, wrote nothing, and named them again. Leave them
/// untouched, as [`drop_colliding_candidates`] does for the same collision
/// against an import the file already has.
fn drop_candidates_colliding_with_each_other(
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
    inline_candidates: &mut Vec<InlineCallCandidate>,
) {
    // `InlineCallDetector` detects `InlineCallCandidate` values only at file top level, so their
    // scope is always the empty chain — the same key the detector gives a
    // top-level `RawCandidate`.
    let mut name_to_modules: BTreeMap<(Vec<String>, String), BTreeSet<Vec<String>>> =
        BTreeMap::new();
    for candidate in module_to_functions.values().flatten() {
        name_to_modules
            .entry((
                candidate.inline_scope.clone(),
                candidate.module_name.clone(),
            ))
            .or_default()
            .insert(candidate.absolute_module.clone());
    }
    for candidate in inline_candidates.iter() {
        name_to_modules
            .entry((Vec::new(), candidate.module_name.clone()))
            .or_default()
            .insert(candidate.absolute_module.clone());
    }

    let colliding: BTreeSet<(Vec<String>, String)> = name_to_modules
        .into_iter()
        .filter(|(_, modules)| modules.len() > 1)
        .map(|(key, _)| key)
        .collect();

    if colliding.is_empty() {
        return;
    }

    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| {
            !colliding.contains(&(
                candidate.inline_scope.clone(),
                candidate.module_name.clone(),
            ))
        });
        !functions.is_empty()
    });
    inline_candidates
        .retain(|candidate| !colliding.contains(&(Vec::new(), candidate.module_name.clone())));
}

/// Drop candidates whose target module name is already bound in the same scope
/// to a *different* module. Introducing `use <module>;` would collide with that
/// existing import (E0252), and rewriting the call to `name::fn(...)` would
/// resolve to the wrong module (E0425). Leave such imports untouched rather than
/// emit an unfixable finding.
fn drop_colliding_candidates(
    imports: &ModuleImports,
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
    inline_candidates: &mut Vec<InlineCallCandidate>,
) {
    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| {
            !imports.collides(
                imports.offset_for_scope(&candidate.inline_scope),
                &candidate.module_name,
                &candidate.absolute_module,
                &ImportProposal::RewriteAt(candidate.span_start),
            )
        });
        !functions.is_empty()
    });
    inline_candidates.retain(|candidate| {
        !imports.collides(
            0,
            &candidate.module_name,
            &candidate.absolute_module,
            &ImportProposal::Addition,
        )
    });
}

/// Drop candidates whose function name is mentioned by an attribute. Attribute
/// payloads belong to the attribute macro's grammar rather than to the path
/// graph the rewrite walks — `#[serde(default = "make_default")]` names the
/// function as a string literal — so the reference collector never sees them and
/// removing the import would leave the name unresolved (E0425). Leave such
/// imports untouched rather than emit a fix that does not compile.
///
/// Inline call candidates are unaffected: they add a module import and shorten a
/// path that is already fully qualified, so nothing an attribute names stops
/// resolving.
/// Drop candidates an inline `mod` re-imports from the file's own top level.
///
/// `#[cfg(test)] mod tests { use super::reflect_component_for; }` sitting under
/// a file-scope `use crate::capabilities::reflect_component_for;` names that
/// file-scope binding. Rewriting the outer import to `use crate::capabilities;`
/// leaves the inner `use` naming nothing (E0432), and the module has no
/// `capabilities` binding to reach the function through (E0433). The pair only
/// works rewritten together, so leave both alone and report nothing rather than
/// emit a fix that compiles in the file but not in the module below it.
///
/// A glob (`use super::*;`) is not a re-import: it makes whatever the file
/// binds visible, so the new `use crate::capabilities;` arrives with it and the
/// references inside the module follow the rewrite normally.
fn drop_candidates_reimported_by_inline_modules(
    syntax: &File,
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
) {
    let mut collector = InlineSuperReimportCollector {
        inline_mod_depth: 0,
        names:            BTreeSet::new(),
    };
    Visit::visit_file(&mut collector, syntax);
    if collector.names.is_empty() {
        return;
    }

    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| {
            !candidate.inline_scope.is_empty()
                || !collector.names.contains(&candidate.function_name)
        });
        !functions.is_empty()
    });
}

struct InlineSuperReimportCollector {
    inline_mod_depth: usize,
    names:            BTreeSet<String>,
}

impl Visit<'_> for InlineSuperReimportCollector {
    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.inline_mod_depth += 1;
            visit_item_mod(self, node);
            self.inline_mod_depth -= 1;
        } else {
            visit_item_mod(self, node);
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        if self.inline_mod_depth == 0 {
            return;
        }
        collect_super_reimports(
            &node.tree,
            &mut Vec::new(),
            self.inline_mod_depth,
            &mut self.names,
        );
    }
}

/// Leaf names of `use` paths that climb exactly `depth` `super` segments — from
/// inside an inline `mod` nested `depth` deep, that is the file's own top level.
fn collect_super_reimports(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    depth: usize,
    names: &mut BTreeSet<String>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_super_reimports(&path.tree, prefix, depth, names);
            prefix.pop();
        },
        UseTree::Name(name) => {
            if matches!(
                file_top_level_reach(prefix, depth),
                FileTopLevelReach::Reaches
            ) {
                names.insert(name.ident.to_string());
            }
        },
        UseTree::Rename(rename) => {
            if matches!(
                file_top_level_reach(prefix, depth),
                FileTopLevelReach::Reaches
            ) {
                names.insert(rename.ident.to_string());
            }
        },
        UseTree::Group(group) => {
            for item in &group.items {
                collect_super_reimports(item, prefix, depth, names);
            }
        },
        UseTree::Glob(_) => {},
    }
}

/// Whether a path climbs exactly the enclosing inline-module depth to the file's own top level.
enum FileTopLevelReach {
    Reaches,
    DoesNotReach,
}

fn file_top_level_reach(prefix: &[String], depth: usize) -> FileTopLevelReach {
    if prefix.len() == depth && prefix.iter().all(|segment| segment == "super") {
        FileTopLevelReach::Reaches
    } else {
        FileTopLevelReach::DoesNotReach
    }
}

fn drop_attribute_referenced_candidates(
    syntax: &File,
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
) {
    let attribute_names = attribute_references::collect(syntax);
    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| !attribute_names.contains(&candidate.function_name));
        !functions.is_empty()
    });
}

fn file_level_insertion(
    syntax: &File,
    text: &str,
    source: &ImportSource<'_>,
) -> Result<FileInsertionSite> {
    let mut last_use_end: Option<usize> = None;
    let mut first_item_start: Option<usize> = None;
    for item in &syntax.items {
        let item_start = support::offset(source, item.span().start())?.byte();
        first_item_start.get_or_insert(item_start);
        if let Item::Use(item_use) = item {
            let end = support::offset(source, item_use.span().end())?.byte();
            let end = if text.as_bytes().get(end) == Some(&b'\n') {
                end + 1
            } else {
                end
            };
            last_use_end = Some(end);
        }
    }
    Ok(match (last_use_end, first_item_start) {
        (Some(offset), _) => FileInsertionSite::AfterImport { offset },
        (None, Some(offset)) => FileInsertionSite::BeforeFirstItem { offset },
        (None, None) => FileInsertionSite::EmptyFile,
    })
}

fn build_findings_and_fixes(
    file_context: &ScanFileContext<'_>,
    bindings: &[RewriteBinding<'_>],
    imports: &mut ModuleImports,
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = file_context.display_path();
    let mut findings = Vec::new();
    let mut fixes = Vec::new();

    for binding in bindings {
        let policy = binding.policy();
        findings.push(build_function_finding(binding, &display_path, policy));
        fixes.push(build_function_use_fix(
            binding,
            file_context,
            imports,
            policy,
        ));
    }
    for binding in bindings {
        fixes.extend(build_reference_fixes(file_context, binding));
    }

    (findings, fixes)
}

fn build_function_finding(
    binding: &RewriteBinding<'_>,
    display_path: &str,
    policy: FunctionImportPolicy<'_>,
) -> Finding {
    let function = binding.function;
    let source_line = binding.location.span.start().source_line().to_string();

    let (message, suggestion) = policy.finding_text(function);

    Finding {
        disposition: Finding::disposition(
            DiagnosticCode::PreferModuleImport,
            Severity::Warning,
            FixSupport::PreferModuleImport,
        ),
        diagnostic_code: DiagnosticCode::PreferModuleImport,
        path: display_path.to_string(),
        line: function.span_start.line,
        column: function.span_start.column + 1,
        highlight_len: function.function_name.len().max(1),
        source_line,
        item: None,
        message,
        suggestion: Some(suggestion),
        related: None,
        item_visibility: ItemVisibility::default(),
    }
}

fn build_function_use_fix(
    binding: &RewriteBinding<'_>,
    file_context: &ScanFileContext<'_>,
    imports: &mut ModuleImports,
    policy: FunctionImportPolicy<'_>,
) -> UseFix {
    let function = binding.function;
    let byte_start = binding.location.span.start().byte();
    let byte_end = binding.location.span.end();
    let byte_end_with_newline = if file_context.text.as_bytes().get(byte_end) == Some(&b'\n') {
        byte_end + 1
    } else {
        byte_end
    };
    let group = Some(binding.group.clone());
    match policy.import_plan(
        imports,
        byte_start,
        &function.module_path,
        &function.conditional_attributes,
    ) {
        ImportRewritePlan::DeleteRedundant | ImportRewritePlan::DeleteDuplicate => UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end_with_newline,
            replacement:  String::new(),
            import_group: group,
        },
        ImportRewritePlan::ReplaceWithModule => UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end,
            replacement:  replacement_use_with_conditional_attributes(function),
            import_group: group,
        },
    }
}

fn replacement_use_with_conditional_attributes(function: &RawCandidate) -> String {
    if function.conditional_attributes.is_empty() {
        return function.replacement_use.clone();
    }

    let mut replacement = function.conditional_attributes.render("");
    replacement.push_str(&" ".repeat(function.span_start.column));
    replacement.push_str(&function.replacement_use);
    replacement
}

fn build_reference_fixes(
    file_context: &ScanFileContext<'_>,
    binding: &RewriteBinding<'_>,
) -> Vec<UseFix> {
    binding
        .references
        .iter()
        .map(|reference| UseFix {
            path:         file_context.path.to_path_buf(),
            start:        reference.byte_start,
            end:          reference.byte_end,
            replacement:  binding
                .policy()
                .reference(&reference.name, reference.inline_mod_depth),
            import_group: Some(binding.group.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cmp::Reverse;
    use std::collections::BTreeSet;

    use anyhow::Context;
    use proc_macro2::LineColumn;

    use super::FunctionImportPolicy;
    use super::ImportRewritePlan;
    use crate::fixes::imports::ImportSource;
    use crate::fixes::imports::ModuleImports;
    use crate::rust_syntax::ParsedSources;

    #[test]
    fn module_import_fix_keeps_if_let_registration_row_local() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("lib.rs"),
            "mod invocation_rows; mod consumer;",
        )?;
        std::fs::write(
            dir.path().join("invocation_rows.rs"),
            "pub struct Row { pub invocation_id: u64 } pub fn row() {}",
        )?;
        let path = dir.path().join("consumer.rs");
        let text = r"use super::invocation_rows::row;
fn registration_row() -> Result<super::invocation_rows::Row, ()> {
    Ok(super::invocation_rows::Row { invocation_id: 0 })
}
fn collect(identity: u64) {
    row();
    let mut rows = Vec::new();
    if let Ok(mut row) = registration_row() {
        row.invocation_id = identity;
        rows.push(row);
    } else {
        row();
    }
    row();
}
";
        std::fs::write(&path, text)?;
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let (findings, fixes) = super::scan_file(
            dir.path(),
            super::CrateSources {
                source_root: dir.path(),
                sources:     &sources,
            },
            &path,
            &module_map,
        )?;
        assert_eq!(findings.len(), 1);
        crate::fixes::imports::apply_fixes(&fixes.try_into()?)?;
        let rewritten = std::fs::read_to_string(&path)?;
        assert_eq!(
            rewritten,
            text.replace(
                "use super::invocation_rows::row;",
                "use super::invocation_rows;"
            )
            .replace("row();", "invocation_rows::row();")
        );
        Ok(())
    }

    #[test]
    fn full_file_scan_shares_one_index_across_both_detectors() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let calls = "crate::util::run();".repeat(128);
        for gate in ["", "#[cfg(any(unix, windows))]\n"] {
            let text = format!(
                "mod util {{ pub fn run() {{}} }} {gate}use crate::util::run; \
                 {gate}fn f() {{ run(); {calls} }} \
                 mod nested {{ {gate}use crate::util::run; {gate}fn g() {{ run(); }} }}"
            );
            std::fs::write(&path, &text)?;
            let sources = ParsedSources::default();
            let module_map = sources.module_map(dir.path());
            let crate_sources = super::CrateSources {
                source_root: dir.path(),
                sources:     &sources,
            };
            let parsed = sources.source(&path)?;
            let before = ImportSource::construction_count();
            let source = ImportSource::new(parsed.text());
            let (imports, calls) =
                super::detect_candidates(crate_sources, &source, &[], parsed.syntax()?);
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert_eq!(imports.len(), 2);
            assert_eq!(calls.len(), 128);
            assert!(
                imports
                    .iter()
                    .all(|site| site.conditional_attributes.render("") == gate)
            );
            assert!(
                calls
                    .iter()
                    .all(|site| site.conditional_attributes.render("") == gate)
            );
            let before = ImportSource::construction_count();
            let (findings, fixes) =
                super::scan_file(dir.path(), crate_sources, &path, &module_map)?;
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert!(!findings.is_empty());
            assert!(!fixes.is_empty());
        }
        Ok(())
    }

    #[test]
    fn file_scan_keeps_module_import_through_ancestor_reexport() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir_all(dir.path().join("a/b"))?;
        let path = dir.path().join("lib.rs");
        let text = concat!(
            "pub mod a;\nuse crate::a::b::c::ancestor::api;\n",
            "fn f() { api::run(); }\n",
        );
        for (file, contents) in [
            ("lib.rs", text),
            ("a.rs", "pub mod b; pub mod api { pub fn run() {} }"),
            ("a/b.rs", "pub mod c;"),
            ("a/b/c.rs", "pub use super::super as ancestor;"),
        ] {
            std::fs::write(dir.path().join(file), contents)?;
        }
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let (findings, fixes) = super::scan_file(
            dir.path(),
            super::CrateSources {
                source_root: dir.path(),
                sources:     &sources,
            },
            &path,
            &module_map,
        )?;
        assert!(findings.is_empty());
        assert!(fixes.is_empty());
        assert_eq!(std::fs::read_to_string(path)?, text);
        Ok(())
    }

    #[test]
    fn import_plans_preserve_scope_and_only_reserve_replacements() -> anyhow::Result<()> {
        let source = "mod tests {}";
        let syntax = syn::parse_file(source)?;
        let indexed = ImportSource::new(source);
        let mut rewritten = ModuleImports::from_syntax(&syntax, &indexed, &[])?;
        let gates = super::ConditionalAttributes::default();
        let parent = FunctionImportPolicy::Parent;
        let other = FunctionImportPolicy::Module { name: "tool" };
        assert!(matches!(
            parent.import_plan(&mut rewritten, 0, "crate::tool", &gates),
            ImportRewritePlan::DeleteRedundant
        ));
        assert!(!rewritten.has_reservations());
        let kept_source = "use crate::tool;";
        let kept_syntax = syn::parse_file(kept_source)?;
        let kept_index = ImportSource::new(kept_source);
        let mut existing = ModuleImports::from_syntax(&kept_syntax, &kept_index, &[])?;
        assert!(matches!(
            other.import_plan(&mut existing, 0, "crate::tool", &gates),
            ImportRewritePlan::DeleteRedundant
        ));
        assert!(!existing.has_reservations());
        let nested_source = "mod tests { use crate::tool; }";
        let nested_syntax = syn::parse_file(nested_source)?;
        let nested_index = ImportSource::new(nested_source);
        let mut rewritten = ModuleImports::from_syntax(&nested_syntax, &nested_index, &[])?;
        assert!(matches!(
            other.import_plan(&mut rewritten, 0, "crate::tool", &gates),
            ImportRewritePlan::ReplaceWithModule
        ));
        // Start with an empty nested scope while retaining a root reservation,
        // just as the original plan test changed its retained-import input.
        let mut rewritten = ModuleImports::from_syntax(&syntax, &indexed, &[])?;
        assert!(matches!(
            other.import_plan(&mut rewritten, 0, "crate::tool", &gates),
            ImportRewritePlan::ReplaceWithModule
        ));
        assert!(matches!(
            other.import_plan(&mut rewritten, 0, "crate::tool", &gates),
            ImportRewritePlan::DeleteDuplicate
        ));
        let nested = rewritten.offset_for_scope(&["tests".to_string()]);
        // A separate empty nested scope still needs its own import.
        assert!(matches!(
            other.import_plan(&mut rewritten, nested, "crate::tool", &gates),
            ImportRewritePlan::ReplaceWithModule
        ));
        assert_eq!(parent.reference("run", 2), "super::super::super::run");
        assert_eq!(other.reference("run", 2), "tool::run");
        Ok(())
    }
    #[test]
    fn rewrite_bindings_keep_checked_lines_references_and_groups_together() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("lib.rs"), "mod util { pub fn run() {} }")?;
        let sources = ParsedSources::default();
        let text = "// é\nuse crate::util::run;\nfn f() { run(); }\n";
        let source = ImportSource::new(text);
        let syntax = syn::parse_file(text)?;
        let (candidates, _) = super::detect_candidates(
            super::CrateSources {
                source_root: dir.path(),
                sources:     &sources,
            },
            &source,
            &[],
            &syntax,
        );
        let mut grouped = super::group_candidates_by_module(candidates);
        let names = BTreeSet::from(["run".to_string()]);
        let mut collector = super::ReferenceCollector::new(&source, &names);
        syn::visit::Visit::visit_file(&mut collector, &syntax);
        let references = collector.references?;
        let bindings = super::RewriteBinding::collect(&source, &grouped, &references)?;
        assert_eq!(bindings.len(), 1);
        let binding = &bindings[0];
        assert_eq!(
            binding.location.span.start().source_line(),
            "use crate::util::run;"
        );
        assert_eq!(binding.references.len(), 1);
        assert_eq!(binding.group.bare_name, "util");
        assert_eq!(binding.group.full_path, "util");
        let context = super::ScanFileContext::new(dir.path(), dir.path(), text);
        let fixes = super::build_reference_fixes(&context, binding);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].replacement, "util::run");
        assert_eq!(
            fixes[0].import_group.as_ref().map(|group| &group.full_path),
            Some(&binding.group.full_path)
        );
        let unbound = [super::BareReference {
            name:             "unbound".to_string(),
            byte_start:       0,
            byte_end:         0,
            inline_mod_depth: 0,
        }];
        assert!(super::RewriteBinding::collect(&source, &grouped, &unbound).is_err());
        for position in [
            LineColumn {
                line:   0,
                column: 0,
            },
            LineColumn {
                line:   8,
                column: 0,
            },
            LineColumn {
                line:   2,
                column: 100,
            },
        ] {
            grouped.get_mut("crate::util").context("missing module")?[0].span_start = position;
            assert!(super::RewriteBinding::collect(&source, &grouped, &references).is_err());
        }
        Ok(())
    }

    #[test]
    fn file_scan_preserves_diagnostic_lines_and_exact_rewrite_bytes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let text = concat!(
            "// é\nmod util { pub fn run() {} }\n",
            "use crate::util::run;\n",
            "fn f() { let α = (); run(); crate::util::run(); }\n",
        );
        std::fs::write(&path, text)?;
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let before = ImportSource::construction_count();
        let (findings, mut fixes) = super::scan_file(
            dir.path(),
            super::CrateSources {
                source_root: dir.path(),
                sources:     &sources,
            },
            &path,
            &module_map,
        )?;
        assert_eq!(ImportSource::construction_count() - before, 1);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].source_line, "use crate::util::run;");
        assert_eq!(
            findings[1].source_line,
            "fn f() { let α = (); run(); crate::util::run(); }"
        );
        assert_eq!(findings[1].highlight_len, "crate::util::run".len());
        fixes.sort_by_key(|fix| Reverse(fix.start));
        let mut rewritten = text.to_string();
        for fix in fixes {
            rewritten.replace_range(fix.start..fix.end, &fix.replacement);
        }
        assert_eq!(
            rewritten,
            concat!(
                "// é\nmod util { pub fn run() {} }\n",
                "use crate::util;\n",
                "fn f() { let α = (); util::run(); util::run(); }\n",
            )
        );
        Ok(())
    }
}
