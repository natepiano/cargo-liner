mod fix;
mod raw;
mod target;

use std::path::PathBuf;

pub(crate) use fix::FixCli;
pub(crate) use fix::FixExecution;
pub(crate) use fix::FixRequest;
pub(crate) use target::CargoCheckCli;
pub(crate) use target::TargetSelection;
pub(crate) use target::WorkspaceSelection;

use crate::reporting::OutputFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildInfoMode {
    Run,
    Show,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarningPolicy {
    Allow,
    Fail,
}

#[derive(Debug)]
pub(crate) struct Cli {
    pub build_info: BuildInfoMode,

    pub output_format: OutputFormat,

    pub warning_policy: WarningPolicy,

    pub cargo: CargoCheckCli,

    pub manifest: ManifestCli,

    pub fix: FixCli,
}

pub(crate) fn parse(after_help: &str) -> Cli { raw::parse(after_help) }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ManifestCli {
    pub config: Option<PathBuf>,
}
