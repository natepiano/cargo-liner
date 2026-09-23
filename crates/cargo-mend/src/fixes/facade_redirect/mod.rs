//! Repointing callers that reach an item through a facade path.
//!
//! - `redirect` — `FacadeRedirect`: the facade path callers write today and the path they write
//!   instead.
//! - `caller_rewrite` — the visitor that rewrites `use` items, inline paths, and paths inside macro
//!   calls in one file.
//! - `module_aliases` — the module-level `use` items that bind a module under another path, which
//!   callers in other files write through.
//! - `module_path` — resolving a written path to a crate-relative one, and spelling a
//!   crate-relative path from a module.
//! - `use_tree` — rendering `use` trees and removing leaves from them.
//! - `source_text` — span positions to byte offsets, `use` item ranges, whole-line deletions, and
//!   the whole-word local-use check.

mod caller_rewrite;
mod module_aliases;
mod module_path;
mod redirect;
mod source_text;
mod use_tree;

pub(super) use caller_rewrite::CallerBuilds;
pub(super) use caller_rewrite::redirect_callers_in_file;
pub(super) use module_aliases::ModuleAliases;
pub(super) use module_path::absolute_use_path;
pub(super) use redirect::FacadeRedirect;
pub(super) use redirect::RedirectTarget;
pub(super) use source_text::SourceLines;
pub(super) use source_text::dedup_fixes;
pub(super) use source_text::find_source_root;
pub(super) use source_text::item_use_byte_range;
pub(super) use source_text::line_deletion;
pub(super) use source_text::word_occurs_outside;
pub(super) use use_tree::remove_use_leaves;
pub(super) use use_tree::render_use_lines;
pub(super) use use_tree::render_use_tree;
