//! Read where each finding's declaration is at the synced tip, and the site evidence it files with.
//!
//! The ledger reads every built rename before its first finding. This reader reads them at the
//! first finding whose declaration no destination gained, so an unreadable built finding fails
//! only a reply that needs the built renames.

mod approvals;
mod branches;
mod built;
mod built_renames;
mod declaration_reader;
mod outcome;

pub(crate) use self::approvals::ApprovalFailure;
pub(crate) use self::approvals::ApprovalSnapshot;
pub(crate) use self::declaration_reader::DeclarationAnswer;
pub(crate) use self::declaration_reader::DeclarationFailure;
pub(crate) use self::declaration_reader::DeclarationInput;
pub(crate) use self::declaration_reader::DeclarationReader;
