//! Process discovery, attribution, and assembly of invocation groups.

mod census;
mod discovery;
mod invocation;
mod observation;
mod refresh;
mod resource_use;
mod scope;

#[cfg(target_os = "linux")]
#[path = "../scan_linux_details.rs"]
mod linux_details;

#[cfg(test)]
#[path = "tests.rs"]
pub(crate) mod tests;

pub(super) use census::Census;
#[cfg(test)]
pub(crate) use census::CensusSequence;
#[cfg(target_os = "linux")]
use discovery::LinuxDiscoveryStat;
#[cfg(target_os = "linux")]
use discovery::ProcessDiscovery;
pub(crate) use invocation::Ancestor;
pub(crate) use invocation::AncestryWalk;
pub(crate) use invocation::CargoGroup;
pub(crate) use invocation::CargoProcess;
pub(crate) use invocation::InvocationExecution;
pub(crate) use invocation::OriginStart;
pub(crate) use invocation::ProcessOwner;
pub(crate) use invocation::RowProvenance;
pub(crate) use invocation::RunStart;
#[cfg(target_os = "linux")]
use invocation::is_transparent;
pub(crate) use invocation::subtree_resource_totals;
#[cfg(any(target_os = "linux", test))]
pub(super) use observation::DetailPresence;
#[cfg(any(target_os = "linux", test))]
pub(crate) use observation::ProcessField;
pub(crate) use observation::ProcessObservation;
#[cfg(any(target_os = "linux", test))]
pub(crate) use observation::ProcessObservations;
pub(super) use observation::WorkingDirectoryObservation;
pub(crate) use refresh::FreeDiskSpace;
pub(crate) use refresh::spawn_with_resolver;
#[cfg(test)]
pub(crate) use resource_use::Compiler;
pub(crate) use resource_use::CompilerObservation;
pub(crate) use resource_use::InvocationResourceUse;
#[cfg(test)]
pub(crate) use resource_use::aggregate;
pub(crate) use resource_use::cpu_label;
pub(crate) use resource_use::sum_resource_use_once_per_pid;
pub(crate) use scope::CensusScope;
pub(crate) use scope::ExcludedCommands;

#[cfg(any(target_os = "linux", test))]
use super::command_text;
