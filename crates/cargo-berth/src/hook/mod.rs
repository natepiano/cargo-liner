//! Engine-owned harness hook protocols.
//!
//! Every harness hook event the engine answers reads one raw JSON payload from
//! standard input, binds this process to the repository and harness session that
//! payload names, and publishes the response object the harness expects. The
//! pieces that are the same for every event live here; each verb's module holds
//! only the response fields and decisions that belong to its own event.

mod context_notice;
pub(crate) mod post_tool_use;
pub(crate) mod pre_tool_use;
mod process_binding;
pub(crate) mod session_start;
