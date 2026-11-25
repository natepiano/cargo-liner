//! Which merge extent observations a later observation supersedes, tracked as the journal replays,
//! and the compaction that rewrites the journal without them.
//!
//! An observation, in either record form, replaces the recorded merge extent of every reservation
//! it names, and no replay step reads a merge extent. So once every reservation an observation
//! names is named again by a later observation, the record leaves no trace in the fold, and
//! compacting the journal may drop it. An observation superseded for only some of its reservations
//! is kept whole. Expired finished history is removed only after checking its dependencies.

#[cfg(test)]
mod fixtures;
mod journal_compaction;
mod maintenance;
mod superseded_extents;

#[cfg(test)]
pub(crate) use fixtures::superseded_bytes;
pub(crate) use journal_compaction::JournalCompaction;
pub(crate) use journal_compaction::JournalCompactionError;
pub(super) use maintenance::remove_refusal;
pub(super) use superseded_extents::SupersededExtents;
