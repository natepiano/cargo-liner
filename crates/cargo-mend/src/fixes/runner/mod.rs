mod apply;
mod chain;
mod combine;
mod execute;
mod function_length;
mod mend_runner;
mod notices;
mod plan;
mod session_snapshot;

use execute::CheckedSelection;
use mend_runner::FixScans;
pub(crate) use mend_runner::MendRunner;
use mend_runner::RunPlan;

#[cfg(test)]
mod fixture;
#[cfg(test)]
pub(crate) use fixture::apply_fixture_inline_paths;
#[cfg(test)]
pub(crate) use fixture::apply_fixture_module_imports;
