use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;
use syn::spanned::Spanned;

use super::crate_files::CrateFiles;
use crate::compiler::PRELUDE_MODULE_NAME;
use crate::config::DiagnosticCode;
use crate::fixes::facade_redirect;
use crate::reporting::AncestorReexport;
use crate::reporting::Finding;
use crate::reporting::ModuleForm;
use crate::reporting::Report;
use crate::reporting::SubtreeReexportFixFact;
use crate::rust_syntax::ParsedSources;

/// Private forwarding modules whose source binding a checked fact identifies.
/// Conditional, public, inline, renamed, and glob paths retain ordinary planning.
pub(in crate::fixes) struct ForwardingChains {
    chains: Vec<Vec<ForwardingLink>>,
}

struct ForwardingLink {
    file:    PathBuf,
    fact:    SubtreeReexportFixFact,
    finding: Finding,
}

impl ForwardingChains {
    pub(in crate::fixes) fn collect(
        root: &Path,
        report: &Report,
        sources: &ParsedSources,
    ) -> Result<Self> {
        let mut chains = Vec::new();
        for seed in report.facts.subtree_reexport_fix_facts.iter() {
            if seed.owner_module_form != ModuleForm::File {
                continue;
            }
            let Some(finding) = report.findings.iter().find(|finding| {
                finding.diagnostic_code == DiagnosticCode::PubUseOutsideSubtree
                    && finding.path == seed.use_path
                    && finding.line == seed.use_line
                    && finding.column == seed.use_column
            }) else {
                continue;
            };
            let files = CrateFiles::resolve(
                &root.join(&seed.crate_root_file),
                report
                    .facts
                    .module_mount_facts
                    .iter()
                    .filter(|mount| mount.crate_root_file == seed.crate_root_file)
                    .map(|mount| (root.join(&mount.file), mount.module_path.clone())),
                sources,
            );
            if !private_path(&files, &seed.owner_module, sources)? {
                continue;
            }
            let chain = collect_links(root, &files, seed, finding, sources)?;
            if !chain.is_empty() {
                chains.push(chain);
            }
        }
        Ok(Self { chains })
    }

    pub(in crate::fixes) const fn is_empty(&self) -> bool { self.chains.is_empty() }

    /// Locate the next forwarding binding after the preceding proposed edits.
    pub(in crate::fixes) fn next_report(
        &self,
        checked: &Report,
        sources: &ParsedSources,
    ) -> Result<Report> {
        let mut report = checked.clone();
        report.findings.clear();
        let mut facts = Vec::new();
        for chain in &self.chains {
            for link in chain {
                let source = sources.source(&link.file)?;
                let syntax = source.syntax()?;
                let target = link
                    .fact
                    .source_module
                    .iter()
                    .cloned()
                    .chain([link.fact.exported_name.clone()])
                    .collect::<Vec<_>>();
                let found = syntax.items.iter().filter_map(public_use).find(|item| {
                    use_path(item)
                        .and_then(|path| {
                            facade_redirect::absolute_use_path(&link.fact.owner_module, &path)
                        })
                        .as_ref()
                        == Some(&target)
                });
                let Some(item) = found else {
                    continue;
                };
                let mut fact = link.fact.clone();
                fact.use_line = item.span().start().line;
                fact.use_column = item.span().start().column + 1;
                fact.written_path = use_path(item).unwrap_or_default().join("::");
                let mut finding = link.finding.clone();
                finding.path.clone_from(&fact.use_path);
                finding.line = fact.use_line;
                finding.column = fact.use_column;
                report.findings.push(finding);
                facts.push(fact);
                break;
            }
        }
        report.facts.subtree_reexport_fix_facts = facts.into();
        Ok(report)
    }
}

fn collect_links(
    root: &Path,
    files: &CrateFiles,
    seed: &SubtreeReexportFixFact,
    finding: &Finding,
    sources: &ParsedSources,
) -> Result<Vec<ForwardingLink>> {
    let mut links = Vec::new();
    let mut target = seed.owner_module.clone();
    target.push(seed.exported_name.clone());
    for depth in (seed.common_ancestor.len() + 1..seed.owner_module.len()).rev() {
        let owner = &seed.owner_module[..depth];
        if !owner.starts_with(&seed.target_scope) {
            break;
        }
        let Some((file, _)) = files.scannable().find(|(_, module)| *module == owner) else {
            break;
        };
        let source = sources.source(file)?;
        let syntax = source.syntax()?;
        let forwarding = syntax.items.iter().filter_map(public_use).any(|item| {
            use_path(item)
                .and_then(|path| facade_redirect::absolute_use_path(owner, &path))
                .as_ref()
                == Some(&target)
        });
        if !forwarding {
            break;
        }
        let root_key = sources.path_key(root);
        let Ok(relative) = file.strip_prefix(root_key.as_path()) else {
            break;
        };
        let mut fact = seed.clone();
        fact.owner_module = owner.to_vec();
        fact.use_path = relative.to_string_lossy().into_owned();
        // The first link already preserves the common ancestor's binding.
        fact.ancestor_reexport = AncestorReexport::NotRequired;
        links.push(ForwardingLink {
            file: file.to_path_buf(),
            fact,
            finding: finding.clone(),
        });
        target = owner
            .iter()
            .cloned()
            .chain([seed.exported_name.clone()])
            .collect();
    }
    Ok(links)
}

/// Every module declaration must be private and unconditional in its own file.
fn private_path(files: &CrateFiles, owner: &[String], sources: &ParsedSources) -> Result<bool> {
    for depth in 0..owner.len() {
        if owner[depth] == PRELUDE_MODULE_NAME {
            return Ok(false);
        }
        let Some((parent, _)) = files
            .scannable()
            .find(|(_, module)| *module == &owner[..depth])
        else {
            return Ok(false);
        };
        let source = sources.source(parent)?;
        let syntax = source.syntax()?;
        let mut declarations = syntax.items.iter().filter_map(|item| match item {
            Item::Mod(module) if module.ident == owner[depth] => Some(module),
            _ => None,
        });
        let Some(module) = declarations.next() else {
            return Ok(false);
        };
        if declarations.next().is_some()
            || !module.attrs.is_empty()
            || !matches!(module.vis, Visibility::Inherited)
            || module.content.is_some()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

const fn public_use(item: &Item) -> Option<&ItemUse> {
    match item {
        Item::Use(item)
            if matches!(item.vis, Visibility::Public(_))
                && item.attrs.is_empty()
                && item.leading_colon.is_none() =>
        {
            Some(item)
        },
        _ => None,
    }
}

fn use_path(item: &ItemUse) -> Option<Vec<String>> {
    let mut path = Vec::new();
    let mut tree = &item.tree;
    loop {
        match tree {
            UseTree::Path(segment) => {
                path.push(segment.ident.to_string());
                tree = &segment.tree;
            },
            UseTree::Name(name) => {
                path.push(name.ident.to_string());
                return Some(path);
            },
            UseTree::Group(_) | UseTree::Glob(_) | UseTree::Rename(_) => return None,
        }
    }
}
