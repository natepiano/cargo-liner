//! Classify the file and name changes in a finding.

mod classification;
mod constants;
mod file_rename;
mod qualified_rename;

pub(crate) use self::classification::FindingChange;
pub(crate) use self::classification::RenameClassifiers;
pub(crate) use self::classification::RenameIntent;
pub(crate) use self::classification::filename;
pub(crate) use self::classification::module_name;
pub(crate) use self::classification::stem;
pub(crate) use self::file_rename::FileRename;
pub(crate) use self::file_rename::FileRenamePlan;
pub(crate) use self::qualified_rename::QualifiedRename;
