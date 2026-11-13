mod constants;
mod facade_redirect;
mod field_visibility;
#[cfg(test)]
mod fixture_analysis;
mod imports;
mod imports_at_top;
mod inline_path_qualified_type;
mod narrow_pub_crate;
mod prefer_module_import;
mod pub_use_fixes;
mod restricted_annotation;
mod runner;
mod subtree_reexport;
mod unused_pub;
mod visibility_annotation_site;

pub(crate) use constants::FIX_CONVERGENCE_MAX_PASSES;
#[cfg(test)]
pub(crate) use fixture_analysis::collect_fixture_syntax_findings;
pub(crate) use pub_use_fixes::facade_use_prefix;
pub(crate) use runner::MendRunner;
#[cfg(test)]
pub(crate) use runner::apply_fixture_inline_paths;
#[cfg(test)]
pub(crate) use runner::apply_fixture_module_imports;
