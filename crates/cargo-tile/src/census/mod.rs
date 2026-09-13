//! Discovery and measurement of cargo invocation groups.

pub(crate) mod command_text;
pub(crate) mod direct_capture;
pub(crate) mod invocation_cpu_accounting;
pub(crate) mod process_identity;
pub(crate) mod scan;

pub(crate) use command_text::command_name;
pub(crate) use direct_capture::DirectAssociation;
pub(crate) use direct_capture::SelectedProof;
pub(crate) use invocation_cpu_accounting::Measurement;
pub(crate) use process_identity::InvocationId;
pub(crate) use process_identity::VisibleParent;
pub(crate) use scan::Ancestor;
pub(crate) use scan::CargoGroup;
pub(crate) use scan::CargoProcess;
pub(crate) use scan::CompilerObservation;
pub(crate) use scan::RowProvenance;
pub(crate) use scan::RunStart;
pub(crate) use scan::spawn_with_resolver;
