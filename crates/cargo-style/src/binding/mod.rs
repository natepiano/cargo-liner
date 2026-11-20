//! Read local binding scopes and align their edits across source snapshots.

mod alignment;
mod applied;
mod comment_plan;
mod constants;
mod declaration;
mod pairs;
mod patterns;
mod presence;
mod scan;

pub(crate) use self::alignment::CurrentAdmission;
pub(crate) use self::alignment::map_current_rename;
pub(crate) use self::applied::BindingOutcome;
pub(crate) use self::applied::BindingProof;
pub(crate) use self::applied::ComparisonSource;
pub(crate) use self::declaration::Declaration;
pub(crate) use self::pairs::Pair;
pub(crate) use self::pairs::read_pair;
pub(crate) use self::presence::PresenceRefusal;
pub(crate) use self::scan::BindingScan;
pub(crate) use self::scan::local_name;
