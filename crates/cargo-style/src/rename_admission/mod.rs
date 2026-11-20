//! Prove a held local rename at its recorded coordinate and the synced tip.

mod admission;
mod outcome;
mod policy;
mod policy_syntax;
mod unprintable;

pub(crate) use self::admission::AdmissionError;
pub(crate) use self::admission::admit;
pub(crate) use self::outcome::BindingRename;
pub(crate) use self::outcome::OuterBinding;
pub(crate) use self::outcome::OwnedRename;
pub(crate) use self::outcome::RefusalCause;
pub(crate) use self::outcome::RefusalLocation;
pub(crate) use self::outcome::RenameAdmission;
pub(crate) use self::outcome::RenameRefusal;
pub(crate) use self::policy::PolicyError;
pub(crate) use self::policy::PolicyInput;
pub(crate) use self::policy::PolicyLoad;
