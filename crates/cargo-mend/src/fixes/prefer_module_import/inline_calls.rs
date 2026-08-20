use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::LineColumn;
use syn::ExprPath;
use syn::ItemMod;
use syn::ItemUse;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use super::function_imports::ImportTarget;
use super::scan::InlineCallFindingInputs;
use super::scan::ScanFileContext;
use super::support;
use crate::config::DiagnosticCode;
use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::GateSource;
use crate::fixes::imports::GateTracking;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;
use crate::rust_syntax::PathAnchor;

pub(super) struct InlineCallCandidate {
    pub(super) function_name:          String,
    pub(super) module_name:            String,
    pub(super) module_path:            String,
    pub(super) absolute_module:        Vec<String>,
    pub(super) prefix_start:           LineColumn,
    pub(super) leaf_start:             LineColumn,
    pub(super) full_span_start:        LineColumn,
    pub(super) full_span_end:          LineColumn,
    /// True when the target module is the file's own parent module.
    /// Rewrite the call to `super::function_name(...)` and add no `use`.
    pub(super) import_target:          ImportTarget,
    /// `#[cfg]` gates enclosing the call site. The synthesized
    /// `use module;` must repeat them or it fails to resolve in builds where
    /// the target module is configured out.
    pub(super) conditional_attributes: ConditionalAttributes,
}

pub(super) struct InlineCallDetector<'a> {
    pub(super) source_root:            &'a Path,
    pub(super) text:                   &'a str,
    pub(super) offsets:                &'a [usize],
    pub(super) current_module_path:    &'a [String],
    pub(super) declared_modules:       &'a BTreeSet<String>,
    pub(super) candidates:             Vec<InlineCallCandidate>,
    pub(super) inline_mod_depth:       usize,
    pub(super) conditional_attributes: ConditionalAttributes,
}

impl GateTracking for InlineCallDetector<'_> {
    fn gate_source(&self) -> GateSource<'_> {
        GateSource {
            text:         self.text,
            line_offsets: self.offsets,
        }
    }

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes {
        &mut self.conditional_attributes
    }
}

impl Visit<'_> for InlineCallDetector<'_> {
    imports::gate_tracking_visit!();

    fn visit_item_use(&mut self, _: &ItemUse) {}

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.inline_mod_depth += 1;
            visit::visit_item_mod(self, node);
            self.inline_mod_depth -= 1;
        } else {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_expr_path(&mut self, node: &ExprPath) {
        if self.inline_mod_depth > 0 || node.qself.is_some() {
            return;
        }
        if let Some(candidate) = analyze_inline_call(
            self.source_root,
            self.current_module_path,
            self.declared_modules,
            node,
            self.conditional_attributes.clone(),
        ) {
            self.candidates.push(candidate);
        }
    }
}

pub(super) fn build_inline_call_findings_and_fixes(
    file_context: &ScanFileContext<'_>,
    inline_inputs: &InlineCallFindingInputs<'_>,
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = file_context.display_path();

    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_modules: BTreeSet<Vec<String>> = BTreeSet::new();
    let covering_gates = covering_gates(inline_inputs.candidates);

    for candidate in inline_inputs.candidates {
        // One `use module;` per file serves every call site, so it has to
        // carry a gate that covers all of them. Call sites whose gates
        // disagree share no such import — those stay fully qualified, and
        // reporting a finding without an applicable fix would only invite the
        // rewrite that breaks the build.
        let import_attributes = match candidate.import_target {
            // The call becomes `super::function(...)`; no import is inserted,
            // so the call site's gates do not constrain anything.
            ImportTarget::ParentModule => None,
            ImportTarget::OtherModule => match covering_gates.get(&candidate.absolute_module) {
                Some(attributes) => Some(attributes),
                None => continue,
            },
        };

        let prefix_start_byte = support::offset(file_context.offsets, candidate.prefix_start);
        let leaf_start_byte = support::offset(file_context.offsets, candidate.leaf_start);
        let full_start_byte = support::offset(file_context.offsets, candidate.full_span_start);
        let full_end_byte = support::offset(file_context.offsets, candidate.full_span_end);

        let source_line = file_context
            .text
            .lines()
            .nth(candidate.full_span_start.line.saturating_sub(1))
            .unwrap_or_default()
            .to_string();

        let full_path_text = file_context
            .text
            .get(full_start_byte..full_end_byte)
            .unwrap_or_default()
            .to_string();

        let (message, suggestion) = if candidate.import_target == ImportTarget::ParentModule {
            (
                format!(
                    "use `super::{}` instead of the fully-qualified path",
                    candidate.function_name
                ),
                Some(format!(
                    "rewrite the call as `super::{}`",
                    candidate.function_name
                )),
            )
        } else {
            (
                format!(
                    "import the module `{}` instead of using the fully-qualified path for `{}`",
                    candidate.module_name, candidate.function_name
                ),
                Some(format!(
                    "add `use {};` and call `{}::{}`",
                    candidate.module_path, candidate.module_name, candidate.function_name
                )),
            )
        };

        findings.push(Finding {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::PreferModuleImport,
            path: display_path.clone(),
            line: candidate.full_span_start.line,
            column: candidate.full_span_start.column + 1,
            highlight_len: full_path_text.len().max(1),
            source_line,
            item: None,
            message,
            suggestion,
            fix_support: FixSupport::PreferModuleImport,
            related: None,
            item_visibility: ItemVisibility::default(),
        });

        let group = Some(ImportGroup {
            bare_name: candidate.module_name.clone(),
            full_path: candidate.absolute_module.join("::"),
        });

        let call_prefix = if candidate.import_target == ImportTarget::ParentModule {
            "super::".to_string()
        } else {
            format!("{}::", candidate.module_name)
        };
        fixes.push(UseFix {
            path:         file_context.path.to_path_buf(),
            start:        prefix_start_byte,
            end:          leaf_start_byte,
            replacement:  call_prefix,
            import_group: group.clone(),
        });

        let Some(import_attributes) = import_attributes else {
            continue;
        };
        if inline_inputs
            .will_import_modules
            .contains(&candidate.absolute_module)
        {
            continue;
        }
        if !inserted_modules.insert(candidate.absolute_module.clone()) {
            continue;
        }
        fixes.push(UseFix {
            path:         file_context.path.to_path_buf(),
            start:        inline_inputs.file_insertion_offset,
            end:          inline_inputs.file_insertion_offset,
            replacement:  format!(
                "{}use {};\n",
                import_attributes.render(""),
                candidate.module_path
            ),
            import_group: group,
        });
    }

    (findings, fixes)
}

/// The gate each imported module's `use` must carry, keyed by absolute module
/// path. Modules whose call sites sit under gates that disagree are absent,
/// which tells the caller to leave those calls fully qualified.
fn covering_gates(
    candidates: &[InlineCallCandidate],
) -> BTreeMap<Vec<String>, ConditionalAttributes> {
    let mut grouped: BTreeMap<Vec<String>, BTreeSet<ConditionalAttributes>> = BTreeMap::new();
    for candidate in candidates {
        if candidate.import_target == ImportTarget::ParentModule {
            continue;
        }
        grouped
            .entry(candidate.absolute_module.clone())
            .or_default()
            .insert(candidate.conditional_attributes.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(module, gates)| {
            ConditionalAttributes::covering(&gates).map(|covering| (module, covering))
        })
        .collect()
}

fn analyze_inline_call(
    source_root: &Path,
    current_module_path: &[String],
    declared_modules: &BTreeSet<String>,
    node: &ExprPath,
    conditional_attributes: ConditionalAttributes,
) -> Option<InlineCallCandidate> {
    let path = &node.path;
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if segments.len() < 3 {
        return None;
    }

    let path_anchor = PathAnchor::first(&segments)?;
    if !path_anchor.is_crate_relative() {
        return None;
    }

    let leaf = segments.last()?;
    if !support::is_snake_case_function_name(leaf) {
        return None;
    }

    let absolute_segments = support::resolve_to_absolute(&segments, current_module_path)?;
    if absolute_segments.is_empty() {
        return None;
    }
    if support::leaf_is_module(source_root, &absolute_segments) {
        return None;
    }

    let absolute_module = absolute_segments[..absolute_segments.len() - 1].to_vec();
    if absolute_module.is_empty() || !support::leaf_is_module(source_root, &absolute_module) {
        return None;
    }

    let module_name = segments[segments.len() - 2].clone();
    if PathAnchor::from(module_name.as_str()).is_crate_relative() {
        return None;
    }
    if !support::is_snake_case_module_name(&module_name) {
        return None;
    }
    if declared_modules.contains(&module_name) {
        return None;
    }

    let module_segments = &segments[..segments.len() - 1];
    let shortened = support::shorten_module_path(current_module_path, module_segments);
    let import_target = match shortened.as_slice() {
        [segment] if PathAnchor::from(segment.as_str()) == PathAnchor::Super => {
            ImportTarget::ParentModule
        },
        _ => ImportTarget::OtherModule,
    };
    let module_path = shortened.join("::");

    let first_seg = path.segments.first()?;
    let leaf_seg = path.segments.last()?;

    Some(InlineCallCandidate {
        function_name: leaf.clone(),
        module_name,
        module_path,
        absolute_module,
        prefix_start: first_seg.ident.span().start(),
        leaf_start: leaf_seg.ident.span().start(),
        full_span_start: path.span().start(),
        full_span_end: path.span().end(),
        import_target,
        conditional_attributes,
    })
}
