//! Classify the file and name changes in a finding.

mod classification;
mod constants;

pub(crate) use self::classification::FileRename;
pub(crate) use self::classification::FindingChange;
pub(crate) use self::classification::RenameClassifiers;
pub(crate) use self::classification::filename;
pub(crate) use self::classification::module_name;
pub(crate) use self::classification::stem;
