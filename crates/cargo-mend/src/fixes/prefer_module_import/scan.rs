use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::visit::visit_item_mod;
use walkdir::WalkDir;

use super::function_imports::ImportDetector;
use super::function_imports::ImportTarget;
use super::function_imports::RawCandidate;
use super::inline_calls;
use super::inline_calls::InlineCallCandidate;
use super::inline_calls::InlineCallDetector;
use super::references::BareReference;
use super::references::ReferenceCollector;
use super::support;
use crate::compiler::SOURCE_DIR_SRC;
use crate::config::DiagnosticCode;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::rust_syntax;
use crate::selection::Selection;

pub(crate) struct PreferModuleImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(super) struct ScanFileContext<'a> {
    pub(super) analysis_root: &'a Path,
    pub(super) path:          &'a Path,
    pub(super) text:          &'a str,
    pub(super) offsets:       &'a [usize],
}

impl ScanFileContext<'_> {
    pub(super) fn display_path(&self) -> String {
        self.path
            .strip_prefix(self.analysis_root)
            .unwrap_or(self.path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// A bare module import (`use path::to::module;`) recorded with the inline
/// `mod` chain that contains it — empty for file top level. An import inside
/// `mod tests` binds nothing at file top level (and vice versa), so every
/// decision that reuses or dedups against an existing import must compare
/// scopes, not just module paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ScopedModuleImport {
    pub(super) inline_scope:    Vec<String>,
    pub(super) absolute_module: Vec<String>,
}

pub(super) struct ImportFindingInputs<'a> {
    module_to_functions:     &'a BTreeMap<String, Vec<RawCandidate>>,
    func_to_module:          &'a BTreeMap<&'a str, (&'a str, ImportTarget)>,
    references:              &'a [BareReference],
    /// Modules the file already imports with a bare `use module;`, keyed by
    /// scope. A function import whose target module is in this set at the same
    /// scope is rewritten to nothing (deleted) instead of to `use module;`,
    /// which would duplicate the existing import and fail to compile (E0252).
    existing_module_imports: &'a BTreeSet<ScopedModuleImport>,
}

pub(super) struct InlineCallFindingInputs<'a> {
    pub(super) candidates:            &'a [InlineCallCandidate],
    pub(super) will_import_modules:   &'a BTreeSet<Vec<String>>,
    pub(super) file_insertion_offset: usize,
}

pub(crate) fn scan_selection(selection: &Selection) -> Result<PreferModuleImportScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        if !source_root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&source_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(OsStr::to_str) != Some("rs")
            {
                continue;
            }
            let (findings, fixes) =
                scan_file(selection.analysis_root.as_path(), &source_root, path)?;
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
    source_root: &Path,
    path: &Path,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let current_module_path = rust_syntax::file_module_path(source_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let offsets = support::line_offsets(&text);
    let file_context = ScanFileContext {
        analysis_root,
        path,
        text: &text,
        offsets: &offsets,
    };

    let declared_modules = collect_declared_modules(&syntax);

    let mut detector = ImportDetector {
        source_root,
        current_module_path: current_module_path.clone(),
        inline_scope: Vec::new(),
        declared_modules: &declared_modules,
        candidates: Vec::new(),
    };
    Visit::visit_file(&mut detector, &syntax);

    let mut inline_detector = InlineCallDetector {
        source_root,
        current_module_path: &current_module_path,
        declared_modules: &declared_modules,
        candidates: Vec::new(),
        inline_mod_depth: 0,
    };
    Visit::visit_file(&mut inline_detector, &syntax);

    if detector.candidates.is_empty() && inline_detector.candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let existing_module_imports =
        collect_existing_module_imports(&syntax, source_root, &current_module_path);

    let mut module_to_functions: BTreeMap<String, Vec<RawCandidate>> = BTreeMap::new();
    for candidate in detector.candidates {
        module_to_functions
            .entry(candidate.module_path.clone())
            .or_default()
            .push(candidate);
    }

    drop_colliding_candidates(
        &existing_module_imports,
        &mut module_to_functions,
        &mut inline_detector.candidates,
    );

    if module_to_functions.is_empty() && inline_detector.candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let imported_names: BTreeSet<String> = module_to_functions
        .values()
        .flatten()
        .map(|candidate| candidate.function_name.clone())
        .collect();

    let mut collector = ReferenceCollector::new(&offsets, &imported_names);
    Visit::visit_file(&mut collector, &syntax);

    let mut func_to_module: BTreeMap<&str, (&str, ImportTarget)> = BTreeMap::new();
    for functions in module_to_functions.values() {
        for function in functions {
            func_to_module.insert(
                function.function_name.as_str(),
                (function.module_name.as_str(), function.import_target),
            );
        }
    }

    let (mut findings, mut fixes) = build_findings_and_fixes(
        &file_context,
        &ImportFindingInputs {
            module_to_functions:     &module_to_functions,
            func_to_module:          &func_to_module,
            references:              &collector.references,
            existing_module_imports: &existing_module_imports,
        },
    );

    if !inline_detector.candidates.is_empty() {
        let will_import_modules =
            build_will_import_modules(&existing_module_imports, &module_to_functions);
        let file_insertion_offset = file_level_insertion_offset(&syntax, &text, &offsets);
        let (inline_findings, inline_fixes) = inline_calls::build_inline_call_findings_and_fixes(
            &file_context,
            &InlineCallFindingInputs {
                candidates: &inline_detector.candidates,
                will_import_modules: &will_import_modules,
                file_insertion_offset,
            },
        );
        findings.extend(inline_findings);
        fixes.extend(inline_fixes);
    }

    Ok((findings, fixes))
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

fn collect_existing_module_imports(
    syntax: &File,
    source_root: &Path,
    current_module_path: &[String],
) -> BTreeSet<ScopedModuleImport> {
    let mut collector = ExistingModuleImportCollector {
        source_root,
        current_module_path: current_module_path.to_vec(),
        inline_scope: Vec::new(),
        imports: BTreeSet::new(),
    };
    Visit::visit_file(&mut collector, syntax);
    collector.imports
}

struct ExistingModuleImportCollector<'a> {
    source_root:         &'a Path,
    current_module_path: Vec<String>,
    inline_scope:        Vec<String>,
    imports:             BTreeSet<ScopedModuleImport>,
}

impl Visit<'_> for ExistingModuleImportCollector<'_> {
    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(flat) = support::flatten_use_tree(&node.tree)
            && flat.rename.is_none()
            && let Some(absolute) =
                support::resolve_to_absolute(&flat.segments, &self.current_module_path)
            && !absolute.is_empty()
            && support::leaf_is_module(self.source_root, &absolute)
        {
            self.imports.insert(ScopedModuleImport {
                inline_scope:    self.inline_scope.clone(),
                absolute_module: absolute,
            });
        }
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.current_module_path.push(node.ident.to_string());
            self.inline_scope.push(node.ident.to_string());
            visit_item_mod(self, node);
            self.inline_scope.pop();
            self.current_module_path.pop();
        } else {
            visit_item_mod(self, node);
        }
    }
}

/// Drop candidates whose target module name is already bound in the same scope
/// to a *different* module. Introducing `use <module>;` would collide with that
/// existing import (E0252), and rewriting the call to `name::fn(...)` would
/// resolve to the wrong module (E0425). Leave such imports untouched rather than
/// emit an unfixable finding.
fn drop_colliding_candidates(
    existing_module_imports: &BTreeSet<ScopedModuleImport>,
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
    inline_candidates: &mut Vec<InlineCallCandidate>,
) {
    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| {
            !module_name_collides(
                existing_module_imports,
                &candidate.inline_scope,
                &candidate.module_name,
                &candidate.absolute_module,
            )
        });
        !functions.is_empty()
    });
    // Inline call candidates are only detected at file top level (the detector
    // skips inline `mod` bodies), so their scope is always the empty chain.
    inline_candidates.retain(|candidate| {
        !module_name_collides(
            existing_module_imports,
            &[],
            &candidate.module_name,
            &candidate.absolute_module,
        )
    });
}

/// True when the same scope already imports a *different* module under the same
/// bare name that a prefer-module-import rewrite would introduce. Rewriting to
/// `use <module>;` in that case duplicates the name (E0252) and misroutes the
/// qualified call (E0425). The same-module case (`absolute_module` equal to an
/// existing import) is handled separately by deleting the redundant import.
fn module_name_collides(
    existing_module_imports: &BTreeSet<ScopedModuleImport>,
    inline_scope: &[String],
    module_name: &str,
    absolute_module: &[String],
) -> bool {
    existing_module_imports.iter().any(|imported| {
        imported.inline_scope == inline_scope
            && imported.absolute_module.last().map(String::as_str) == Some(module_name)
            && imported.absolute_module.as_slice() != absolute_module
    })
}

/// Modules that will be importable at file top level once the planned `use`
/// rewrites are applied. Imports and candidates inside inline `mod` blocks are
/// excluded: a `use` inside `mod tests` does not cover a top-level call site,
/// so it must not suppress the insertion of a top-level `use`.
fn build_will_import_modules(
    existing_module_imports: &BTreeSet<ScopedModuleImport>,
    module_to_functions: &BTreeMap<String, Vec<RawCandidate>>,
) -> BTreeSet<Vec<String>> {
    let mut will_import_modules: BTreeSet<Vec<String>> = existing_module_imports
        .iter()
        .filter(|import| import.inline_scope.is_empty())
        .map(|import| import.absolute_module.clone())
        .collect();
    for functions in module_to_functions.values() {
        for candidate in functions {
            if candidate.inline_scope.is_empty() {
                will_import_modules.insert(candidate.absolute_module.clone());
            }
        }
    }
    will_import_modules
}

fn file_level_insertion_offset(syntax: &File, text: &str, offsets: &[usize]) -> usize {
    let mut last_use_end: Option<usize> = None;
    let mut first_item_start: Option<usize> = None;
    for item in &syntax.items {
        let item_start = support::offset(offsets, item.span().start());
        first_item_start.get_or_insert(item_start);
        if let Item::Use(item_use) = item {
            let end = support::offset(offsets, item_use.span().end());
            let end = if text.as_bytes().get(end) == Some(&b'\n') {
                end + 1
            } else {
                end
            };
            last_use_end = Some(end);
        }
    }
    last_use_end.or(first_item_start).unwrap_or(0)
}

fn build_findings_and_fixes(
    file_context: &ScanFileContext<'_>,
    import_inputs: &ImportFindingInputs<'_>,
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = file_context.display_path();
    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut rewritten_modules: BTreeSet<ScopedModuleImport> = BTreeSet::new();

    for functions in import_inputs.module_to_functions.values() {
        for function in functions {
            findings.push(build_function_finding(
                function,
                &display_path,
                file_context,
            ));
            fixes.push(build_function_use_fix(
                function,
                file_context,
                import_inputs.existing_module_imports,
                &mut rewritten_modules,
            ));
        }
    }

    fixes.extend(build_reference_fixes(file_context, import_inputs));

    (findings, fixes)
}

fn build_function_finding(
    function: &RawCandidate,
    display_path: &str,
    file_context: &ScanFileContext<'_>,
) -> Finding {
    let source_line = file_context
        .text
        .lines()
        .nth(function.span_start.line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    let (message, suggestion) = if function.import_target == ImportTarget::ParentModule {
        (
            format!(
                "drop the import and call `super::{}` directly",
                function.function_name
            ),
            Some(format!(
                "remove this `use` and call `super::{}` at the use sites",
                function.function_name
            )),
        )
    } else {
        (
            format!(
                "import the module `{}` instead of the function `{}`",
                function.module_name, function.function_name
            ),
            Some(format!("consider using: `{}`", function.replacement_use)),
        )
    };

    Finding {
        severity: Severity::Warning,
        diagnostic_code: DiagnosticCode::PreferModuleImport,
        path: display_path.to_string(),
        line: function.span_start.line,
        column: function.span_start.column + 1,
        highlight_len: function.function_name.len().max(1),
        source_line,
        item: None,
        message,
        suggestion,
        fix_support: FixSupport::PreferModuleImport,
        related: None,
    }
}

fn build_function_use_fix(
    function: &RawCandidate,
    file_context: &ScanFileContext<'_>,
    existing_module_imports: &BTreeSet<ScopedModuleImport>,
    rewritten_modules: &mut BTreeSet<ScopedModuleImport>,
) -> UseFix {
    let byte_start = support::offset(file_context.offsets, function.span_start);
    let byte_end = support::offset(file_context.offsets, function.span_end);
    let byte_end_with_newline = if file_context.text.as_bytes().get(byte_end) == Some(&b'\n') {
        byte_end + 1
    } else {
        byte_end
    };
    let group = Some(ImportGroup {
        bare_name: function.module_name.clone(),
        full_path: function.absolute_module.join("::"),
    });
    let scoped_module = ScopedModuleImport {
        inline_scope:    function.inline_scope.clone(),
        absolute_module: function.absolute_module.clone(),
    };

    if function.import_target == ImportTarget::ParentModule
        || existing_module_imports.contains(&scoped_module)
    {
        // Either call sites become `super::fn(...)` (parent module, no `use`
        // needed), or the same scope already imports the target module — so
        // the function import is redundant. Delete the line in both cases;
        // rewriting it to `use module;` when the module is already imported
        // would produce a duplicate import (E0252).
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end_with_newline,
            replacement:  String::new(),
            import_group: group,
        }
    } else if rewritten_modules.insert(scoped_module) {
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end,
            replacement:  function.replacement_use.clone(),
            import_group: group,
        }
    } else {
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end_with_newline,
            replacement:  String::new(),
            import_group: group,
        }
    }
}

fn build_reference_fixes(
    file_context: &ScanFileContext<'_>,
    import_inputs: &ImportFindingInputs<'_>,
) -> Vec<UseFix> {
    let mut fixes = Vec::new();
    for reference in import_inputs.references {
        if let Some(&(module_name, import_target)) =
            import_inputs.func_to_module.get(reference.name.as_str())
        {
            let group = import_inputs
                .module_to_functions
                .values()
                .flatten()
                .find(|function| function.module_name == module_name)
                .map(|function| ImportGroup {
                    bare_name: function.module_name.clone(),
                    full_path: function.absolute_module.join("::"),
                });
            let replacement = if import_target == ImportTarget::ParentModule {
                // Inside an inline `mod` (e.g. `#[cfg(test)] mod tests`) the
                // file's parent is one `super` further away per nesting level.
                let supers = "super::".repeat(reference.inline_mod_depth + 1);
                format!("{supers}{}", reference.name)
            } else {
                format!("{module_name}::{}", reference.name)
            };
            fixes.push(UseFix {
                path: file_context.path.to_path_buf(),
                start: reference.byte_start,
                end: reference.byte_end,
                replacement,
                import_group: group,
            });
        }
    }
    fixes
}
