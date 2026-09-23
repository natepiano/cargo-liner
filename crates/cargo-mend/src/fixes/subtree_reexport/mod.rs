//! `--fix` for `pub_use_outside_subtree`: moves a re-export that reaches
//! outside its module's subtree to where callers can use it.
//!
//! - `scan` — the entry point: ties facts to fixable findings, builds every edit, and drops the
//!   facts whose edits cannot be applied.
//! - `owner_edit` — removes the re-export from its owner module, keeping a private `use` the module
//!   still needs.
//! - `local_use` — whether a module under the owner reaches the name through a glob import of the
//!   owner.
//! - `ancestor` — the re-export added to the common ancestor module.
//! - `crate_files` — the files of one crate root and the module path of each.

mod ancestor;
mod crate_files;
mod local_use;
mod owner_edit;
mod scan;

pub(super) use scan::SubtreeReexportScan;
pub(super) use scan::scan_selection;
